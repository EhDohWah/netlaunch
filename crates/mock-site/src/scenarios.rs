//! The login scenarios from the spec's testing plan, one per path prefix.
//!
//! Every scenario follows the same URL shape, so the `generic-form` recipe's
//! success rule (`url_not_contains: "/login"`) works on all of them:
//!
//! - `/<scenario>/login...` is the login flow (forms, steps, errors);
//! - `/<scenario>/home` is the dashboard with the `#welcome` marker.
//!
//! The exception is `/no-success`, which signs you in but keeps `/login`
//! in the URL and shows no marker, on purpose.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde::Deserialize;

use crate::html::{self, escape, page};
use crate::{MFA_CODE, MockState, PASSWORD, PENDING_COOKIE, SESSION_COOKIE, SiteConfig, USERNAME, set_cookie};

/// (path, what it simulates, what NetLaunch should do). Drives the index
/// page; the spec's testing table is the source.
pub const SCENARIOS: &[(&str, &str, &str)] = &[
    ("simple", "One-page username + password form", "Logged in"),
    ("two-step", "Username, Next, then password on a second page", "Logged in (needs its own recipe)"),
    ("spa", "React-style controlled form; changes URL without a page load", "Logged in"),
    ("mfa", "Login, then a 6-digit code page (test code 123456)", "Pauses with \"Waiting for 2FA\"; logged in after the code is typed"),
    ("slow", "Form appears after 5 seconds (?delay=ms to change)", "Waits, then logged in"),
    ("wrong-password", "Always rejects", "Failed, with a clear message"),
    ("redirect-away", "Redirects to a different port before the form", "Stops with origin mismatch; nothing filled"),
    ("no-success", "Logs in but never shows the success marker", "Times out, window handed over"),
    ("password-only", "Only a password field, no submit-type button, same address after login", "Logged in (needs the password-only recipe)"),
];

#[derive(Clone)]
struct AppState {
    mock: MockState,
    site: SiteConfig,
}

pub fn router(mock: MockState, site: SiteConfig) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/_mock/status", get(status))
        .route("/_mock/reset", post(reset))
        // Scenarios with a plain one-page form.
        .route("/simple/login", get(simple_form).post(simple_submit))
        .route("/slow/login", get(slow_form).post(slow_submit))
        .route("/wrong-password/login", get(wrong_password_form).post(wrong_password_submit))
        // Two pages: username, then password.
        .route("/two-step/login", get(two_step_username_form).post(two_step_username_submit))
        .route("/two-step/login/password", get(two_step_password_form).post(two_step_password_submit))
        // Password, then a verification code.
        .route("/mfa/login", get(mfa_form).post(mfa_submit))
        .route("/mfa/login/verify", get(mfa_code_form).post(mfa_code_submit))
        // Single-page app: one HTML shell, a JSON login endpoint.
        .route("/spa/login", get(spa_shell))
        .route("/spa/home", get(spa_home))
        .route("/spa/api/login", post(spa_api_login))
        .route("/redirect-away/login", get(redirect_away))
        .route("/no-success/login", get(no_success_form).post(no_success_submit))
        .route("/no-success/login/signed-in", get(no_success_signed_in))
        .route("/password-only/login", get(password_only_page).post(password_only_submit))
        // Dashboards for the form-based scenarios.
        .route("/{scenario}/home", get(home))
        .with_state(AppState { mock, site })
}

/// Form fields. `default` so a missing field is an empty string (a failed
/// login) rather than a 422 error page.
#[derive(Deserialize, Default)]
struct Credentials {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

fn credentials_ok(c: &Credentials) -> bool {
    c.username == USERNAME && c.password == PASSWORD
}

const BAD_LOGIN: &str = "Invalid username or password";

/// 303 to `location`, setting a cookie.
fn redirect_with_cookie(location: &str, cookie_name: &str, value: &str) -> Response {
    let mut response = Redirect::to(location).into_response();
    if let Ok(cookie) = HeaderValue::from_str(&set_cookie(cookie_name, value)) {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

/// Signs in to `scenario` and sends the browser to its dashboard.
fn sign_in(state: &AppState, scenario: &str, username: &str) -> Response {
    let token = state.mock.start_session(scenario, username);
    redirect_with_cookie(&format!("/{scenario}/home"), SESSION_COOKIE, &token)
}

fn login_page(action: &str, error: Option<&str>) -> Html<String> {
    page("Sign in", &html::login_form(action, error))
}

// ----- index and test helpers -----

async fn index(State(state): State<AppState>) -> Html<String> {
    let rows: String = SCENARIOS
        .iter()
        .map(|(path, simulates, expected)| {
            format!(
                r#"<tr><td><a href="/{path}/login">/{path}</a></td><td>{}</td><td>{}</td></tr>"#,
                escape(simulates),
                escape(expected)
            )
        })
        .collect();
    let hits = state.mock.lookalike_hits();
    page(
        "Scenarios",
        &format!(
            r#"<h1>Mock login scenarios</h1>
<p>Test account (fake): <code>{USERNAME}</code> / <code>{PASSWORD}</code>. MFA code: <code>{MFA_CODE}</code>.</p>
<table>
<tr><th>Scenario</th><th>Simulates</th><th>Expected NetLaunch result</th></tr>
{rows}
</table>
<h2>Lookalike site</h2>
<p><code>/redirect-away</code> sends the browser to <a href="{origin}/redirect-away/login">{origin}</a>, a copy of the login page on another port.
NetLaunch must type nothing there.</p>
<p id="lookalike-hits">Input events: <strong>{inputs}</strong> · submits: <strong>{submits}</strong></p>
<form method="post" action="/_mock/reset"><button type="submit">Reset counters</button></form>"#,
            origin = escape(&state.site.lookalike_origin),
            inputs = hits.input_events,
            submits = hits.submits,
        ),
    )
}

/// JSON for tests: `{"input_events": n, "submits": n}`.
async fn status(State(state): State<AppState>) -> Json<crate::LookalikeHits> {
    Json(state.mock.lookalike_hits())
}

async fn reset(State(state): State<AppState>) -> Redirect {
    state.mock.reset_lookalike_hits();
    Redirect::to("/")
}

// ----- /simple -----

async fn simple_form() -> Html<String> {
    login_page("/simple/login", None)
}

async fn simple_submit(State(state): State<AppState>, Form(c): Form<Credentials>) -> Response {
    if credentials_ok(&c) {
        sign_in(&state, "simple", &c.username)
    } else {
        login_page("/simple/login", Some(BAD_LOGIN)).into_response()
    }
}

// ----- /slow -----

#[derive(Deserialize)]
struct SlowQuery {
    delay: Option<u64>,
}

/// The page loads at once, but the form is only added by script after the
/// delay, like a console that loads its login widget late.
async fn slow_form(State(state): State<AppState>, Query(q): Query<SlowQuery>) -> Html<String> {
    let delay = q.delay.unwrap_or(state.site.slow_delay_ms).min(60_000);
    page(
        "Sign in",
        &format!(
            r#"<p id="loading">Loading the sign-in form…</p>
<template id="form-template">{form}</template>
<script>
  setTimeout(() => {{
    document.getElementById('loading').replaceWith(
      document.getElementById('form-template').content.cloneNode(true));
  }}, {delay});
</script>"#,
            form = html::login_form("/slow/login", None),
        ),
    )
}

async fn slow_submit(State(state): State<AppState>, Form(c): Form<Credentials>) -> Response {
    if credentials_ok(&c) {
        sign_in(&state, "slow", &c.username)
    } else {
        // No delay on the retry page: the form is shown straight away.
        login_page("/slow/login", Some(BAD_LOGIN)).into_response()
    }
}

// ----- /wrong-password -----

async fn wrong_password_form() -> Html<String> {
    login_page("/wrong-password/login", None)
}

/// Rejects everything, including the right password.
async fn wrong_password_submit() -> Html<String> {
    login_page("/wrong-password/login", Some(BAD_LOGIN))
}

// ----- /two-step -----

async fn two_step_username_form() -> Html<String> {
    two_step_username_page(None)
}

fn two_step_username_page(error: Option<&str>) -> Html<String> {
    page(
        "Sign in",
        &format!(
            r#"<h1>Sign in</h1>
{error}
<form method="post" action="/two-step/login">
  <label for="username">Username</label>
  <input id="username" name="username" type="text" autocomplete="username">
  <div><button id="next" type="submit">Next</button></div>
</form>"#,
            error = html::error_line(error),
        ),
    )
}

/// Any username moves on to the password page (real consoles don't reveal
/// whether a user exists); the check happens with the password.
async fn two_step_username_submit(State(state): State<AppState>, Form(c): Form<Credentials>) -> Response {
    if c.username.is_empty() {
        return two_step_username_page(Some("Enter a username")).into_response();
    }
    let token = state.mock.start_pending("two-step", &c.username);
    redirect_with_cookie("/two-step/login/password", PENDING_COOKIE, &token)
}

async fn two_step_password_form(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.mock.pending_user(&headers, "two-step") {
        Some(user) => two_step_password_page(&user, None).into_response(),
        None => Redirect::to("/two-step/login").into_response(),
    }
}

fn two_step_password_page(username: &str, error: Option<&str>) -> Html<String> {
    page(
        "Sign in",
        &format!(
            r#"<h1>Enter your password</h1>
<p>Signing in as <strong id="who">{user}</strong></p>
{error}
<form method="post" action="/two-step/login/password">
  <label for="password">Password</label>
  <input id="password" name="password" type="password" autocomplete="current-password">
  <div><button type="submit">Sign in</button></div>
</form>"#,
            user = escape(username),
            error = html::error_line(error),
        ),
    )
}

async fn two_step_password_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(c): Form<Credentials>,
) -> Response {
    let Some(username) = state.mock.pending_user(&headers, "two-step") else {
        return Redirect::to("/two-step/login").into_response();
    };
    if username == USERNAME && c.password == PASSWORD {
        sign_in(&state, "two-step", &username)
    } else {
        two_step_password_page(&username, Some(BAD_LOGIN)).into_response()
    }
}

// ----- /mfa -----

async fn mfa_form() -> Html<String> {
    login_page("/mfa/login", None)
}

async fn mfa_submit(State(state): State<AppState>, Form(c): Form<Credentials>) -> Response {
    if credentials_ok(&c) {
        let token = state.mock.start_pending("mfa", &c.username);
        redirect_with_cookie("/mfa/login/verify", PENDING_COOKIE, &token)
    } else {
        login_page("/mfa/login", Some(BAD_LOGIN)).into_response()
    }
}

/// The code page. Both detection rules from the generic recipe match it:
/// the `one-time-code` field and the text "verification code".
fn mfa_code_page(error: Option<&str>) -> Html<String> {
    page(
        "Verify",
        &format!(
            r#"<h1>Two-step verification</h1>
<p>Enter the 6-digit verification code from your authenticator app.</p>
{error}
<form method="post" action="/mfa/login/verify">
  <label for="code">Code</label>
  <input id="code" name="code" type="text" inputmode="numeric" autocomplete="one-time-code" maxlength="6">
  <div><button type="submit">Verify</button></div>
</form>"#,
            error = html::error_line(error),
        ),
    )
}

async fn mfa_code_form(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.mock.pending_user(&headers, "mfa") {
        Some(_) => mfa_code_page(None).into_response(),
        None => Redirect::to("/mfa/login").into_response(),
    }
}

#[derive(Deserialize)]
struct CodeForm {
    #[serde(default)]
    code: String,
}

async fn mfa_code_submit(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<CodeForm>) -> Response {
    let Some(username) = state.mock.pending_user(&headers, "mfa") else {
        return Redirect::to("/mfa/login").into_response();
    };
    if f.code.trim() == MFA_CODE {
        sign_in(&state, "mfa", &username)
    } else {
        mfa_code_page(Some("That code didn't work. Try again.")).into_response()
    }
}

// ----- /spa -----

/// One HTML shell for both `/spa/login` and `/spa/home`, like a
/// single-page app. The script keeps its own copy of what was typed and
/// updates it only on `input` events, as React does; on success it
/// switches the URL to `/spa/home` with `history.pushState`, without a
/// page load. The form appears after 300 ms, like an app starting up.
fn spa_page(user: Option<&str>) -> Html<String> {
    let user_attr = user.map(|u| format!(r#" data-user="{}""#, escape(u))).unwrap_or_default();
    page(
        "Console",
        &format!(
            r#"<div id="app"{user_attr}><p>Loading…</p></div>
<script>
  const app = document.getElementById('app');
  const esc = (s) => s.replace(/[&<>"']/g, (c) => ({{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}})[c]);

  function renderHome(user) {{
    app.innerHTML = `<h1 id="welcome">Welcome, ${{esc(user)}}</h1>
      <p>You are signed in to the <code>spa</code> mock console.</p><p><a href="/">All scenarios</a></p>`;
  }}

  function renderLogin(error) {{
    const state = {{ username: '', password: '' }};
    app.innerHTML = `<h1>Sign in</h1>
      ${{error ? `<p id="error" role="alert">${{esc(error)}}</p>` : ''}}
      <form id="login">
        <label for="username">Username</label>
        <input id="username" type="text" autocomplete="username">
        <label for="password">Password</label>
        <input id="password" type="password" autocomplete="current-password">
        <div><button type="submit">Sign in</button></div>
      </form>`;
    for (const field of ['username', 'password']) {{
      document.getElementById(field).addEventListener('input', (e) => {{ state[field] = e.target.value; }});
    }}
    document.getElementById('login').addEventListener('submit', async (e) => {{
      e.preventDefault();
      const response = await fetch('/spa/api/login', {{
        method: 'POST',
        headers: {{ 'Content-Type': 'application/json' }},
        body: JSON.stringify(state),
      }});
      if (response.ok) {{
        const body = await response.json();
        history.pushState({{}}, '', '/spa/home');
        renderHome(body.username);
      }} else {{
        renderLogin('{BAD_LOGIN}');
      }}
    }});
  }}

  setTimeout(() => {{
    if (location.pathname === '/spa/home' && app.dataset.user) renderHome(app.dataset.user);
    else renderLogin();
  }}, 300);
</script>"#
        ),
    )
}

async fn spa_shell() -> Html<String> {
    spa_page(None)
}

async fn spa_home(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.mock.session_user(&headers, "spa") {
        Some(user) => spa_page(Some(&user)).into_response(),
        None => Redirect::to("/spa/login").into_response(),
    }
}

async fn spa_api_login(State(state): State<AppState>, Json(c): Json<Credentials>) -> Response {
    if !credentials_ok(&c) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "ok": false }))).into_response();
    }
    let token = state.mock.start_session("spa", &c.username);
    let mut response = Json(serde_json::json!({ "ok": true, "username": c.username })).into_response();
    if let Ok(cookie) = HeaderValue::from_str(&set_cookie(SESSION_COOKIE, &token)) {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

// ----- /redirect-away -----

/// Sends the browser to the same path on the lookalike port. A real attack
/// would use a lookalike host; a different port is already a different
/// origin, which is what NetLaunch's origin check compares.
async fn redirect_away(State(state): State<AppState>) -> Redirect {
    Redirect::temporary(&format!("{}/redirect-away/login", state.site.lookalike_origin))
}

// ----- /no-success -----

async fn no_success_form() -> Html<String> {
    login_page("/no-success/login", None)
}

async fn no_success_submit(State(state): State<AppState>, Form(c): Form<Credentials>) -> Response {
    if credentials_ok(&c) {
        let token = state.mock.start_session("no-success", &c.username);
        redirect_with_cookie("/no-success/login/signed-in", SESSION_COOKIE, &token)
    } else {
        login_page("/no-success/login", Some(BAD_LOGIN)).into_response()
    }
}

/// Signed in, but the URL still contains `/login` and there is no
/// `#welcome`, so no success rule ever matches.
async fn no_success_signed_in(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.mock.session_user(&headers, "no-success") {
        Some(_) => page(
            "Console",
            r#"<h1>Console</h1><p>You are signed in, but this page never shows the success marker.</p>"#,
        )
        .into_response(),
        None => Redirect::to("/no-success/login").into_response(),
    }
}

// ----- dashboards -----

async fn home(State(state): State<AppState>, Path(scenario): Path<String>, headers: HeaderMap) -> Response {
    // Only the form-based scenarios have a /home; /spa has its own route.
    if !matches!(scenario.as_str(), "simple" | "two-step" | "mfa" | "slow") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match state.mock.session_user(&headers, &scenario) {
        Some(user) => html::welcome(&scenario, &user).into_response(),
        None => Redirect::to(&format!("/{scenario}/login")).into_response(),
    }
}

// ----- /password-only -----

/// A device that only asks for a password, like a Ruijie access point.
/// The button has no `type=submit` attribute, and the address is the same
/// before and after login: signed in, this URL shows the dashboard.
fn password_only_form(error: Option<&str>) -> Html<String> {
    page(
        "Sign in",
        &format!(
            r#"<h1>Access point</h1>
{error}
<form method="post" action="/password-only/login">
  <label for="password">Password</label>
  <input id="password" name="password" type="password">
  <div><button>Log In</button></div>
</form>"#,
            error = html::error_line(error),
        ),
    )
}

async fn password_only_page(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    match state.mock.session_user(&headers, "password-only") {
        Some(user) => html::welcome("password-only", &user),
        None => password_only_form(None),
    }
}

async fn password_only_submit(State(state): State<AppState>, Form(c): Form<Credentials>) -> Response {
    if c.password == PASSWORD {
        let token = state.mock.start_session("password-only", "admin");
        redirect_with_cookie("/password-only/login", SESSION_COOKIE, &token)
    } else {
        password_only_form(Some("Incorrect password")).into_response()
    }
}
