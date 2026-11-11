//! NetLaunch's open login windows (milestones M5 and M6).
//!
//! [`Sessions`] runs a login and then looks after its window:
//!
//! - **2FA hand-over:** after the engine stops at a 2FA prompt, a watcher
//!   keeps checking the page (it never types). When the admin finishes, the
//!   status flips to "logged in" and the login history is updated. If they
//!   don't finish within [`Settings::handover_watch`], the window is simply
//!   theirs.
//! - **Timed-out logins** (a step failed, or no rule matched) are watched
//!   the same way: if the admin finishes the login by hand, the status
//!   flips to "logged in".
//! - **Managed windows** stay connected until the admin presses Close or
//!   Release, closes the window themselves, or NetLaunch exits.
//! - **Released windows** are handed over (disconnected, Edge keeps
//!   running) as soon as the login is finished, after the 2FA if any.
//! - **Signed-in warning** (decision 9): the watcher checks whether Edge
//!   signed the window in to the Windows account, and adds a warning.
//!
//! The app gets news through [`Hooks`]: view changes for the session
//! panel, ended windows, and login results for `login_log`. Nothing here
//! depends on Tauri, so the whole flow is tested with a real browser
//! (`tests/windows.rs`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use netlaunch_driver_pw::{LaunchOptions, Session};
use netlaunch_engine::{DriverError, LoginTarget, Outcome, Progress, Recipe};
use netlaunch_vault::{LoginOutcome, WindowMode};
use secrecy::SecretString;
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// What the session panel shows for one window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionView {
    pub site_id: i64,
    pub site_name: String,
    pub state: State,
    /// A sentence about the state; empty while opening.
    pub message: String,
    pub warnings: Vec<String>,
    pub window_mode: WindowMode,
}

/// Where a window is. The first four are the login running; the others
/// are an open window after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Launching,
    Loading,
    Filling,
    Waiting,
    /// The site asked for a 2FA code; the watcher waits for the admin.
    #[serde(rename = "waiting_for_2fa")]
    WaitingFor2fa,
    LoggedIn,
    /// The login timed out, or the 2FA wasn't finished in time. The window
    /// is the admin's.
    HandedOver,
    /// The site rejected the login, or a step failed. The window stays so
    /// the admin can see why.
    Failed,
    /// The page was on another origin; nothing was typed.
    Stopped,
}

impl State {
    /// True while the login itself runs: Close, Release and Retry wait.
    pub fn is_busy(self) -> bool {
        matches!(self, State::Launching | State::Loading | State::Filling | State::Waiting)
    }
}

/// Why a window left the session list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ended {
    /// NetLaunch closed it (the Close button, Retry, or the app exiting).
    Closed,
    /// Handed over: Edge keeps running without NetLaunch.
    Released,
    /// The admin closed the Edge window.
    WindowClosed,
    /// The login ended without a usable window (Edge didn't start, or the
    /// page didn't load).
    NoWindow,
}

/// How the app hears about sessions. Called from background tasks, so
/// implementations must be quick and must not block.
pub trait Hooks: Send + Sync {
    /// A window's view changed (or a new one appeared).
    fn changed(&self, view: &SessionView);
    /// A window left the list.
    fn ended(&self, site_id: i64, why: Ended);
    /// How a login went, for `login_log`. Called once when the login
    /// finishes, and again if a 2FA hand-over later succeeds or runs out.
    /// `message` never contains a password.
    fn record(&self, log_id: i64, outcome: LoginOutcome, message: &str);
}

/// Timings. The defaults are for the app; tests shorten them.
#[derive(Debug, Clone)]
pub struct Settings {
    /// How long to wait for the admin to finish a 2FA prompt.
    pub handover_watch: Duration,
    /// How often the watcher looks at each window.
    pub tick: Duration,
    /// How long after launch to keep checking for Edge's implicit sign-in.
    /// Edge writes it to the profile a few seconds after it happens.
    pub sign_in_check: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            handover_watch: Duration::from_secs(10 * 60),
            tick: Duration::from_secs(1),
            sign_in_check: Duration::from_secs(120),
        }
    }
}

/// Everything needed to open one site.
pub struct OpenRequest {
    pub site_id: i64,
    pub site_name: String,
    pub login_url: String,
    pub username: String,
    pub recipe: Recipe,
    pub window_mode: WindowMode,
    pub launch: LaunchOptions,
    /// The `login_log` row for this attempt, passed back to
    /// [`Hooks::record`].
    pub log_id: i64,
}

/// Reads the password from the vault. Called once, when the engine fills
/// the password field.
pub type PasswordSource = Box<dyn FnMut() -> Result<SecretString, String> + Send>;

/// What the Open button gets back once the login has finished.
#[derive(Debug, Clone, Serialize)]
pub struct OpenResult {
    pub outcome: LoginOutcome,
    /// One sentence for the admin; also what's recorded in `login_log`.
    pub message: String,
    pub warnings: Vec<String>,
    /// A one-click fix the UI can offer: `"trust_certificate"` when Edge
    /// rejected the site's certificate.
    pub fix: Option<&'static str>,
}

pub const SIGNED_IN_WARNING: &str = "Edge signed this window in to your Windows account (its implicit sign-in). \
     Sync and extensions are off, but Microsoft websites opened in this window will know who you are.";

/// The open windows, one per site at most. Cheap to clone: clones share
/// the same list.
#[derive(Clone)]
pub struct Sessions {
    inner: Arc<Inner>,
}

struct Inner {
    entries: Mutex<HashMap<i64, Entry>>,
    hooks: Arc<dyn Hooks>,
    settings: Settings,
}

struct Entry {
    view: SessionView,
    /// `None` while the login runs (the `open` call holds the session).
    session: Option<Arc<Session>>,
    /// The watcher task, aborted before a Close or Release.
    watcher: Option<JoinHandle<()>>,
}

impl Sessions {
    pub fn new(hooks: Arc<dyn Hooks>, settings: Settings) -> Self {
        Sessions { inner: Arc::new(Inner { entries: Mutex::new(HashMap::new()), hooks, settings }) }
    }

    /// The session panel's rows.
    pub fn list(&self) -> Vec<SessionView> {
        let entries = self.lock();
        let mut views: Vec<_> = entries.values().map(|e| e.view.clone()).collect();
        views.sort_by(|a, b| a.site_name.cmp(&b.site_name));
        views
    }

    /// The browser tab of an open window. For tests (playing the admin,
    /// who types the 2FA code) and diagnostics. Drop it soon: Close and
    /// Release wait until nobody else holds it.
    pub fn driver(&self, site_id: i64) -> Option<Arc<Session>> {
        self.lock().get(&site_id).and_then(|e| e.session.clone())
    }

    /// Runs a login and keeps or hands over its window. Returns once the
    /// login has finished (the 2FA, if any, is then watched in the
    /// background). Errors only when the site can't be opened at all
    /// (it's already open).
    pub async fn open(&self, request: OpenRequest, mut password: PasswordSource) -> Result<OpenResult, String> {
        let OpenRequest { site_id, site_name, login_url, username, recipe, window_mode, launch, log_id } = request;
        let hooks = self.inner.hooks.clone();

        let mut view = SessionView {
            site_id,
            site_name,
            state: State::Launching,
            message: String::new(),
            warnings: Vec::new(),
            window_mode,
        };
        {
            let mut entries = self.lock();
            if let Some(existing) = entries.get(&site_id) {
                return Err(if existing.view.state.is_busy() {
                    "This site is already opening.".into()
                } else {
                    "This site's window is already open. Use Close or Retry in Open windows.".into()
                });
            }
            entries.insert(site_id, Entry { view: view.clone(), session: None, watcher: None });
        }
        hooks.changed(&view);
        // Until the window is stored, any early return removes the entry.
        let mut claim = Claim { sessions: self, site_id, ended: Ended::NoWindow, armed: true };

        let session = match Session::launch(launch).await {
            Ok(session) => session,
            Err(message) => {
                hooks.record(log_id, LoginOutcome::Failed, &message);
                return Ok(OpenResult { outcome: LoginOutcome::Failed, message, warnings: Vec::new(), fix: None });
            }
        };
        view.warnings = session.isolation_warnings().await;
        for warning in &view.warnings {
            tracing::warn!("{warning}");
        }

        let target = LoginTarget { login_url: &login_url, username: &username };
        let outcome = netlaunch_engine::run(&session, &recipe, &target, &mut *password, &mut |stage| {
            view.state = match stage {
                Progress::Loading => State::Loading,
                Progress::Filling => State::Filling,
                Progress::Waiting => State::Waiting,
            };
            self.set_view(&view);
        })
        .await;
        // The password source isn't needed again.
        drop(password);

        let message = outcome.message();
        hooks.record(log_id, outcome.log_outcome(), &message);
        let untrusted = matches!(outcome, Outcome::UntrustedCertificate { .. });
        let result = OpenResult {
            outcome: outcome.log_outcome(),
            message: message.clone(),
            warnings: view.warnings.clone(),
            fix: untrusted.then_some("trust_certificate"),
        };

        // No usable window: closed by the admin, or only Edge's certificate
        // warning (trusting the certificate needs a new Edge process).
        if !session.is_running() || untrusted {
            session.close().await;
            return Ok(result);
        }

        view.message = message;
        view.state = match outcome {
            Outcome::LoggedIn => State::LoggedIn,
            Outcome::MfaHandoff => State::WaitingFor2fa,
            Outcome::TimedOut { .. } => State::HandedOver,
            Outcome::OriginMismatch { .. } => State::Stopped,
            Outcome::Rejected { .. } | Outcome::Error(_) | Outcome::UntrustedCertificate { .. } => State::Failed,
        };
        let watch_2fa = view.state == State::WaitingFor2fa;
        // A timed-out login is watched too: when the admin finishes it by
        // hand, the status flips to logged in.
        let watch_by_hand = view.state == State::HandedOver;

        // A released window is handed over as soon as the login is
        // finished. With a 2FA prompt it isn't yet: watch it first.
        if window_mode == WindowMode::Released && !watch_2fa {
            session.release().await;
            claim.ended = Ended::Released;
            return Ok(result);
        }

        // Keep the window and start its watcher.
        claim.armed = false;
        let session = Arc::new(session);
        {
            let mut entries = self.lock();
            if let Some(entry) = entries.get_mut(&site_id) {
                entry.view = view.clone();
                entry.session = Some(session.clone());
            }
        }
        hooks.changed(&view);
        let watch = Watch {
            recipe,
            login_url,
            log_id,
            handover_until: (watch_2fa || watch_by_hand)
                .then(|| Instant::now() + self.inner.settings.handover_watch),
            after_2fa: watch_2fa,
        };
        let watcher = tokio::spawn(watch_window(self.clone(), site_id, watch));
        if let Some(entry) = self.lock().get_mut(&site_id) {
            entry.watcher = Some(watcher);
        }
        Ok(result)
    }

    /// The Release button: hand the window to the admin.
    pub async fn release(&self, site_id: i64) -> Result<(), String> {
        let session = self.take(site_id).await?;
        session.release().await;
        self.inner.hooks.ended(site_id, Ended::Released);
        Ok(())
    }

    /// The Close button (and Retry, before it opens again): close the
    /// window, deleting a fresh profile.
    pub async fn close(&self, site_id: i64) -> Result<(), String> {
        let session = self.take(site_id).await?;
        session.close().await;
        self.inner.hooks.ended(site_id, Ended::Closed);
        Ok(())
    }

    /// Closes every managed window, for when NetLaunch exits. Logins still
    /// running are left to finish on their own.
    pub async fn close_all(&self) {
        let ids: Vec<i64> = self.lock().iter().filter(|(_, e)| e.session.is_some()).map(|(id, _)| *id).collect();
        for id in ids {
            let _ = self.close(id).await;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Entry>> {
        // A panic while holding the lock can't leave the map half-changed
        // in a way that matters here, so keep going with its contents.
        self.inner.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Stores `view` and tells the app.
    fn set_view(&self, view: &SessionView) {
        if let Some(entry) = self.lock().get_mut(&view.site_id) {
            entry.view = view.clone();
        }
        self.inner.hooks.changed(view);
    }

    /// Changes a stored view and tells the app. Returns the new view.
    fn update(&self, site_id: i64, change: impl FnOnce(&mut SessionView)) -> Option<SessionView> {
        let view = {
            let mut entries = self.lock();
            let entry = entries.get_mut(&site_id)?;
            change(&mut entry.view);
            entry.view.clone()
        };
        self.inner.hooks.changed(&view);
        Some(view)
    }

    /// Removes an open window from the list and returns sole ownership of
    /// its session, stopping its watcher first.
    async fn take(&self, site_id: i64) -> Result<Session, String> {
        let entry = {
            let mut entries = self.lock();
            match entries.get(&site_id) {
                None => return Err("This site has no open window.".into()),
                Some(e) if e.session.is_none() => return Err("This site is still opening; wait for it to finish.".into()),
                Some(_) => entries.remove(&site_id).expect("checked above"),
            }
        };
        if let Some(watcher) = entry.watcher {
            watcher.abort();
            // Wait for it to stop, so its copy of the session is dropped.
            let _ = watcher.await;
        }
        unwrap_session(entry.session.expect("checked above")).await
    }

    /// Takes a window out of the list from inside its own watcher, which
    /// must not abort itself.
    async fn take_from_watcher(&self, site_id: i64) -> Option<Session> {
        let entry = self.lock().remove(&site_id)?;
        // Dropping the handle detaches it; the watcher returns right after.
        drop(entry.watcher);
        unwrap_session(entry.session?).await.ok()
    }
}

/// `Arc::try_unwrap`, retried for a moment: a test or a diagnostic may
/// hold a copy briefly (see [`Sessions::driver`]).
async fn unwrap_session(mut session: Arc<Session>) -> Result<Session, String> {
    for _ in 0..50 {
        match Arc::try_unwrap(session) {
            Ok(owned) => return Ok(owned),
            Err(shared) => session = shared,
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("The window is busy; try again.".into())
}

/// While `open` runs, removes the site's entry if `open` returns before
/// storing the window.
struct Claim<'a> {
    sessions: &'a Sessions,
    site_id: i64,
    ended: Ended,
    armed: bool,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.sessions.lock().remove(&self.site_id);
            self.sessions.inner.hooks.ended(self.site_id, self.ended);
        }
    }
}

/// What a watcher needs to know about its window.
struct Watch {
    recipe: Recipe,
    login_url: String,
    log_id: i64,
    /// While set, the window is waiting for the admin's 2FA, until then.
    handover_until: Option<Instant>,
    /// What's being waited for: a 2FA prompt (true), or the admin
    /// finishing a timed-out login by hand (false).
    after_2fa: bool,
}

/// One per kept window. Every tick it checks the window still exists,
/// whether a 2FA hand-over has finished, and (for the first couple of
/// minutes) whether Edge signed the window in to the Windows account.
async fn watch_window(sessions: Sessions, site_id: i64, mut watch: Watch) {
    let settings = sessions.inner.settings.clone();
    let hooks = sessions.inner.hooks.clone();
    let started = Instant::now();
    let mut warned_sign_in = false;

    loop {
        tokio::time::sleep(settings.tick).await;
        let Some(session) = sessions.driver(site_id) else { return };

        // The admin closed the Edge window.
        if !session.is_running() {
            drop(session);
            if let Some(dead) = sessions.take_from_watcher(site_id).await {
                // Stops the driver and deletes a fresh profile.
                dead.close().await;
            }
            hooks.ended(site_id, Ended::WindowClosed);
            return;
        }

        if !warned_sign_in && started.elapsed() <= settings.sign_in_check && session.signed_in() {
            warned_sign_in = true;
            tracing::warn!("the login window is signed in to an account (implicit sign-in)");
            sessions.update(site_id, |view| view.warnings.push(SIGNED_IN_WARNING.into()));
        }

        let Some(until) = watch.handover_until else { continue };
        let finished = match netlaunch_engine::logged_in(&*session, &watch.recipe, &watch.login_url).await {
            Ok(true) => {
                let message =
                    if watch.after_2fa { "Logged in after 2FA." } else { "Logged in: you finished it by hand." };
                hooks.record(watch.log_id, LoginOutcome::Success, message);
                sessions.update(site_id, |view| {
                    view.state = State::LoggedIn;
                    view.message = message.into();
                });
                true
            }
            // Gone: the next tick notices it isn't running.
            Err(DriverError::Closed(_)) => false,
            // A hand-finished login that never came: the row already
            // says the window is theirs, so just stop watching.
            _ if Instant::now() >= until && !watch.after_2fa => true,
            _ if Instant::now() >= until => {
                let minutes = settings.handover_watch.as_secs().div_ceil(60);
                let message = format!("The 2FA wasn't finished within {minutes} min. The window is yours.");
                hooks.record(watch.log_id, LoginOutcome::Timeout, &message);
                sessions.update(site_id, |view| {
                    view.state = State::HandedOver;
                    view.message = message.clone();
                });
                true
            }
            _ => false,
        };
        if !finished {
            continue;
        }
        watch.handover_until = None;

        // A released window waited only for the 2FA; hand it over now.
        let released = sessions.lock().get(&site_id).is_some_and(|e| e.view.window_mode == WindowMode::Released);
        if released {
            drop(session);
            if let Some(owned) = sessions.take_from_watcher(site_id).await {
                owned.release().await;
            }
            hooks.ended(site_id, Ended::Released);
            return;
        }
    }
}
