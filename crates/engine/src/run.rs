//! Running a recipe against one browser tab.
//!
//! [`run`] goes through the recipe's steps, checks the origin before every
//! fill, then polls the page until a rule says how the login went:
//!
//! 1. `success` → logged in;
//! 2. `mfa` → a 2FA prompt, so the window is handed to the admin;
//! 3. `failure` → the site rejected the login;
//! 4. none before `timeout_ms` → timed out, and the window is handed over.
//!
//! Every path ends with the window brought to the front: the admin always
//! gets a usable window, whatever happened.

use std::time::Duration;

use netlaunch_vault::{LoginOutcome, normalize_origin};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use tokio::time::Instant;

use crate::driver::{BrowserDriver, DriverError, DriverResult};
use crate::recipe::{DEFAULT_WAIT_MS, FillValue, Recipe, Rule, RuleSet, Step};

/// How often the page is checked against the rules after the last step.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The site being logged in to. The password isn't here: [`run`] asks for
/// it only when a step needs it.
pub struct LoginTarget<'a> {
    /// The saved login URL. Its origin is the only one fills may go to.
    pub login_url: &'a str,
    pub username: &'a str,
}

/// Where a login has got to, for the status line in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Progress {
    /// Opening the login page.
    Loading,
    /// Filling in the form.
    Filling,
    /// The form was sent; waiting for the site to answer.
    Waiting,
}

/// How a login ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    LoggedIn,
    /// The site asked for a 2FA code. NetLaunch never types one; the
    /// window is the admin's now.
    MfaHandoff,
    /// A `failure` rule matched, such as an error message on the page.
    Rejected { rule: String },
    /// Something didn't appear in time. The window is handed over.
    TimedOut { during: String },
    /// A field was on a different origin from the saved site. Nothing was
    /// typed into it.
    OriginMismatch { expected: String, found: String },
    /// The window closed, the page didn't load, or the password couldn't
    /// be read from the vault.
    Error(String),
    /// The page didn't load because its HTTPS certificate isn't trusted,
    /// usually a device with a self-signed certificate. Nothing was typed.
    /// The app offers to turn on the site's self-signed setting and retry.
    UntrustedCertificate { origin: String },
}

impl Outcome {
    /// The value for `login_log.outcome`.
    pub fn log_outcome(&self) -> LoginOutcome {
        match self {
            Outcome::LoggedIn => LoginOutcome::Success,
            Outcome::MfaHandoff => LoginOutcome::MfaHandoff,
            Outcome::Rejected { .. } | Outcome::Error(_) | Outcome::UntrustedCertificate { .. } => {
                LoginOutcome::Failed
            }
            Outcome::TimedOut { .. } => LoginOutcome::Timeout,
            Outcome::OriginMismatch { .. } => LoginOutcome::OriginMismatch,
        }
    }

    /// A sentence for the admin. Never contains a password; it's also what
    /// goes into `login_log.detail`.
    pub fn message(&self) -> String {
        match self {
            Outcome::LoggedIn => "Logged in.".into(),
            Outcome::MfaHandoff => "The site asked for a 2FA code. Enter it in the Edge window.".into(),
            Outcome::Rejected { rule } => format!(
                "The site rejected the login ({rule} appeared). Check the username and password with Edit."
            ),
            Outcome::TimedOut { during } => {
                format!("Timed out while {during}. The window is yours to finish by hand.")
            }
            Outcome::OriginMismatch { expected, found } => format!(
                "Stopped before typing anything: the page is on {found}, but this site is saved as {expected}. \
                 If {found} is the right address, change the site's login URL."
            ),
            Outcome::Error(message) => message.clone(),
            Outcome::UntrustedCertificate { origin } => format!(
                "Edge doesn't trust the certificate of {origin}, so it stopped before showing the login page. \
                 Nothing was typed. Devices on your own network usually have a self-signed certificate: \
                 if this is one, tick \"This site uses a self-signed certificate\" for the site."
            ),
        }
    }
}

/// Logs in to `target` by running `recipe` in `driver`'s tab.
///
/// - `password` is called the first time a step fills `{password}`, and
///   not at all if the login stops before then (for example on an origin
///   mismatch). That keeps the password out of memory for as long as
///   possible.
/// - `progress` is told when the login moves to a new stage.
pub async fn run<D: BrowserDriver>(
    driver: &D,
    recipe: &Recipe,
    target: &LoginTarget<'_>,
    password: &mut (dyn FnMut() -> Result<SecretString, String> + Send),
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Outcome {
    let outcome = run_steps_and_wait(driver, recipe, target, password, progress).await;
    tracing::info!(recipe = %recipe.id, outcome = ?outcome.log_outcome(), "login finished");

    // Hand the window over, whatever happened, unless it's gone.
    let window_gone = matches!(&outcome, Outcome::Error(m) if m.starts_with(WINDOW_CLOSED));
    if !window_gone {
        let _ = driver.bring_to_front().await;
    }
    outcome
}

const WINDOW_CLOSED: &str = "The browser window was closed before the login finished";

/// The closed-window message, with what noticed it (the tab closed, Edge
/// exited, or the library's words) in brackets, for diagnosing.
fn window_closed(detail: &str) -> Outcome {
    tracing::warn!(%detail, "the window closed during the login");
    Outcome::Error(format!("{WINDOW_CLOSED} ({detail})."))
}

async fn run_steps_and_wait<D: BrowserDriver>(
    driver: &D,
    recipe: &Recipe,
    target: &LoginTarget<'_>,
    password: &mut (dyn FnMut() -> Result<SecretString, String> + Send),
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Outcome {
    let expected = match normalize_origin(target.login_url) {
        Ok(origin) => origin,
        Err(e) => return Outcome::Error(format!("The saved login URL can't be used: {e}")),
    };

    // Filled in on the first `{password}` fill, then reused.
    let mut secret: Option<SecretString> = None;
    // Failure rules that already match before the form is sent, such as a
    // banner that's always on the login page. They'd say "rejected" at
    // once, so they are ignored afterwards.
    let mut already_failing: Vec<Rule> = Vec::new();
    let last = recipe.steps.len() - 1;
    let mut said_filling = false;

    for (index, step) in recipe.steps.iter().enumerate() {
        if index == last {
            already_failing = matching_rules(driver, &recipe.failure, &expected).await;
        }
        let number = index + 1;
        match step {
            Step::Goto {} => {
                progress(Progress::Loading);
                tracing::info!(step = number, "goto login URL");
                if let Err(e) = driver.goto(target.login_url).await {
                    return page_did_not_load(e, &expected);
                }
            }
            Step::Wait { selector, timeout_ms } => {
                tracing::info!(step = number, %selector, "wait");
                if let Err(e) = driver.wait_visible(selector, timeout_ms.unwrap_or(DEFAULT_WAIT_MS)).await {
                    return step_failed(e, &format!("waiting for `{selector}` on the login page"), None);
                }
            }
            Step::Fill { selector, value } => {
                if !said_filling {
                    progress(Progress::Filling);
                    said_filling = true;
                }
                tracing::info!(step = number, %selector, "fill");

                // The origin check, before every fill: the frame that holds
                // the field must be on the saved site's origin.
                let frame_url = match driver.frame_url_of(selector).await {
                    Ok(url) => url,
                    Err(e) => return step_failed(e, &format!("looking for `{selector}`"), None),
                };
                let found = describe_origin(&frame_url);
                if found != expected {
                    tracing::warn!(%expected, %found, "origin mismatch; nothing typed");
                    return Outcome::OriginMismatch { expected, found };
                }

                // Only now is the text to type made. For the password, this
                // is the moment it leaves the vault.
                let owned;
                let text: &SecretString = match value {
                    FillValue::Username => {
                        owned = SecretString::from(target.username.to_string());
                        &owned
                    }
                    FillValue::Literal(literal) => {
                        owned = SecretString::from(literal.clone());
                        &owned
                    }
                    FillValue::Password => {
                        if secret.is_none() {
                            match password() {
                                Ok(p) => secret = Some(p),
                                Err(e) => return Outcome::Error(format!("Couldn't read the password from the vault: {e}")),
                            }
                        }
                        secret.as_ref().expect("set above")
                    }
                };
                if let Err(e) = driver.fill(selector, text).await {
                    return step_failed(e, &format!("filling `{selector}`"), Some(text));
                }
            }
            Step::Click { selector, else_press_enter_in } => {
                // No such button on this page? Press Enter in the fallback
                // field instead of waiting for a button that won't come.
                let button_there = driver.is_visible(selector).await.unwrap_or(false);
                match else_press_enter_in {
                    Some(field) if !button_there => {
                        tracing::info!(step = number, %selector, %field, "no button matched; pressing Enter");
                        if let Err(e) = driver.press(field, "Enter").await {
                            return step_failed(e, &format!("pressing Enter in `{field}`"), None);
                        }
                    }
                    _ => {
                        tracing::info!(step = number, %selector, "click");
                        if let Err(e) = driver.click(selector).await {
                            return step_failed(e, &format!("clicking `{selector}`"), None);
                        }
                    }
                }
            }
            Step::Select { selector, value } => {
                tracing::info!(step = number, %selector, "select");
                if let Err(e) = driver.select(selector, value).await {
                    return step_failed(e, &format!("choosing an option in `{selector}`"), None);
                }
            }
            Step::Press { selector, key } => {
                tracing::info!(step = number, %selector, %key, "press");
                if let Err(e) = driver.press(selector, key).await {
                    return step_failed(e, &format!("pressing {key} in `{selector}`"), None);
                }
            }
        }
    }
    // The password isn't needed any more; dropping it wipes it.
    drop(secret);

    progress(Progress::Waiting);
    wait_for_result(driver, recipe, &expected, &already_failing).await
}

/// True if the recipe's `success` rule matches the page right now. For
/// watching a window after the hand-over (M6): the admin types the 2FA
/// code, and this notices when the site lets them in.
///
/// Only reads the page; it never types or clicks. A closed window is an
/// error (`DriverError::Closed`), so the caller can stop watching.
pub async fn logged_in<D: BrowserDriver>(driver: &D, recipe: &Recipe, login_url: &str) -> DriverResult<bool> {
    let expected = normalize_origin(login_url).map_err(|e| DriverError::Other(e.to_string()))?;
    // Fails with Closed when the window is gone; anything else (the page is
    // mid-load) just means "not yet".
    if let Err(DriverError::Closed(m)) = driver.current_url().await {
        return Err(DriverError::Closed(m));
    }
    Ok(first_match(driver, &recipe.success, &expected).await.is_some())
}

/// Polls the page until a rule matches or the recipe's timeout runs out.
async fn wait_for_result<D: BrowserDriver>(
    driver: &D,
    recipe: &Recipe,
    expected: &str,
    already_failing: &[Rule],
) -> Outcome {
    let deadline = Instant::now() + Duration::from_millis(recipe.timeout_ms);
    let failure = RuleSet {
        any: recipe.failure.any.iter().filter(|r| !already_failing.contains(r)).cloned().collect(),
    };
    loop {
        // The window being closed is the only error that ends the wait.
        // Others (the page is halfway through loading) just mean "not yet".
        if let Err(DriverError::Closed(detail)) = driver.current_url().await {
            return window_closed(&detail);
        }
        // Order matters: a dashboard can have an alert banner, and a 2FA
        // page can mention an error, so success wins, then 2FA.
        if let Some(rule) = first_match(driver, &recipe.success, expected).await {
            tracing::info!(%rule, "success rule matched");
            return Outcome::LoggedIn;
        }
        if let Some(rule) = first_match(driver, &recipe.mfa, expected).await {
            tracing::info!(%rule, "2FA rule matched");
            return Outcome::MfaHandoff;
        }
        if let Some(rule) = first_match(driver, &failure, expected).await {
            tracing::info!(%rule, "failure rule matched");
            return Outcome::Rejected { rule: rule.to_string() };
        }
        if Instant::now() >= deadline {
            let seconds = recipe.timeout_ms / 1000;
            return Outcome::TimedOut { during: format!("waiting {seconds} s for the site to show the login worked") };
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// The first rule in `set` that matches the page now.
async fn first_match<D: BrowserDriver>(driver: &D, set: &RuleSet, expected: &str) -> Option<Rule> {
    // Read lazily, and at most once per check: most rules don't need them.
    let mut url: Option<String> = None;
    let mut text: Option<String> = None;
    for rule in &set.any {
        let matched = match rule {
            Rule::Selector(selector) => driver.is_visible(selector).await.unwrap_or(false),
            Rule::Text(needle) => {
                if text.is_none() {
                    text = Some(driver.page_text().await.unwrap_or_default().to_lowercase());
                }
                text.as_deref().is_some_and(|t| t.contains(&needle.to_lowercase()))
            }
            Rule::UrlContains(part) | Rule::UrlNotContains(part) => {
                if url.is_none() {
                    url = Some(driver.current_url().await.unwrap_or_default());
                }
                let url = url.as_deref().unwrap_or("");
                // URL rules only count on the site's own origin. Otherwise
                // "the URL no longer contains /login" would also be true of
                // an error page, or of a redirect to some other site.
                let on_site = describe_origin(url) == expected;
                on_site && (url.contains(part.as_str()) == matches!(rule, Rule::UrlContains(_)))
            }
            Rule::SelectorGone(selector) => {
                // Checked twice, a second apart: while a page reloads, its
                // fields are gone for a moment too. Any error (the page is
                // loading, the window closed) counts as "still there".
                if gone(driver, selector, expected).await {
                    tokio::time::sleep(GONE_FOR).await;
                    gone(driver, selector, expected).await
                } else {
                    false
                }
            }
        };
        if matched {
            return Some(rule.clone());
        }
    }
    None
}

/// How long a `selector_gone` rule's element must stay gone.
const GONE_FOR: Duration = Duration::from_secs(1);

/// True if the page is on the site's origin and shows nothing matching
/// `selector`.
async fn gone<D: BrowserDriver>(driver: &D, selector: &str, expected: &str) -> bool {
    let on_site = driver.current_url().await.is_ok_and(|url| describe_origin(&url) == expected);
    on_site && matches!(driver.is_visible(selector).await, Ok(false))
}

/// Every rule in `set` that matches now.
async fn matching_rules<D: BrowserDriver>(driver: &D, set: &RuleSet, expected: &str) -> Vec<Rule> {
    let mut matched = Vec::new();
    for rule in &set.any {
        let one = RuleSet { any: vec![rule.clone()] };
        if first_match(driver, &one, expected).await.is_some() {
            matched.push(rule.clone());
        }
    }
    matched
}

/// The normalized origin of `url`, or a short description when it has
/// none (`about:blank`, a browser error page).
fn describe_origin(url: &str) -> String {
    match normalize_origin(url) {
        Ok(origin) => origin,
        Err(_) => {
            let scheme = url.split(':').next().unwrap_or("");
            format!("a non-web page ({scheme}:)")
        }
    }
}

/// Why the login page didn't load, in words the admin can act on. The
/// browser reports network failures as `net::ERR_...` codes.
fn page_did_not_load(error: DriverError, origin: &str) -> Outcome {
    let text = match error {
        DriverError::Closed(detail) => return window_closed(&detail),
        DriverError::Timeout(_) => {
            return Outcome::Error(format!("{origin} took too long to load. Check the device is on and reachable."));
        }
        DriverError::Other(text) => text,
    };
    let code = net_error_code(&text);
    let message = match code.as_deref() {
        Some(c) if c.starts_with("ERR_CERT_") => {
            return Outcome::UntrustedCertificate { origin: origin.to_string() };
        }
        Some("ERR_CONNECTION_REFUSED") => {
            format!("Nothing answered at {origin}. Check the device is on and the port in the URL is right.")
        }
        Some("ERR_NAME_NOT_RESOLVED") => {
            format!("The address in {origin} couldn't be found. Check the URL for typos.")
        }
        Some("ERR_CONNECTION_TIMED_OUT" | "ERR_TIMED_OUT" | "ERR_ADDRESS_UNREACHABLE" | "ERR_NETWORK_ACCESS_DENIED") => {
            format!("{origin} didn't answer. Check you're on the right network (or VPN) and the device is on.")
        }
        Some("ERR_SSL_PROTOCOL_ERROR" | "ERR_SSL_VERSION_OR_CIPHER_MISMATCH") => format!(
            "Edge couldn't make a secure connection to {origin}. If the device only offers http, change the URL \
             to http://; old devices may also use a TLS version Edge no longer accepts."
        ),
        Some("ERR_CONNECTION_RESET" | "ERR_EMPTY_RESPONSE") => format!(
            "{origin} dropped the connection. If the URL starts with https://, check the device really serves \
             https on that port."
        ),
        Some(other) => format!("The login page didn't load ({other})."),
        None => format!("The login page didn't load: {text}"),
    };
    Outcome::Error(message)
}

/// `ERR_CERT_AUTHORITY_INVALID` from a message containing
/// `net::ERR_CERT_AUTHORITY_INVALID`.
fn net_error_code(text: &str) -> Option<String> {
    let start = text.find("net::ERR_")? + "net::".len();
    let code: String = text[start..].chars().take_while(|c| c.is_ascii_uppercase() || *c == '_').collect();
    Some(code)
}

/// Turns a failed step into an outcome. `typed` is the text a fill step
/// was typing: if the library's error message repeats it, it's removed.
fn step_failed(error: DriverError, doing: &str, typed: Option<&SecretString>) -> Outcome {
    let redact = |message: String| match typed {
        Some(secret) if !secret.expose_secret().is_empty() => message.replace(secret.expose_secret(), "[redacted]"),
        _ => message,
    };
    match error {
        DriverError::Closed(detail) => window_closed(&detail),
        DriverError::Timeout(_) => Outcome::TimedOut { during: doing.to_string() },
        DriverError::Other(message) => Outcome::Error(format!("Failed while {doing}: {}", redact(message))),
    }
}
