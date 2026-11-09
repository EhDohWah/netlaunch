//! Opening a site: what the Open button does (milestone M5).
//!
//! 1. Look up the site and its recipe, and record the attempt in
//!    `login_log` (outcome `running`).
//! 2. Launch Edge with the site's own profile folder and attach.
//! 3. Run the recipe. The engine asks for the password only when it fills
//!    the password field; that's the one place `Vault::reveal_password` is
//!    called.
//! 4. Record how it went, and keep the window (managed) or hand it over
//!    (released).
//!
//! Progress goes to the UI as `login-progress` events; the command's
//! return value is the final result. Neither ever carries a password.

use std::collections::HashMap;
use std::sync::Mutex;

use netlaunch_driver_pw::{LaunchOptions, Profile, Session};
use netlaunch_engine::{LoginTarget, Outcome, Progress};
use netlaunch_vault::{LoginOutcome, ProfileMode, WindowMode};
use serde::Serialize;

use crate::AppState;

/// The browser windows NetLaunch has open, one per site at most.
#[derive(Default)]
pub struct Sessions {
    slots: Mutex<HashMap<i64, Slot>>,
}

enum Slot {
    /// A login is running for this site.
    Opening,
    /// A managed window, logged in (or handed over). Closed when NetLaunch
    /// exits. Boxed because a `Session` is large and `Opening` is empty.
    Open(Box<Session>),
}

impl Sessions {
    /// Claims the site for a new login. A window from an earlier login
    /// that the admin has since closed is cleaned up first.
    async fn reserve(&self, site_id: i64) -> Result<Reservation<'_>, String> {
        let leftover = {
            let mut slots = self.slots.lock().map_err(|_| "session list unavailable".to_string())?;
            match slots.get(&site_id) {
                Some(Slot::Opening) => return Err("This site is already opening.".into()),
                Some(Slot::Open(session)) if session.is_running() => {
                    return Err("This site's window is already open.".into());
                }
                _ => {}
            }
            let leftover = slots.remove(&site_id);
            slots.insert(site_id, Slot::Opening);
            leftover
        };
        if let Some(Slot::Open(dead)) = leftover {
            // Edge has exited; this stops the driver and deletes a fresh
            // profile folder.
            dead.close().await;
        }
        Ok(Reservation { sessions: self, site_id, done: false })
    }

    /// Takes every open window, to close them when the app exits.
    pub fn take_all(&self) -> Vec<Session> {
        let Ok(mut slots) = self.slots.lock() else { return Vec::new() };
        slots
            .drain()
            .filter_map(|(_, slot)| match slot {
                Slot::Open(session) => Some(*session),
                Slot::Opening => None,
            })
            .collect()
    }
}

/// The claim on a site while its login runs. If the login stops early
/// (an error, a `?`), dropping it frees the site again.
struct Reservation<'a> {
    sessions: &'a Sessions,
    site_id: i64,
    done: bool,
}

impl Reservation<'_> {
    fn keep(mut self, session: Session) {
        if let Ok(mut slots) = self.sessions.slots.lock() {
            slots.insert(self.site_id, Slot::Open(Box::new(session)));
        }
        self.done = true;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.done
            && let Ok(mut slots) = self.sessions.slots.lock()
        {
            slots.remove(&self.site_id);
        }
    }
}

/// Sent to the UI as the `login-progress` event.
#[derive(Clone, Serialize)]
pub struct ProgressEvent {
    pub site_id: i64,
    /// `launching`, then the engine's stages: `loading`, `filling`,
    /// `waiting`.
    pub stage: &'static str,
}

/// What the Open button gets back.
#[derive(Serialize)]
pub struct OpenResult {
    pub outcome: LoginOutcome,
    /// One sentence for the admin; also saved in `login_log.detail`.
    pub message: String,
    /// Isolation problems with the window (Edge policies can override
    /// NetLaunch's switches). Empty when all is well.
    pub warnings: Vec<String>,
    /// A one-click fix the UI can offer. Only `"trust_certificate"` today:
    /// the site's certificate wasn't trusted, so offer to tick its
    /// self-signed setting and open it again.
    pub fix: Option<&'static str>,
}

/// Runs a login for `site_id`. `progress` is called as it moves along.
pub async fn open_site(
    state: &AppState,
    site_id: i64,
    progress: impl Fn(ProgressEvent) + Send + Sync,
) -> Result<OpenResult, String> {
    let site = state.with_vault(|vault| vault.get_site(site_id))?;
    let recipe = netlaunch_engine::builtin(&site.recipe_id)
        .ok_or_else(|| format!("This site uses an unknown login recipe '{}'. Edit it and pick one.", site.recipe_id))?;

    let reservation = state.sessions.reserve(site_id).await?;
    let log_id = state.with_vault(|vault| vault.start_login(site_id))?;
    // From here on every path records how it ended.
    let finish = |outcome: LoginOutcome, message: String, warnings: Vec<String>| {
        if let Err(e) = state.with_vault(|vault| vault.finish_login(log_id, outcome, Some(&message))) {
            tracing::warn!("couldn't record the login result: {e}");
        }
        OpenResult { outcome, message, warnings, fix: None }
    };

    progress(ProgressEvent { site_id, stage: "launching" });
    let Some(browser) = netlaunch_driver_pw::find_edge() else {
        return Ok(finish(
            LoginOutcome::Failed,
            "Microsoft Edge wasn't found on this PC. NetLaunch opens sites in Edge.".into(),
            Vec::new(),
        ));
    };
    let profile = match site.profile_mode {
        ProfileMode::Fresh => Profile::Fresh,
        ProfileMode::Persistent => match netlaunch_driver_pw::persistent_profile_dir(site_id) {
            Some(dir) => Profile::Persistent(dir),
            None => {
                return Ok(finish(
                    LoginOutcome::Failed,
                    "LOCALAPPDATA isn't set, so there's nowhere to keep this site's profile.".into(),
                    Vec::new(),
                ));
            }
        },
    };
    let options = LaunchOptions {
        browser,
        profile,
        accept_self_signed: site.allow_self_signed,
        headless: false,
        extra_args: Vec::new(),
    };
    let session = match Session::launch(options).await {
        Ok(session) => session,
        Err(message) => return Ok(finish(LoginOutcome::Failed, message, Vec::new())),
    };
    let warnings = session.isolation_warnings().await;
    for warning in &warnings {
        tracing::warn!("{warning}");
    }

    let target = LoginTarget { login_url: &site.login_url, username: &site.username };
    let outcome = netlaunch_engine::run(
        &session,
        &recipe,
        &target,
        // The only call to reveal_password in the app, made at fill time.
        &mut || state.with_vault(|vault| vault.reveal_password(site_id)),
        &mut |stage| {
            let stage = match stage {
                Progress::Loading => "loading",
                Progress::Filling => "filling",
                Progress::Waiting => "waiting",
            };
            progress(ProgressEvent { site_id, stage });
        },
    )
    .await;
    let mut result = finish(outcome.log_outcome(), outcome.message(), warnings);
    let untrusted = matches!(outcome, Outcome::UntrustedCertificate { .. });
    if untrusted {
        result.fix = Some("trust_certificate");
    }

    // What happens to the window now.
    if !session.is_running() || untrusted {
        // Closed by the admin, or showing nothing but Edge's certificate
        // warning. Close it: trusting the certificate needs a new Edge
        // process anyway, because the setting is a launch switch.
        session.close().await;
    } else if site.window_mode == WindowMode::Released {
        session.release().await;
    } else {
        reservation.keep(session);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_site_can_only_be_opening_once() {
        let sessions = Sessions::default();
        let first = sessions.reserve(7).await.unwrap();
        assert!(sessions.reserve(7).await.is_err());
        assert!(sessions.reserve(8).await.is_ok(), "other sites are independent");
        drop(first);
        assert!(sessions.reserve(7).await.is_ok(), "dropping the claim frees the site");
    }

    #[test]
    fn results_never_carry_a_password_field() {
        let result =
            OpenResult { outcome: LoginOutcome::Success, message: "Logged in.".into(), warnings: vec![], fix: None };
        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(json, r#"{"outcome":"success","message":"Logged in.","warnings":[],"fix":null}"#);
    }
}
