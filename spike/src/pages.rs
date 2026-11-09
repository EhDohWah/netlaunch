//! A tiny HTTP server for the spike's two test forms, on 127.0.0.1 with a
//! random port. Plain std, one thread per connection: browsers open spare
//! connections they may never send on, so handling connections one at a
//! time would stall real requests behind an idle one. The real mock site
//! (M2) is axum.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// The fake account the forms accept. Not a real secret; the log check
/// searches for this exact string.
pub const USERNAME: &str = "spike-user";
pub const PASSWORD: &str = "Spike-Pa55-7f3c";

const PLAIN_HTML: &str = include_str!("../pages/plain.html");
const CONTROLLED_HTML: &str = include_str!("../pages/controlled.html");

/// Starts the server on a background thread and returns its base URL.
pub fn serve() -> std::io::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}", listener.local_addr()?);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let _ = handle(stream);
            });
        }
    });
    Ok(base)
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
    }
    let mut body = vec![0u8; content_length.min(64 * 1024)];
    reader.read_exact(&mut body)?;
    let body = String::from_utf8_lossy(&body);

    let (status, content_type, payload) = match (method.as_str(), path.as_str()) {
        ("GET", "/plain") => ("200 OK", "text/html", PLAIN_HTML.to_string()),
        ("GET", "/controlled") => ("200 OK", "text/html", CONTROLLED_HTML.to_string()),
        // Classic form post: the browser navigates to the result page.
        ("POST", "/plain/login") => {
            let ok = credentials_ok(&body);
            ("200 OK", "text/html", result_page(ok))
        }
        // fetch() from the controlled page: JSON back, page renders it.
        ("POST", "/controlled/login") => {
            let ok = credentials_ok(&body);
            ("200 OK", "application/json", format!("{{\"ok\":{ok}}}"))
        }
        _ => ("404 Not Found", "text/plain", "not found".to_string()),
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(response.as_bytes())
}

fn credentials_ok(form_body: &str) -> bool {
    let mut username = None;
    let mut password = None;
    for pair in form_body.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "username" => username = Some(url_decode(value)),
            "password" => password = Some(url_decode(value)),
            _ => {}
        }
    }
    username.as_deref() == Some(USERNAME) && password.as_deref() == Some(PASSWORD)
}

fn result_page(ok: bool) -> String {
    let message = if ok {
        format!("Logged in as {USERNAME}")
    } else {
        "Wrong username or password".to_string()
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Result</title></head>\
         <body><p id=\"result\" data-ok=\"{ok}\">{message}</p></body></html>"
    )
}

/// Decodes `application/x-www-form-urlencoded` values.
fn url_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let decoded = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                match decoded {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_form_values() {
        assert_eq!(url_decode("a+b%21%2Fc"), "a b!/c");
        assert_eq!(url_decode("bad%zz"), "bad%zz");
        assert_eq!(url_decode("end%2"), "end%2");
    }

    #[test]
    fn accepts_only_the_spike_account() {
        assert!(credentials_ok(&format!("username={USERNAME}&password={PASSWORD}")));
        assert!(!credentials_ok(&format!("username={USERNAME}&password=wrong")));
        assert!(!credentials_ok(&format!("password={PASSWORD}")));
    }
}
