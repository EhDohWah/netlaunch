//! The M5 "done when" tests: the login engine, through the real driver and
//! a real browser, against every scenario on the mock site.
//!
//! **Which browser:** `NETLAUNCH_TEST_BROWSER` if set, else the installed
//! Edge (Windows), else a Chrome or Chromium found in the usual Linux
//! places. With none, each test prints why and passes without running,
//! unless `NETLAUNCH_REQUIRE_BROWSER=1` (CI sets it), which makes a missing
//! browser a failure.
//!
//! The browser runs headless; everything else is as in the app: NetLaunch
//! launches it, attaches over CDP, and runs the built-in recipes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use netlaunch_driver_pw::{LaunchOptions, Profile, Session};
use netlaunch_engine::{BrowserDriver, LoginTarget, Outcome, Recipe, builtin, run};
use netlaunch_mock_site::{MFA_CODE, MockConfig, PASSWORD, RunningMock, USERNAME};
use secrecy::SecretString;
use tokio::sync::Semaphore;

fn browser() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NETLAUNCH_TEST_BROWSER") {
        return Some(PathBuf::from(path));
    }
    if let Some(edge) = netlaunch_driver_pw::find_edge() {
        return Some(edge);
    }
    [
        "/usr/bin/microsoft-edge",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/opt/pw-browsers/chromium",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// The browser to use, or `None` to skip (see the module docs).
fn browser_or_skip(test: &str) -> Option<PathBuf> {
    let found = browser();
    if found.is_none() {
        if std::env::var("NETLAUNCH_REQUIRE_BROWSER").as_deref() == Ok("1") {
            panic!("{test}: no browser found and NETLAUNCH_REQUIRE_BROWSER=1; set NETLAUNCH_TEST_BROWSER");
        }
        eprintln!("{test}: SKIPPED, no Edge or Chrome found (set NETLAUNCH_TEST_BROWSER)");
    }
    found
}

/// Every log line from every test in this file, collected by one global
/// subscriber with the app's filter at its most verbose: `trace` for
/// NetLaunch, capped at `debug` for playwright-rs.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        let writer = capture.clone();
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new(netlaunch_engine::log_directives("trace")))
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .init();
        capture
    })
}

/// At most two browsers at once, so a 2-core CI runner keeps up.
static BROWSERS: Semaphore = Semaphore::const_new(2);

struct Run {
    outcome: Outcome,
    session: Session,
    mock: RunningMock,
}

/// Starts the mock site and a browser, and logs in to `scenario` with
/// `recipe`.
async fn login(browser: PathBuf, scenario: &str, recipe: &Recipe) -> Run {
    capture();
    let mock = netlaunch_mock_site::serve(MockConfig { port: 0, lookalike_port: 0, slow_delay_ms: 5000 })
        .await
        .expect("mock site starts");

    let mut extra_args = Vec::new();
    // A Linux container usually runs as root, where Chromium needs this.
    if cfg!(target_os = "linux") {
        extra_args.push("--no-sandbox".to_string());
    }
    let session = Session::launch(LaunchOptions {
        browser,
        profile: Profile::Fresh,
        // The mock site's certificate is self-signed.
        accept_self_signed: true,
        headless: true,
        extra_args,
    })
    .await
    .expect("browser launches and attaches");

    let login_url = format!("{}/{scenario}/login", mock.base_url);
    let target = LoginTarget { login_url: &login_url, username: USERNAME };
    let outcome = run(
        &session,
        recipe,
        &target,
        &mut || Ok(SecretString::from(PASSWORD.to_string())),
        &mut |_| {},
    )
    .await;
    Run { outcome, session, mock }
}

async fn finish(run: Run) {
    assert!(run.session.close().await, "Edge quits cleanly on managed close");
    run.mock.shutdown();
}

fn recipe(id: &str, timeout_ms: u64) -> Recipe {
    let mut recipe = builtin(id).unwrap();
    recipe.timeout_ms = timeout_ms;
    recipe
}

/// The browser to test with, plus a turn to use it (at most two at once).
/// `None` means skip.
async fn start(test: &str) -> Option<(PathBuf, tokio::sync::SemaphorePermit<'static>)> {
    let browser = browser_or_skip(test)?;
    Some((browser, BROWSERS.acquire().await.unwrap()))
}

#[tokio::test(flavor = "multi_thread")]
async fn simple_logs_in() {
    let Some((browser, _turn)) = start("simple_logs_in").await else { return };
    let run = login(browser, "simple", &recipe("generic-form", 15_000)).await;
    assert_eq!(run.outcome, Outcome::LoggedIn);
    assert!(run.session.is_visible("#welcome").await.unwrap());
    assert!(run.session.isolation_warnings().await.is_empty());
    finish(run).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_step_logs_in_with_its_recipe() {
    let Some((browser, _turn)) = start("two_step_logs_in_with_its_recipe").await else { return };
    let run = login(browser, "two-step", &recipe("two-step-form", 15_000)).await;
    assert_eq!(run.outcome, Outcome::LoggedIn);
    finish(run).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spa_controlled_form_logs_in() {
    let Some((browser, _turn)) = start("spa_controlled_form_logs_in").await else { return };
    // The SPA only sees text typed with real input events, and changes the
    // URL with pushState, without a page load.
    let run = login(browser, "spa", &recipe("generic-form", 15_000)).await;
    assert_eq!(run.outcome, Outcome::LoggedIn);
    assert!(run.session.current_url().await.unwrap().ends_with("/spa/home"));
    finish(run).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_form_is_waited_for() {
    let Some((browser, _turn)) = start("slow_form_is_waited_for").await else { return };
    // The form appears after 5 s; the recipe's wait step allows 15 s.
    let run = login(browser, "slow", &recipe("generic-form", 15_000)).await;
    assert_eq!(run.outcome, Outcome::LoggedIn);
    finish(run).await;
}

/// A password-only device whose address doesn't change on login: the
/// recipe types no username, presses Enter (the button isn't a submit-type
/// one) and sees the password field go away.
#[tokio::test(flavor = "multi_thread")]
async fn password_only_device_logs_in() {
    let Some((browser, _turn)) = start("password_only_device_logs_in").await else { return };
    let run = login(browser, "password-only", &recipe("password-only", 15_000)).await;
    assert_eq!(run.outcome, Outcome::LoggedIn);
    assert!(run.session.is_visible("#welcome").await.unwrap());
    assert!(run.session.current_url().await.unwrap().ends_with("/password-only/login"));
    finish(run).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_is_rejected() {
    let Some((browser, _turn)) = start("wrong_password_is_rejected").await else { return };
    let run = login(browser, "wrong-password", &recipe("generic-form", 15_000)).await;
    assert!(matches!(run.outcome, Outcome::Rejected { .. }), "{:?}", run.outcome);
    finish(run).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_away_types_nothing_on_the_lookalike() {
    let Some((browser, _turn)) = start("redirect_away_types_nothing_on_the_lookalike").await else { return };
    let run = login(browser, "redirect-away", &recipe("generic-form", 15_000)).await;
    match &run.outcome {
        // The mock's URLs carry explicit ports, so they are already in
        // normalized form.
        Outcome::OriginMismatch { expected, found } => {
            assert_eq!(expected, &run.mock.base_url);
            assert_eq!(found, &run.mock.lookalike_url);
        }
        other => panic!("expected an origin mismatch, got {other:?}"),
    }
    // Give a stray beacon time to arrive before counting.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let hits = run.mock.state.lookalike_hits();
    assert_eq!((hits.input_events, hits.submits), (0, 0), "nothing typed or sent on the lookalike");
    finish(run).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mfa_hands_over_and_never_types_the_code() {
    let Some((browser, _turn)) = start("mfa_hands_over_and_never_types_the_code").await else { return };
    let run = login(browser, "mfa", &recipe("generic-form", 15_000)).await;
    assert_eq!(run.outcome, Outcome::MfaHandoff);

    // The admin's part, played by the test through the driver: type the
    // code and send it. The engine itself never does this.
    let code_field = "input[autocomplete=one-time-code]";
    run.session.fill(code_field, &SecretString::from(MFA_CODE.to_string())).await.unwrap();
    run.session.click("button[type=submit]").await.unwrap();
    run.session.wait_visible("#welcome", 10_000).await.unwrap();
    finish(run).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_success_times_out_and_leaves_the_window() {
    let Some((browser, _turn)) = start("no_success_times_out_and_leaves_the_window").await else { return };
    let run = login(browser, "no-success", &recipe("generic-form", 3_000)).await;
    assert!(matches!(run.outcome, Outcome::TimedOut { .. }), "{:?}", run.outcome);
    assert!(run.session.is_running(), "the window is still there for the admin");
    finish(run).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn untrusted_certificate_fails_with_a_hint() {
    let Some((browser, _turn)) = start("untrusted_certificate_fails_with_a_hint").await else { return };
    capture();
    let mock = netlaunch_mock_site::serve(MockConfig { port: 0, lookalike_port: 0, slow_delay_ms: 0 }).await.unwrap();
    let session = Session::launch(LaunchOptions {
        browser,
        profile: Profile::Fresh,
        accept_self_signed: false,
        headless: true,
        extra_args: if cfg!(target_os = "linux") { vec!["--no-sandbox".into()] } else { vec![] },
    })
    .await
    .unwrap();
    let login_url = format!("{}/simple/login", mock.base_url);
    let target = LoginTarget { login_url: &login_url, username: USERNAME };
    let outcome = run(
        &session,
        &recipe("generic-form", 5_000),
        &target,
        &mut || panic!("the password must not be read when the page didn't load"),
        &mut |_| {},
    )
    .await;
    assert_eq!(outcome, Outcome::UntrustedCertificate { origin: mock.base_url.clone() });
    assert!(outcome.message().contains("self-signed"));
    assert!(session.close().await);
    mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_password_never_reaches_the_log() {
    let Some((browser, _turn)) = start("the_password_never_reaches_the_log").await else { return };
    let run = login(browser, "simple", &recipe("generic-form", 15_000)).await;
    assert_eq!(run.outcome, Outcome::LoggedIn);
    finish(run).await;

    // Every test in this file types the same password, so this covers the
    // ones that ran alongside too.
    let log = String::from_utf8(capture().0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("fill"), "the capture should hold the login's log lines");
    assert!(!log.contains(PASSWORD), "the password is in the log");
}
