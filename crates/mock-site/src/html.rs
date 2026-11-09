//! HTML building blocks shared by the scenarios. Plain `format!` strings:
//! the pages are small, and no template engine means no extra dependency.
//!
//! Anything that came from a request (a username) goes through `escape`
//! before it is put in a page.

use axum::response::Html;

/// Escapes text for use inside HTML elements and quoted attributes.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Wraps a body in the shared page layout. `title` and `body` are trusted
/// (written by this crate); callers escape request data first.
pub fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · NetLaunch mock</title>
<style>
  body {{ font-family: system-ui, sans-serif; background: #eef1f5; color: #1d2733; margin: 0; }}
  header {{ background: #24364b; color: #fff; padding: 12px 20px; font-weight: 600; }}
  main {{ max-width: 760px; margin: 32px auto; background: #fff; padding: 24px 28px; border-radius: 8px; box-shadow: 0 1px 3px rgba(0,0,0,.15); }}
  label {{ display: block; margin: 12px 0 4px; }}
  input {{ font: inherit; padding: 6px 8px; width: 260px; }}
  button {{ font: inherit; margin-top: 16px; padding: 6px 16px; }}
  #error {{ color: #a4161a; }}
  table {{ border-collapse: collapse; width: 100%; }}
  td, th {{ text-align: left; padding: 6px 8px; border-bottom: 1px solid #dde3ea; vertical-align: top; }}
  code {{ background: #eef1f5; padding: 1px 4px; border-radius: 3px; }}
</style>
</head>
<body>
<header>NetLaunch mock console</header>
<main>
{body}
</main>
</body>
</html>"#
    ))
}

/// The error line every scenario shows after a rejected attempt. A recipe
/// can detect failure with the `#error` selector or the text.
pub fn error_line(message: Option<&str>) -> String {
    match message {
        Some(text) => format!(r#"<p id="error" role="alert">{}</p>"#, escape(text)),
        None => String::new(),
    }
}

/// A username + password form that the `generic-form` recipe can fill:
/// the username field is `type=text` with `autocomplete=username`, the
/// password field is `type=password`, the button is `type=submit`.
pub fn login_form(action: &str, error: Option<&str>) -> String {
    format!(
        r#"<h1>Sign in</h1>
{error}
<form method="post" action="{action}">
  <label for="username">Username</label>
  <input id="username" name="username" type="text" autocomplete="username">
  <label for="password">Password</label>
  <input id="password" name="password" type="password" autocomplete="current-password">
  <div><button type="submit">Sign in</button></div>
</form>"#,
        error = error_line(error),
    )
}

/// The page shown after a successful login. `#welcome` is the success
/// marker a recipe can wait for; the URL has no `/login` in it.
pub fn welcome(scenario: &str, username: &str) -> Html<String> {
    page(
        "Dashboard",
        &format!(
            r#"<h1 id="welcome">Welcome, {user}</h1>
<p>You are signed in to the <code>{scenario}</code> mock console.</p>
<p><a href="/">All scenarios</a></p>"#,
            user = escape(username),
            scenario = escape(scenario),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_and_quotes() {
        assert_eq!(escape(r#"<b a="1">'x' & y</b>"#), "&lt;b a=&quot;1&quot;&gt;&#39;x&#39; &amp; y&lt;/b&gt;");
    }
}
