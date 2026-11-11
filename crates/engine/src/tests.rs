//! Engine tests against the fake tab. The real browser runs in
//! `netlaunch-driver-pw`'s tests, against the mock site.

use std::sync::{Arc, Mutex};

use netlaunch_vault::LoginOutcome;
use secrecy::SecretString;

use crate::fake::{ALERT, FakeDriver, OTP, PASS, SUBMIT, USER};
use crate::recipe::{Recipe, builtin};
use crate::run::{LoginTarget, Outcome, Progress, run};
use crate::DriverError;

const LOGIN_URL: &str = "https://console.example:8443/login";
const USERNAME: &str = "test-admin";
const PASSWORD: &str = "Test-Pa55-unit";

/// The generic recipe with a short result timeout, so timeouts don't make
/// the tests slow.
fn generic(timeout_ms: u64) -> Recipe {
    let mut recipe = builtin("generic-form").unwrap();
    recipe.timeout_ms = timeout_ms;
    recipe
}

/// The outcome, how often the password was asked for, and the progress
/// stages seen.
async fn login(driver: &FakeDriver, recipe: &Recipe) -> (Outcome, usize, Vec<Progress>) {
    let mut asked = 0;
    let mut stages = Vec::new();
    let target = LoginTarget { login_url: LOGIN_URL, username: USERNAME };
    let outcome = run(
        driver,
        recipe,
        &target,
        &mut || {
            asked += 1;
            Ok(SecretString::from(PASSWORD.to_string()))
        },
        &mut |p| stages.push(p),
    )
    .await;
    (outcome, asked, stages)
}

#[tokio::test]
async fn logs_in_to_a_simple_form() {
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_click(|page, _| page.url = "https://console.example:8443/home".into());

    let (outcome, asked, stages) = login(&driver, &generic(1000)).await;

    assert_eq!(outcome, Outcome::LoggedIn);
    assert_eq!(outcome.log_outcome(), LoginOutcome::Success);
    assert_eq!(asked, 1);
    assert_eq!(
        driver.fills(),
        vec![(USER.to_string(), USERNAME.to_string()), (PASS.to_string(), PASSWORD.to_string())]
    );
    assert_eq!(driver.clicks(), vec![SUBMIT.to_string()]);
    assert_eq!(stages, vec![Progress::Loading, Progress::Filling, Progress::Waiting]);
    assert!(driver.brought_to_front());
}

#[tokio::test]
async fn default_port_in_the_saved_url_still_matches() {
    // Saved without a port, page reports :443 (or the other way round):
    // both normalize to the same origin.
    let driver = FakeDriver::login_form_at("https://console.example:443/login");
    driver.on_click(|page, _| page.url = "https://console.example/home".into());
    let target = LoginTarget { login_url: "https://Console.Example/login", username: USERNAME };
    let outcome = run(
        &driver,
        &generic(1000),
        &target,
        &mut || Ok(SecretString::from(PASSWORD.to_string())),
        &mut |_| {},
    )
    .await;
    assert_eq!(outcome, Outcome::LoggedIn);
}

#[tokio::test]
async fn redirect_to_another_origin_types_nothing() {
    // Like the mock site's /redirect-away: the login page sends the
    // browser to another port before the form.
    let driver = FakeDriver::login_form_at("https://console.example:8444/login");

    let (outcome, asked, _) = login(&driver, &generic(1000)).await;

    assert_eq!(
        outcome,
        Outcome::OriginMismatch {
            expected: "https://console.example:8443".into(),
            found: "https://console.example:8444".into()
        }
    );
    assert_eq!(outcome.log_outcome(), LoginOutcome::OriginMismatch);
    assert!(driver.fills().is_empty(), "nothing may be typed on another origin");
    assert_eq!(asked, 0, "the password must not even leave the vault");
    assert!(driver.brought_to_front());
}

#[tokio::test]
async fn origin_is_checked_before_every_fill() {
    // The page moves to another origin after the username is typed.
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_fill(|page, _| page.url = "https://evil.example/login".into());

    let (outcome, asked, _) = login(&driver, &generic(1000)).await;

    assert!(matches!(outcome, Outcome::OriginMismatch { ref found, .. } if found == "https://evil.example:443"));
    assert_eq!(driver.fills().len(), 1, "only the username was typed, on the right origin");
    assert_eq!(asked, 0);
}

#[tokio::test]
async fn a_non_web_page_is_a_mismatch() {
    let driver = FakeDriver::login_form_at("about:blank");
    let (outcome, _, _) = login(&driver, &generic(1000)).await;
    assert!(matches!(outcome, Outcome::OriginMismatch { ref found, .. } if found.contains("about:")));
    assert!(driver.fills().is_empty());
}

#[tokio::test]
async fn error_message_means_rejected() {
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_click(|page, _| {
        page.visible.insert(ALERT.into());
    });

    let (outcome, _, _) = login(&driver, &generic(5000)).await;

    assert!(matches!(outcome, Outcome::Rejected { .. }), "{outcome:?}");
    assert_eq!(outcome.log_outcome(), LoginOutcome::Failed);
    assert!(outcome.message().contains("rejected"));
}

#[tokio::test]
async fn an_alert_that_was_there_before_submitting_is_ignored() {
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_goto(|page, _| {
        page.url = LOGIN_URL.into();
        page.visible = [USER, PASS, SUBMIT, ALERT].map(String::from).into();
    });

    let (outcome, _, _) = login(&driver, &generic(600)).await;

    assert!(matches!(outcome, Outcome::TimedOut { .. }), "{outcome:?}");
}

#[tokio::test]
async fn otp_field_means_mfa_handoff() {
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_click(|page, _| {
        page.url = "https://console.example:8443/login/verify".into();
        page.visible = [OTP.to_string()].into();
    });

    let (outcome, _, _) = login(&driver, &generic(5000)).await;

    assert_eq!(outcome, Outcome::MfaHandoff);
    assert_eq!(outcome.log_outcome(), LoginOutcome::MfaHandoff);
    assert!(driver.brought_to_front());
}

#[tokio::test]
async fn mfa_text_matches_ignoring_case() {
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_click(|page, _| {
        page.visible.clear();
        page.text = "Enter the Verification Code from your app".into();
    });
    let (outcome, _, _) = login(&driver, &generic(5000)).await;
    assert_eq!(outcome, Outcome::MfaHandoff);
}

#[tokio::test]
async fn success_beats_an_alert_on_the_dashboard() {
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_click(|page, _| {
        page.url = "https://console.example:8443/home".into();
        page.visible.insert(ALERT.into());
    });
    let (outcome, _, _) = login(&driver, &generic(5000)).await;
    assert_eq!(outcome, Outcome::LoggedIn);
}

#[tokio::test]
async fn url_rules_only_count_on_the_sites_origin() {
    // After submitting, the browser ends up on another site (or an error
    // page). Its URL has no "/login", but that isn't a successful login.
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_click(|page, _| page.url = "chrome-error://chromewebdata/".into());
    let (outcome, _, _) = login(&driver, &generic(600)).await;
    assert!(matches!(outcome, Outcome::TimedOut { .. }), "{outcome:?}");
}

#[tokio::test]
async fn nothing_matching_times_out_and_hands_over() {
    // Like /no-success: logged in, but no rule ever matches.
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    let (outcome, _, _) = login(&driver, &generic(600)).await;
    assert!(matches!(outcome, Outcome::TimedOut { .. }), "{outcome:?}");
    assert_eq!(outcome.log_outcome(), LoginOutcome::Timeout);
    assert!(driver.brought_to_front());
}

#[tokio::test]
async fn form_that_never_appears_times_out() {
    let driver = FakeDriver::default();
    driver.on_goto(|page, url| page.url = url.to_string());
    let (outcome, asked, _) = login(&driver, &generic(1000)).await;
    assert!(matches!(outcome, Outcome::TimedOut { ref during } if during.contains("input[type=password]")));
    assert_eq!(asked, 0);
}

/// A tab where loading the page fails with `error`.
/// A device that only asks for a password (a Ruijie access point): no
/// username is typed, Enter submits, and the password field going away
/// is the sign of success, because the address doesn't change.
#[tokio::test]
async fn password_only_recipe_types_just_the_password() {
    let driver = FakeDriver::default();
    driver.on_goto(|page, _| page.visible = [PASS.to_string()].into());
    driver.on_click(|page, _| page.visible.clear());
    let mut recipe = builtin("password-only").unwrap();
    recipe.timeout_ms = 3000;

    let (outcome, asked, _) = login(&driver, &recipe).await;

    assert_eq!(outcome, Outcome::LoggedIn);
    assert_eq!(asked, 1);
    assert_eq!(driver.fills(), vec![(PASS.to_string(), PASSWORD.to_string())]);
    assert_eq!(driver.presses(), vec![(PASS.to_string(), "Enter".to_string())]);
}

#[tokio::test]
async fn password_only_recipe_notices_a_rejected_password() {
    let driver = FakeDriver::default();
    driver.on_goto(|page, _| page.visible = [PASS.to_string()].into());
    // The form stays and an error appears.
    driver.on_click(|page, _| {
        page.visible.insert(ALERT.to_string());
    });
    let mut recipe = builtin("password-only").unwrap();
    recipe.timeout_ms = 3000;

    let (outcome, _, _) = login(&driver, &recipe).await;
    assert!(matches!(outcome, Outcome::Rejected { .. }), "{outcome:?}");
}

/// The field is gone, but the browser has left the site: not a success.
#[tokio::test]
async fn a_gone_field_on_another_origin_is_not_success() {
    let driver = FakeDriver::default();
    driver.on_goto(|page, _| page.visible = [PASS.to_string()].into());
    driver.on_click(|page, _| {
        page.visible.clear();
        page.url = "https://elsewhere.example/".into();
    });
    let mut recipe = builtin("password-only").unwrap();
    recipe.timeout_ms = 1500;

    let (outcome, _, _) = login(&driver, &recipe).await;
    assert!(matches!(outcome, Outcome::TimedOut { .. }), "{outcome:?}");
}

fn failing_goto(error: &str) -> FakeDriver {
    let driver = FakeDriver::default();
    driver.fail_goto_with(DriverError::Other(error.to_string()));
    driver
}

#[tokio::test]
async fn certificate_errors_are_recognized() {
    // The shape of Playwright's real message, stack trace included.
    let driver = failing_goto(
        "Protocol error: net::ERR_CERT_AUTHORITY_INVALID at https://console.example:8443/login \
         Error: net::ERR_CERT_AUTHORITY_INVALID at _FrameSession._navigate (C:\\x\\coreBundle.js:1:2)",
    );
    let (outcome, asked, _) = login(&driver, &generic(1000)).await;
    assert_eq!(outcome, Outcome::UntrustedCertificate { origin: "https://console.example:8443".into() });
    assert_eq!(outcome.log_outcome(), LoginOutcome::Failed);
    assert_eq!(asked, 0);
    let message = outcome.message();
    assert!(message.contains("self-signed certificate"), "{message}");
    assert!(!message.contains("coreBundle"), "no stack trace: {message}");
}

/// A site that redirects at once makes the browser report the first
/// navigation as aborted (seen in Edge on Windows with `/redirect-away`).
/// That isn't a failure: the login carries on, and the origin check still
/// decides whether anything is typed.
#[tokio::test]
async fn an_aborted_first_navigation_is_not_an_error() {
    // The redirect stayed on the site: the form is there, so log in.
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.fail_goto_with(DriverError::Other("net::ERR_ABORTED at https://console.example:8443/login".into()));
    driver.on_click(|page, _| page.url = "https://console.example:8443/home".into());
    let (outcome, _, _) = login(&driver, &generic(1000)).await;
    assert_eq!(outcome, Outcome::LoggedIn);

    // The redirect left the site: a mismatch, and nothing typed.
    let driver = FakeDriver::login_form_at("https://console.example:8444/login");
    driver.fail_goto_with(DriverError::Other("net::ERR_ABORTED".into()));
    let (outcome, asked, _) = login(&driver, &generic(1000)).await;
    assert!(matches!(outcome, Outcome::OriginMismatch { .. }), "{outcome:?}");
    assert_eq!(asked, 0);
    assert!(driver.fills().is_empty());
}

#[tokio::test]
async fn network_errors_get_plain_messages() {
    for (code, expect) in [
        ("ERR_CONNECTION_REFUSED", "Nothing answered"),
        ("ERR_NAME_NOT_RESOLVED", "couldn't be found"),
        ("ERR_ADDRESS_UNREACHABLE", "didn't answer"),
        ("ERR_SSL_PROTOCOL_ERROR", "secure connection"),
        ("ERR_BLOCKED_BY_CLIENT", "(ERR_BLOCKED_BY_CLIENT)"),
    ] {
        let driver = failing_goto(&format!("Protocol error: net::{code} at https://x/ at _Frame (C:\\p.js:1:1)"));
        let (outcome, _, _) = login(&driver, &generic(1000)).await;
        let message = outcome.message();
        assert!(message.contains(expect), "{code}: {message}");
        assert!(!message.contains("p.js"), "{code}: {message}");
    }
}

#[tokio::test]
async fn without_a_submit_button_enter_is_pressed_in_the_password_field() {
    // Like UniFi or a Huawei switch: the login button isn't type=submit.
    let driver = FakeDriver::default();
    driver.on_goto(|page, _| {
        page.url = LOGIN_URL.into();
        page.visible = [USER, PASS].map(String::from).into();
    });
    driver.on_click(|page, _| page.url = "https://console.example:8443/home".into());

    let (outcome, _, _) = login(&driver, &generic(1000)).await;

    assert_eq!(outcome, Outcome::LoggedIn);
    assert!(driver.clicks().is_empty());
    assert_eq!(driver.presses(), vec![(PASS.to_string(), "Enter".to_string())]);
}

#[tokio::test]
async fn logged_in_watches_only_the_success_rule() {
    let recipe = generic(1000);
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.goto_now(LOGIN_URL).await;

    // On the 2FA page: not logged in yet, even with an error showing.
    driver.on_click(|page, _| {
        page.url = "https://console.example:8443/login/verify".into();
        page.visible = [OTP, ALERT].map(String::from).into();
    });
    driver.click_now(SUBMIT).await;
    assert_eq!(crate::logged_in(&driver, &recipe, LOGIN_URL).await, Ok(false));

    // The admin typed the code; the dashboard shows.
    driver.on_click(|page, _| page.url = "https://console.example:8443/home".into());
    driver.click_now(OTP).await;
    assert_eq!(crate::logged_in(&driver, &recipe, LOGIN_URL).await, Ok(true));

    // A "dashboard" on another origin doesn't count.
    driver.on_click(|page, _| page.url = "https://elsewhere.example/home".into());
    driver.click_now(OTP).await;
    assert_eq!(crate::logged_in(&driver, &recipe, LOGIN_URL).await, Ok(false));

    driver.on_click(|page, _| page.closed = true);
    driver.click_now(OTP).await;
    assert!(matches!(crate::logged_in(&driver, &recipe, LOGIN_URL).await, Err(DriverError::Closed(_))));
}

#[tokio::test]
async fn closed_window_is_an_error_without_hand_over() {
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_click(|page, _| page.closed = true);
    let (outcome, _, _) = login(&driver, &generic(5000)).await;
    assert!(matches!(outcome, Outcome::Error(ref m) if m.contains("closed")), "{outcome:?}");
    assert!(!driver.brought_to_front());
}

#[tokio::test]
async fn vault_error_stops_before_the_password_field() {
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    let target = LoginTarget { login_url: LOGIN_URL, username: USERNAME };
    let outcome = run(&driver, &generic(1000), &target, &mut || Err("locked".into()), &mut |_| {}).await;
    assert!(matches!(outcome, Outcome::Error(ref m) if m.contains("locked")), "{outcome:?}");
    assert_eq!(driver.fills().len(), 1, "only the username");
}

#[tokio::test]
async fn driver_errors_never_repeat_the_password() {
    let driver = FakeDriver::login_form_at(LOGIN_URL);
    // An error message that (hypothetically) quotes what was being typed.
    driver.fail_fills_with(DriverError::Other(format!("could not type \"{PASSWORD}\" or \"{USERNAME}\"")));

    // A recipe whose only fill is the password, so the failing fill is
    // the password one.
    let mut recipe = generic(1000);
    recipe.steps.retain(|s| !matches!(s, crate::recipe::Step::Fill { value: crate::recipe::FillValue::Username, .. }));

    let (outcome, _, _) = login(&driver, &recipe).await;
    let message = outcome.message();
    assert!(!message.contains(PASSWORD), "{message}");
    assert!(message.contains("[redacted]"), "{message}");
}

#[tokio::test]
async fn two_step_recipe_fills_each_page_in_turn() {
    let recipe = {
        let mut r = builtin("two-step-form").unwrap();
        r.timeout_ms = 1000;
        r
    };
    let driver = FakeDriver::default();
    driver.on_goto(|page, _| {
        page.url = LOGIN_URL.into();
        page.visible = [USER, SUBMIT].map(String::from).into();
    });
    let clicks = Arc::new(Mutex::new(0));
    let counter = clicks.clone();
    driver.on_click(move |page, _| {
        let mut n = counter.lock().unwrap();
        *n += 1;
        if *n == 1 {
            page.url = "https://console.example:8443/login/password".into();
            page.visible = [PASS, SUBMIT].map(String::from).into();
        } else {
            page.url = "https://console.example:8443/home".into();
        }
    });

    let (outcome, _, _) = login(&driver, &recipe).await;
    assert_eq!(outcome, Outcome::LoggedIn);
    assert_eq!(driver.fills().len(), 2);
    assert_eq!(*clicks.lock().unwrap(), 2);
}

/// Collects every log line in the test binary. Installed as the global
/// subscriber once, by the first test that needs it. (A per-test
/// subscriber is unreliable here: tracing caches "nobody is listening" per
/// log line, and the other tests run at the same time without one.)
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

fn global_capture() -> &'static Capture {
    static CAPTURE: std::sync::OnceLock<Capture> = std::sync::OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("no other subscriber in engine tests");
        capture
    })
}

#[tokio::test]
async fn the_engine_never_logs_what_it_types() {
    let capture = global_capture();

    let driver = FakeDriver::login_form_at(LOGIN_URL);
    driver.on_click(|page, _| page.url = "https://console.example:8443/home".into());
    let (outcome, _, _) = login(&driver, &generic(1000)).await;
    assert_eq!(outcome, Outcome::LoggedIn);

    // Every test types the same password, so this also covers whatever
    // the tests running alongside logged.
    let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("fill"), "the capture should hold the engine's log lines:\n{log}");
    assert!(!log.contains(PASSWORD), "password found in the log:\n{log}");
}
