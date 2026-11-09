//! The lookalike site: where `/redirect-away` sends the browser.
//!
//! It shows a copy of the login form on another port (so another origin).
//! NetLaunch's origin check must stop before typing anything here. To make
//! that testable, the page reports each field's first `input` event, and
//! the server counts form submissions. It counts them and nothing else:
//! whatever was typed is never read, stored or shown.

use std::sync::atomic::Ordering;

use axum::Router;
use axum::extract::State;
use axum::http::Method;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::post;

use crate::MockState;
use crate::html::{login_form, page};

pub fn router(state: MockState) -> Router {
    Router::new()
        .route("/_mock/input", post(input_event))
        .fallback(any_page)
        .with_state(state)
}

/// Called by the page's script (with no body) the first time each field
/// receives input.
async fn input_event(State(state): State<MockState>) -> &'static str {
    state.inner.lookalike_inputs.fetch_add(1, Ordering::SeqCst);
    "counted"
}

/// GET anything: the lookalike form. POST anything: count a submit, then a
/// page saying what happened. The request body is never read.
async fn any_page(State(state): State<MockState>, method: Method) -> Response {
    if method == Method::POST {
        state.inner.lookalike_submits.fetch_add(1, Ordering::SeqCst);
        return page(
            "Lookalike",
            r#"<h1 id="lookalike-submitted">This was the lookalike site</h1>
<p>A form was submitted here. NetLaunch should never fill or submit this page.</p>"#,
        )
        .into_response();
    }
    lookalike_page().into_response()
}

fn lookalike_page() -> Html<String> {
    // The form posts back to this site, so a submit is counted above.
    let form = login_form("/redirect-away/login", None);
    let Html(mut body) = page(
        "Sign in",
        &format!(
            r#"{form}
<script>
  for (const field of document.querySelectorAll('input')) {{
    field.addEventListener('input', () => navigator.sendBeacon('/_mock/input'), {{ once: true }});
  }}
</script>"#
        ),
    );
    // A visible hint for someone testing by hand; NetLaunch never reads it.
    body = body.replace(
        "<header>NetLaunch mock console</header>",
        "<header>NetLaunch mock console <small>(lookalike, different port)</small></header>",
    );
    Html(body)
}
