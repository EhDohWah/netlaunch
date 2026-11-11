//! M6 "done when": `/mfa` and `/no-success` behave as expected in managed
//! and released mode, against the mock site in a real browser. Plus the
//! window being closed by hand, an unfinished 2FA, Close, and opening a
//! site twice.
//!
//! Browser: `NETLAUNCH_TEST_BROWSER`, else the installed Edge, else Chrome
//! or Chromium in the usual Linux places. With none the tests skip, unless
//! `NETLAUNCH_REQUIRE_BROWSER=1` (CI).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use netlaunch_driver_pw::{LaunchOptions, Profile};
use netlaunch_engine::{BrowserDriver, builtin};
use netlaunch_mock_site::{MFA_CODE, MockConfig, PASSWORD, RunningMock, USERNAME};
use netlaunch_sessions::{
    Ended, Hooks, OpenRequest, OpenResult, SIGNED_IN_WARNING, SessionView, Sessions, Settings, State,
};
use netlaunch_vault::{LoginOutcome, WindowMode};
use secrecy::SecretString;
use tokio::sync::Semaphore;

fn browser() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NETLAUNCH_TEST_BROWSER") {
        return Some(PathBuf::from(path));
    }
    if let Some(edge) = netlaunch_driver_pw::find_edge() {
        return Some(edge);
    }
    ["/usr/bin/microsoft-edge", "/usr/bin/google-chrome", "/usr/bin/chromium", "/opt/pw-browsers/chromium"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

static BROWSERS: Semaphore = Semaphore::const_new(2);

/// The browser to use and a turn to use it, or `None` to skip.
async fn start(test: &str) -> Option<(PathBuf, tokio::sync::SemaphorePermit<'static>)> {
    let Some(found) = browser() else {
        if std::env::var("NETLAUNCH_REQUIRE_BROWSER").as_deref() == Ok("1") {
            panic!("{test}: no browser found and NETLAUNCH_REQUIRE_BROWSER=1");
        }
        eprintln!("{test}: SKIPPED, no Edge or Chrome found (set NETLAUNCH_TEST_BROWSER)");
        return None;
    };
    Some((found, BROWSERS.acquire().await.unwrap()))
}

/// Everything the app would be told, in order.
#[derive(Default)]
struct Recorder {
    views: Mutex<Vec<SessionView>>,
    ended: Mutex<Vec<(i64, Ended)>>,
    records: Mutex<Vec<(i64, LoginOutcome, String)>>,
}

impl Hooks for Recorder {
    fn changed(&self, view: &SessionView) {
        self.views.lock().unwrap().push(view.clone());
    }
    fn ended(&self, site_id: i64, why: Ended) {
        self.ended.lock().unwrap().push((site_id, why));
    }
    fn record(&self, log_id: i64, outcome: LoginOutcome, message: &str) {
        self.records.lock().unwrap().push((log_id, outcome, message.to_string()));
    }
}

impl Recorder {
    fn outcomes(&self) -> Vec<LoginOutcome> {
        self.records.lock().unwrap().iter().map(|(_, o, _)| *o).collect()
    }
    fn ended(&self) -> Vec<(i64, Ended)> {
        self.ended.lock().unwrap().clone()
    }
}

struct Fixture {
    sessions: Sessions,
    recorder: Arc<Recorder>,
    mock: RunningMock,
    browser: PathBuf,
}

const SITE: i64 = 1;
const LOG: i64 = 77;

async fn fixture(browser: PathBuf, handover_watch: Duration) -> Fixture {
    let recorder = Arc::new(Recorder::default());
    let settings = Settings {
        handover_watch,
        tick: Duration::from_millis(200),
        sign_in_check: Duration::from_secs(120),
    };
    let sessions = Sessions::new(recorder.clone(), settings);
    let mock = netlaunch_mock_site::serve(MockConfig { port: 0, lookalike_port: 0, slow_delay_ms: 0 }).await.unwrap();
    Fixture { sessions, recorder, mock, browser }
}

impl Fixture {
    async fn open(&self, scenario: &str, mode: WindowMode, timeout_ms: u64) -> Result<OpenResult, String> {
        self.open_with(scenario, mode, timeout_ms, Profile::Fresh).await
    }

    async fn open_with(
        &self,
        scenario: &str,
        mode: WindowMode,
        timeout_ms: u64,
        profile: Profile,
    ) -> Result<OpenResult, String> {
        let mut recipe = builtin("generic-form").unwrap();
        recipe.timeout_ms = timeout_ms;
        self.open_recipe(scenario, mode, recipe, profile).await
    }

    async fn open_recipe(
        &self,
        scenario: &str,
        mode: WindowMode,
        recipe: netlaunch_engine::Recipe,
        profile: Profile,
    ) -> Result<OpenResult, String> {
        let request = OpenRequest {
            site_id: SITE,
            site_name: format!("Mock {scenario}"),
            login_url: format!("{}/{scenario}/login", self.mock.base_url),
            username: USERNAME.into(),
            recipe,
            window_mode: mode,
            launch: LaunchOptions {
                browser: self.browser.clone(),
                profile,
                accept_self_signed: true,
                headless: true,
                extra_args: if cfg!(target_os = "linux") { vec!["--no-sandbox".into()] } else { vec![] },
            },
            log_id: LOG,
        };
        self.sessions.open(request, Box::new(|| Ok(SecretString::from(PASSWORD.to_string())))).await
    }

    fn state(&self) -> Option<State> {
        self.sessions.list().iter().find(|v| v.site_id == SITE).map(|v| v.state)
    }

    /// Waits up to 15 s for `check` to hold.
    async fn wait_for(&self, what: &str, check: impl Fn(&Fixture) -> bool) {
        for _ in 0..150 {
            if check(self) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!(
            "timed out waiting for {what}; state {:?}, ended {:?}, records {:?}",
            self.state(),
            self.recorder.ended(),
            self.recorder.outcomes()
        );
    }

    /// The admin's part: type the 2FA code into the window and send it.
    /// The engine never does this.
    async fn type_the_code(&self) {
        let tab = self.sessions.driver(SITE).expect("window kept for the 2FA");
        tab.fill("input[autocomplete=one-time-code]", &SecretString::from(MFA_CODE.to_string())).await.unwrap();
        tab.click("button[type=submit]").await.unwrap();
    }

    /// The Edge process id and fresh profile folder of the open window,
    /// for checking (and cleaning up) a released window.
    fn process(&self) -> (u32, PathBuf) {
        let tab = self.sessions.driver(SITE).expect("window open");
        (tab.pid().unwrap(), tab.profile_dir().to_path_buf())
    }
}

fn process_alive(pid: u32) -> bool {
    if cfg!(windows) {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .expect("tasklist");
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    } else {
        // A finished child we never waited for is a zombie: gone for our
        // purposes.
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|stat| !stat.split_whitespace().nth(2).is_some_and(|s| s == "Z"))
            .unwrap_or(false)
    }
}

/// Kills an Edge NetLaunch released (and its helper processes), as the
/// admin closing it would.
fn kill(pid: u32) {
    if cfg!(windows) {
        let _ = std::process::Command::new("taskkill").args(["/F", "/T", "/PID", &pid.to_string()]).output();
    } else {
        // Edge was started in its own process group, so this ends the
        // whole browser.
        let _ = std::process::Command::new("kill").args(["-9", "--", &format!("-{pid}")]).output();
    }
}

async fn remove_dir(dir: PathBuf) {
    for _ in 0..40 {
        if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mfa_managed_waits_then_flips_to_logged_in() {
    let Some((browser, _turn)) = start("mfa_managed_waits_then_flips_to_logged_in").await else { return };
    let f = fixture(browser, Duration::from_secs(60)).await;

    let result = f.open("mfa", WindowMode::Managed, 15_000).await.unwrap();
    assert_eq!(result.outcome, LoginOutcome::MfaHandoff);
    assert_eq!(f.state(), Some(State::WaitingFor2fa));

    // Nothing changes until the admin acts.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(f.state(), Some(State::WaitingFor2fa));

    f.type_the_code().await;
    f.wait_for("logged in", |f| f.state() == Some(State::LoggedIn)).await;
    assert_eq!(f.recorder.outcomes(), vec![LoginOutcome::MfaHandoff, LoginOutcome::Success]);
    assert!(f.recorder.records.lock().unwrap().iter().all(|(log, _, _)| *log == LOG));

    // Managed: the window stays until it's closed.
    assert!(f.recorder.ended().is_empty());
    f.sessions.close(SITE).await.unwrap();
    assert_eq!(f.recorder.ended(), vec![(SITE, Ended::Closed)]);
    assert!(f.sessions.list().is_empty());
    f.mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn mfa_released_waits_then_hands_the_window_over() {
    let Some((browser, _turn)) = start("mfa_released_waits_then_hands_the_window_over").await else { return };
    let f = fixture(browser, Duration::from_secs(60)).await;

    let result = f.open("mfa", WindowMode::Released, 15_000).await.unwrap();
    assert_eq!(result.outcome, LoginOutcome::MfaHandoff);
    // Released, but not yet: the 2FA is watched first.
    assert_eq!(f.state(), Some(State::WaitingFor2fa));
    let (pid, profile) = f.process();

    f.type_the_code().await;
    f.wait_for("release", |f| f.recorder.ended() == vec![(SITE, Ended::Released)]).await;
    assert_eq!(f.recorder.outcomes(), vec![LoginOutcome::MfaHandoff, LoginOutcome::Success]);
    assert!(f.sessions.list().is_empty());

    // Edge keeps running on its own.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(process_alive(pid), "a released window survives");
    kill(pid);
    remove_dir(profile).await;
    f.mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn no_success_managed_is_handed_over_and_kept() {
    let Some((browser, _turn)) = start("no_success_managed_is_handed_over_and_kept").await else { return };
    let f = fixture(browser, Duration::from_secs(60)).await;

    let result = f.open("no-success", WindowMode::Managed, 2_000).await.unwrap();
    assert_eq!(result.outcome, LoginOutcome::Timeout);
    assert_eq!(f.state(), Some(State::HandedOver));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(f.state(), Some(State::HandedOver), "a timeout isn't watched further");
    assert!(f.recorder.ended().is_empty());

    f.sessions.close(SITE).await.unwrap();
    assert_eq!(f.recorder.ended(), vec![(SITE, Ended::Closed)]);
    f.mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn no_success_released_is_handed_over_at_once() {
    let Some((browser, _turn)) = start("no_success_released_is_handed_over_at_once").await else { return };
    let f = fixture(browser, Duration::from_secs(60)).await;

    // The window is released straight after the login, so the test never
    // sees its process id. A profile folder with a name only this test
    // uses finds the process afterwards (and tries a persistent profile).
    let profile = std::env::temp_dir().join(format!("netlaunch-test-released-{}", std::process::id()));
    let result = f.open_with("no-success", WindowMode::Released, 2_000, Profile::Persistent(profile.clone())).await;
    let result = result.unwrap();
    assert_eq!(result.outcome, LoginOutcome::Timeout);
    assert_eq!(f.recorder.ended(), vec![(SITE, Ended::Released)]);
    assert!(f.sessions.list().is_empty());
    assert_eq!(f.recorder.outcomes(), vec![LoginOutcome::Timeout]);

    if cfg!(target_os = "linux") {
        let found = std::process::Command::new("pgrep").args(["-f", &profile.display().to_string()]).output().unwrap();
        assert!(!found.stdout.is_empty(), "the released browser is still running");
        let _ = std::process::Command::new("pkill").args(["-9", "-f", &profile.display().to_string()]).output();
    }
    // On Windows the released headless Edge is left to the CI runner,
    // which is thrown away after the job.
    remove_dir(profile).await;
    f.mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn finishing_a_timed_out_login_by_hand_is_noticed() {
    let Some((browser, _turn)) = start("finishing_a_timed_out_login_by_hand_is_noticed").await else { return };
    let f = fixture(browser, Duration::from_secs(60)).await;

    // /mfa with a recipe that knows no 2FA rule: the login "times out" on
    // the code page, as a real site's would when a step fails.
    let mut recipe = builtin("generic-form").unwrap();
    recipe.timeout_ms = 2_000;
    recipe.mfa.any.clear();
    let result = f.open_recipe("mfa", WindowMode::Managed, recipe, Profile::Fresh).await.unwrap();
    assert_eq!(result.outcome, LoginOutcome::Timeout);
    assert_eq!(f.state(), Some(State::HandedOver));

    // The admin finishes by hand.
    f.type_the_code().await;
    f.wait_for("logged in", |f| f.state() == Some(State::LoggedIn)).await;
    assert_eq!(f.recorder.outcomes(), vec![LoginOutcome::Timeout, LoginOutcome::Success]);

    f.sessions.close(SITE).await.unwrap();
    f.mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn unfinished_2fa_is_handed_over_after_the_watch() {
    let Some((browser, _turn)) = start("unfinished_2fa_is_handed_over_after_the_watch").await else { return };
    let f = fixture(browser, Duration::from_secs(2)).await;

    let result = f.open("mfa", WindowMode::Managed, 15_000).await.unwrap();
    assert_eq!(result.outcome, LoginOutcome::MfaHandoff);
    f.wait_for("hand-over", |f| f.state() == Some(State::HandedOver)).await;
    assert_eq!(f.recorder.outcomes(), vec![LoginOutcome::MfaHandoff, LoginOutcome::Timeout]);

    f.sessions.close(SITE).await.unwrap();
    f.mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_the_window_by_hand_is_noticed() {
    let Some((browser, _turn)) = start("closing_the_window_by_hand_is_noticed").await else { return };
    let f = fixture(browser, Duration::from_secs(60)).await;

    let result = f.open("simple", WindowMode::Managed, 15_000).await.unwrap();
    assert_eq!(result.outcome, LoginOutcome::Success);
    let (pid, profile) = f.process();

    kill(pid);
    f.wait_for("the window to be noticed gone", |f| f.recorder.ended() == vec![(SITE, Ended::WindowClosed)]).await;
    assert!(f.sessions.list().is_empty());
    // The fresh profile is deleted once Edge is gone.
    f.wait_for("profile deleted", |_| !profile.exists()).await;
    f.mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_site_opens_once_and_close_all_closes_it() {
    let Some((browser, _turn)) = start("a_site_opens_once_and_close_all_closes_it").await else { return };
    let f = fixture(browser, Duration::from_secs(60)).await;

    assert_eq!(f.open("simple", WindowMode::Managed, 15_000).await.unwrap().outcome, LoginOutcome::Success);
    assert_eq!(f.state(), Some(State::LoggedIn));
    let again = f.open("simple", WindowMode::Managed, 15_000).await;
    assert!(again.unwrap_err().contains("already open"));
    let (pid, _) = f.process();

    f.sessions.close_all().await;
    assert!(f.sessions.list().is_empty());
    assert_eq!(f.recorder.ended(), vec![(SITE, Ended::Closed)]);
    assert!(!process_alive(pid), "a managed window closes with the app");

    // The progress the panel saw for the login, in order, without repeats.
    let mut states: Vec<State> = f.recorder.views.lock().unwrap().iter().map(|v| v.state).collect();
    states.dedup();
    assert_eq!(states, vec![State::Launching, State::Loading, State::Filling, State::Waiting, State::LoggedIn]);
    f.mock.shutdown();
}

#[test]
fn views_serialize_for_the_ui() {
    let view = SessionView {
        site_id: 3,
        site_name: "UniFi".into(),
        state: State::WaitingFor2fa,
        message: "m".into(),
        warnings: vec![SIGNED_IN_WARNING.into()],
        window_mode: WindowMode::Released,
    };
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["state"], "waiting_for_2fa");
    assert_eq!(json["window_mode"], "released");

    // What the Open button gets: no password field, ever.
    let result = OpenResult {
        outcome: LoginOutcome::Success,
        message: "Logged in.".into(),
        warnings: vec![],
        fix: None,
    };
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"outcome":"success","message":"Logged in.","warnings":[],"fix":null}"#
    );
}
