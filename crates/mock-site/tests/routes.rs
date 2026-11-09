//! Route tests for every scenario. They call the routers directly
//! (`tower::ServiceExt::oneshot`), without TLS or a browser, so they check
//! the server side: redirects, cookies, error messages and markers. The
//! browser side (scripts, the SPA, the slow form) is checked by hand in
//! Edge and, from M5, by the engine's integration tests.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use netlaunch_mock_site::{MFA_CODE, MockState, PASSWORD, SCENARIOS, SiteConfig, USERNAME, lookalike_router, main_router};
use tower::ServiceExt;

const LOOKALIKE: &str = "https://localhost:9444";

fn site() -> (Router, MockState) {
    let state = MockState::default();
    let config = SiteConfig { lookalike_origin: LOOKALIKE.to_string(), slow_delay_ms: 5000 };
    (main_router(state.clone(), config), state)
}

struct Reply {
    status: StatusCode,
    location: Option<String>,
    /// `name=value` of the first Set-Cookie, ready to send back.
    cookie: Option<String>,
    body: String,
}

async fn send(app: &Router, method: &str, uri: &str, cookie: Option<&str>, form: Option<&str>) -> Reply {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let body = match form {
        Some(form) => {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
            Body::from(form.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let location = response.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_string());
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
    let body = String::from_utf8(response.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    Reply { status, location, cookie, body }
}

async fn get(app: &Router, uri: &str, cookie: Option<&str>) -> Reply {
    send(app, "GET", uri, cookie, None).await
}

async fn post(app: &Router, uri: &str, cookie: Option<&str>, form: &str) -> Reply {
    send(app, "POST", uri, cookie, Some(form)).await
}

fn good_login() -> String {
    format!("username={USERNAME}&password={PASSWORD}")
}

fn bad_login() -> String {
    format!("username={USERNAME}&password=wrong")
}

#[tokio::test]
async fn index_lists_every_scenario() {
    let (app, _) = site();
    let page = get(&app, "/", None).await;
    assert_eq!(page.status, StatusCode::OK);
    for (path, _, _) in SCENARIOS {
        assert!(page.body.contains(&format!(r#"href="/{path}/login""#)), "missing /{path}");
    }
    assert_eq!(SCENARIOS.len(), 8);
}

#[tokio::test]
async fn simple_logs_in_and_rejects_a_wrong_password() {
    let (app, _) = site();
    let form = get(&app, "/simple/login", None).await;
    assert!(form.body.contains(r#"type="password""#));
    assert!(form.body.contains(r#"autocomplete="username""#));
    assert!(form.body.contains(r#"type="submit""#));

    let ok = post(&app, "/simple/login", None, &good_login()).await;
    assert_eq!(ok.status, StatusCode::SEE_OTHER);
    assert_eq!(ok.location.as_deref(), Some("/simple/home"));
    let home = get(&app, "/simple/home", ok.cookie.as_deref()).await;
    assert!(home.body.contains(r#"id="welcome""#));
    assert!(home.body.contains(USERNAME));

    let bad = post(&app, "/simple/login", None, &bad_login()).await;
    assert_eq!(bad.status, StatusCode::OK);
    assert!(bad.body.contains(r#"role="alert""#));
}

#[tokio::test]
async fn dashboards_need_a_session_for_that_scenario() {
    let (app, _) = site();
    let no_cookie = get(&app, "/simple/home", None).await;
    assert_eq!(no_cookie.location.as_deref(), Some("/simple/login"));

    // A session from one scenario doesn't open another one's dashboard.
    let ok = post(&app, "/simple/login", None, &good_login()).await;
    let other = get(&app, "/slow/home", ok.cookie.as_deref()).await;
    assert_eq!(other.location.as_deref(), Some("/slow/login"));

    assert_eq!(get(&app, "/nope/home", None).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn two_step_asks_for_the_password_on_a_second_page() {
    let (app, _) = site();
    let step1 = get(&app, "/two-step/login", None).await;
    assert!(step1.body.contains(r#"id="next""#));
    assert!(!step1.body.contains(r#"type="password""#));

    let next = post(&app, "/two-step/login", None, &format!("username={USERNAME}")).await;
    assert_eq!(next.location.as_deref(), Some("/two-step/login/password"));
    let pending = next.cookie.unwrap();

    let step2 = get(&app, "/two-step/login/password", Some(&pending)).await;
    assert!(step2.body.contains(r#"type="password""#));

    let wrong = post(&app, "/two-step/login/password", Some(&pending), "password=wrong").await;
    assert!(wrong.body.contains(r#"role="alert""#));

    let ok = post(&app, "/two-step/login/password", Some(&pending), &format!("password={PASSWORD}")).await;
    assert_eq!(ok.location.as_deref(), Some("/two-step/home"));
    let home = get(&app, "/two-step/home", ok.cookie.as_deref()).await;
    assert!(home.body.contains(r#"id="welcome""#));

    // Without the pending cookie the password page sends you back.
    let lost = get(&app, "/two-step/login/password", None).await;
    assert_eq!(lost.location.as_deref(), Some("/two-step/login"));
}

#[tokio::test]
async fn two_step_escapes_the_username_it_shows() {
    let (app, _) = site();
    let next = post(&app, "/two-step/login", None, "username=%3Cscript%3Ex%3C%2Fscript%3E").await;
    let page = get(&app, "/two-step/login/password", next.cookie.as_deref()).await;
    assert!(page.body.contains("&lt;script&gt;x&lt;/script&gt;"));
    assert!(!page.body.contains("<script>x</script>"));
}

#[tokio::test]
async fn mfa_stops_at_a_code_page_that_matches_the_mfa_rules() {
    let (app, _) = site();
    let login = post(&app, "/mfa/login", None, &good_login()).await;
    assert_eq!(login.location.as_deref(), Some("/mfa/login/verify"));
    let pending = login.cookie.unwrap();

    let verify = get(&app, "/mfa/login/verify", Some(&pending)).await;
    // The generic recipe's two mfa rules.
    assert!(verify.body.contains(r#"autocomplete="one-time-code""#));
    assert!(verify.body.contains("verification code"));

    let wrong = post(&app, "/mfa/login/verify", Some(&pending), "code=000000").await;
    assert!(wrong.body.contains(r#"role="alert""#));

    let ok = post(&app, "/mfa/login/verify", Some(&pending), &format!("code={MFA_CODE}")).await;
    assert_eq!(ok.location.as_deref(), Some("/mfa/home"));
    let home = get(&app, "/mfa/home", ok.cookie.as_deref()).await;
    assert!(home.body.contains(r#"id="welcome""#));
}

#[tokio::test]
async fn mfa_rejects_a_bad_password_before_the_code_page() {
    let (app, _) = site();
    let bad = post(&app, "/mfa/login", None, &bad_login()).await;
    assert_eq!(bad.status, StatusCode::OK);
    assert!(bad.body.contains(r#"role="alert""#));
    assert_eq!(get(&app, "/mfa/login/verify", None).await.location.as_deref(), Some("/mfa/login"));
}

#[tokio::test]
async fn slow_adds_the_form_by_script_after_the_delay() {
    let (app, _) = site();
    let page = get(&app, "/slow/login", None).await;
    // The form is inside a <template>, so it isn't on the page until the
    // script runs.
    assert!(page.body.contains(r#"<template id="form-template">"#));
    assert!(page.body.contains("}, 5000);"));

    let quick = get(&app, "/slow/login?delay=250", None).await;
    assert!(quick.body.contains("}, 250);"));
    let capped = get(&app, "/slow/login?delay=999999", None).await;
    assert!(capped.body.contains("}, 60000);"));

    let ok = post(&app, "/slow/login", None, &good_login()).await;
    assert_eq!(ok.location.as_deref(), Some("/slow/home"));
}

#[tokio::test]
async fn wrong_password_rejects_even_the_right_password() {
    let (app, _) = site();
    let reply = post(&app, "/wrong-password/login", None, &good_login()).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.cookie.is_none());
    assert!(reply.body.contains("Invalid username or password"));
}

#[tokio::test]
async fn redirect_away_sends_the_browser_to_the_lookalike_origin() {
    let (app, _) = site();
    let reply = get(&app, "/redirect-away/login", None).await;
    assert!(reply.status.is_redirection());
    assert_eq!(reply.location.as_deref(), Some("https://localhost:9444/redirect-away/login"));
}

#[tokio::test]
async fn no_success_signs_in_but_never_leaves_login_or_shows_welcome() {
    let (app, _) = site();
    let ok = post(&app, "/no-success/login", None, &good_login()).await;
    let location = ok.location.unwrap();
    assert!(location.contains("/login"));
    let page = get(&app, &location, ok.cookie.as_deref()).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("signed in"));
    assert!(!page.body.contains(r#"id="welcome""#));
}

#[tokio::test]
async fn spa_logs_in_through_its_json_endpoint() {
    let (app, _) = site();
    let shell = get(&app, "/spa/login", None).await;
    assert!(shell.body.contains("history.pushState"));

    let send_json = |body: String| {
        let app = app.clone();
        async move {
            let request = Request::post("/spa/api/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap();
            app.oneshot(request).await.unwrap()
        }
    };
    let bad = send_json(format!(r#"{{"username":"{USERNAME}","password":"wrong"}}"#)).await;
    assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);

    let ok = send_json(format!(r#"{{"username":"{USERNAME}","password":"{PASSWORD}"}}"#)).await;
    assert_eq!(ok.status(), StatusCode::OK);
    let cookie = ok.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_string();

    let home = get(&app, "/spa/home", Some(&cookie)).await;
    assert!(home.body.contains(&format!(r#"data-user="{USERNAME}""#)));
    assert_eq!(get(&app, "/spa/home", None).await.location.as_deref(), Some("/spa/login"));
}

#[tokio::test]
async fn lookalike_counts_input_and_submits_but_never_echoes_them() {
    let state = MockState::default();
    let lookalike = lookalike_router(state.clone());

    let form = get(&lookalike, "/redirect-away/login", None).await;
    assert!(form.body.contains(r#"type="password""#));
    assert!(form.body.contains("sendBeacon('/_mock/input')"));

    post(&lookalike, "/_mock/input", None, "").await;
    post(&lookalike, "/_mock/input", None, "").await;
    let submitted = post(&lookalike, "/redirect-away/login", None, &good_login()).await;
    assert!(!submitted.body.contains(PASSWORD));

    let hits = state.lookalike_hits();
    assert_eq!((hits.input_events, hits.submits), (2, 1));

    // The main site reports and resets the same counters.
    let main = main_router(state.clone(), SiteConfig { lookalike_origin: LOOKALIKE.into(), slow_delay_ms: 0 });
    let status = get(&main, "/_mock/status", None).await;
    assert_eq!(status.body, r#"{"input_events":2,"submits":1}"#);
    post(&main, "/_mock/reset", None, "").await;
    assert_eq!(state.lookalike_hits().submits, 0);
}
