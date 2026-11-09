//! Mock admin-console login site for NetLaunch (milestone M2).
//!
//! Two HTTPS servers on 127.0.0.1, both with a self-signed certificate
//! generated at start-up:
//!
//! - the **main site** (default port 8443) with one login scenario per
//!   path prefix, listed on its index page;
//! - the **lookalike site** (default port 8444), where `/redirect-away`
//!   sends the browser. It looks like a login page and counts any input it
//!   receives, so a test can prove NetLaunch filled nothing there.
//!
//! `serve` is a library function so the M5 engine tests can start the site
//! in-process on free ports; `main.rs` is only argument parsing around it.
//!
//! The credentials are fake and printed on the index page on purpose.

mod html;
mod lookalike;
mod scenarios;

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::http::HeaderMap;
use axum_server::Handle;
use axum_server::tls_rustls::RustlsConfig;

pub use scenarios::SCENARIOS;

/// The one account every scenario accepts.
pub const USERNAME: &str = "mock-admin";
pub const PASSWORD: &str = "Mock-Pa55-2026";
/// The code the `/mfa` scenario accepts. Tests type it through the driver;
/// the engine itself never does.
pub const MFA_CODE: &str = "123456";

/// How to run the mock site. Port 0 picks a free port.
#[derive(Debug, Clone)]
pub struct MockConfig {
    pub port: u16,
    pub lookalike_port: u16,
    /// How long `/slow` waits before showing its form, unless the URL
    /// overrides it with `?delay=<ms>`.
    pub slow_delay_ms: u64,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self { port: 8443, lookalike_port: 8444, slow_delay_ms: 5000 }
    }
}

/// What the routers need to know about each other.
#[derive(Debug, Clone)]
pub struct SiteConfig {
    /// `https://localhost:<lookalike port>`, where `/redirect-away` points.
    pub lookalike_origin: String,
    pub slow_delay_ms: u64,
}

/// State shared by both servers: sessions, half-finished logins, and the
/// lookalike site's counters.
#[derive(Clone, Default)]
pub struct MockState {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Session cookie value -> (scenario, username).
    sessions: Mutex<HashMap<String, (String, String)>>,
    /// Cookie value for a login that is between steps -> username.
    /// Used by `/two-step` (username entered) and `/mfa` (password ok).
    pending: Mutex<HashMap<String, (String, String)>>,
    lookalike_inputs: AtomicUsize,
    lookalike_submits: AtomicUsize,
}

/// The lookalike site's counters. Both must stay 0 after NetLaunch runs
/// the `/redirect-away` scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LookalikeHits {
    /// `input` events on the lookalike form's fields (one per field).
    pub input_events: usize,
    /// Form submissions to the lookalike site.
    pub submits: usize,
}

impl MockState {
    pub fn lookalike_hits(&self) -> LookalikeHits {
        LookalikeHits {
            input_events: self.inner.lookalike_inputs.load(Ordering::SeqCst),
            submits: self.inner.lookalike_submits.load(Ordering::SeqCst),
        }
    }

    pub fn reset_lookalike_hits(&self) {
        self.inner.lookalike_inputs.store(0, Ordering::SeqCst);
        self.inner.lookalike_submits.store(0, Ordering::SeqCst);
    }

    fn start_session(&self, scenario: &str, username: &str) -> String {
        let token = new_token();
        self.inner.sessions.lock().unwrap().insert(token.clone(), (scenario.into(), username.into()));
        token
    }

    /// The username of a session for `scenario`, if the request carries one.
    fn session_user(&self, headers: &HeaderMap, scenario: &str) -> Option<String> {
        let token = cookie(headers, SESSION_COOKIE)?;
        let sessions = self.inner.sessions.lock().unwrap();
        match sessions.get(&token) {
            Some((s, user)) if s == scenario => Some(user.clone()),
            _ => None,
        }
    }

    fn start_pending(&self, scenario: &str, username: &str) -> String {
        let token = new_token();
        self.inner.pending.lock().unwrap().insert(token.clone(), (scenario.into(), username.into()));
        token
    }

    fn pending_user(&self, headers: &HeaderMap, scenario: &str) -> Option<String> {
        let token = cookie(headers, PENDING_COOKIE)?;
        let pending = self.inner.pending.lock().unwrap();
        match pending.get(&token) {
            Some((s, user)) if s == scenario => Some(user.clone()),
            _ => None,
        }
    }
}

const SESSION_COOKIE: &str = "mock_session";
const PENDING_COOKIE: &str = "mock_pending";

/// A `Set-Cookie` value for this site. `Secure` because the site is HTTPS;
/// the router tests send cookies as plain headers, so it does not get in
/// their way.
fn set_cookie(name: &str, value: &str) -> String {
    format!("{name}={value}; Path=/; HttpOnly; Secure; SameSite=Lax")
}

/// Reads one cookie from the request's `Cookie` header(s).
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
}

/// A hard-to-guess cookie value. Not cryptographic, and it doesn't need to
/// be: this is a local mock with a published password. `RandomState` is
/// seeded randomly per process, which is enough to keep values unique.
fn new_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut a = std::collections::hash_map::RandomState::new().build_hasher();
    a.write_u64(n);
    a.write_u128(nanos);
    let b = std::collections::hash_map::RandomState::new().hash_one((n, nanos));
    format!("{:016x}{:016x}", a.finish(), b)
}

/// The main site's routes.
pub fn main_router(state: MockState, config: SiteConfig) -> Router {
    scenarios::router(state, config)
}

/// The lookalike site's routes.
pub fn lookalike_router(state: MockState) -> Router {
    lookalike::router(state)
}

/// Both servers, running. Dropping this does not stop them; call
/// `shutdown`.
pub struct RunningMock {
    /// `https://localhost:<port>`
    pub base_url: String,
    /// `https://localhost:<lookalike port>`
    pub lookalike_url: String,
    pub state: MockState,
    /// The self-signed certificate both servers present (DER).
    pub certificate_der: Vec<u8>,
    handles: Vec<Handle<SocketAddr>>,
}

impl RunningMock {
    pub fn shutdown(&self) {
        for handle in &self.handles {
            handle.shutdown();
        }
    }
}

/// Starts both servers on 127.0.0.1 and returns once they are listening.
pub async fn serve(config: MockConfig) -> std::io::Result<RunningMock> {
    // Bind first, so port 0 resolves to a real port before the routers
    // need the lookalike origin.
    let main_listener = TcpListener::bind(("127.0.0.1", config.port))?;
    let lookalike_listener = TcpListener::bind(("127.0.0.1", config.lookalike_port))?;
    main_listener.set_nonblocking(true)?;
    lookalike_listener.set_nonblocking(true)?;
    let main_port = main_listener.local_addr()?.port();
    let lookalike_port = lookalike_listener.local_addr()?.port();

    let (tls, certificate_der) = tls_config()?;
    let state = MockState::default();
    let site = SiteConfig {
        lookalike_origin: format!("https://localhost:{lookalike_port}"),
        slow_delay_ms: config.slow_delay_ms,
    };

    let mut handles = Vec::new();
    for (listener, router) in [
        (main_listener, main_router(state.clone(), site.clone())),
        (lookalike_listener, lookalike_router(state.clone())),
    ] {
        let handle = Handle::new();
        let server = axum_server::from_tcp_rustls(listener, tls.clone())?.handle(handle.clone());
        tokio::spawn(async move {
            if let Err(e) = server.serve(router.into_make_service()).await {
                eprintln!("mock-site: server stopped: {e}");
            }
        });
        handle.listening().await;
        handles.push(handle);
    }

    Ok(RunningMock {
        base_url: format!("https://localhost:{main_port}"),
        lookalike_url: site.lookalike_origin,
        state,
        certificate_der,
        handles,
    })
}

/// A fresh self-signed certificate for `localhost` and `127.0.0.1`, and a
/// rustls config that uses the `ring` crypto provider explicitly (rustls
/// would otherwise look for a process-wide default and panic if none).
fn tls_config() -> std::io::Result<(RustlsConfig, Vec<u8>)> {
    let io_err = |e: &dyn std::fmt::Display| std::io::Error::other(e.to_string());

    // A readable name, so Edge's certificate warning says what this is.
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
        .map_err(|e| io_err(&e))?;
    params.distinguished_name.push(rcgen::DnType::CommonName, "NetLaunch mock site");
    let key_pair = rcgen::KeyPair::generate().map_err(|e| io_err(&e))?;
    let cert = params.self_signed(&key_pair).map_err(|e| io_err(&e))?;
    let cert_der = cert.der().clone();
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(key_pair.serialize_der().into());

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut server_config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| io_err(&e))?
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .map_err(|e| io_err(&e))?;
    // The routers are HTTP/1.1 only (axum's `http2` feature is off).
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok((RustlsConfig::from_config(Arc::new(server_config)), cert_der.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cookies_from_one_or_more_headers() {
        let mut headers = HeaderMap::new();
        headers.append("cookie", "a=1; mock_session=abc".parse().unwrap());
        headers.append("cookie", "b=2".parse().unwrap());
        assert_eq!(cookie(&headers, "mock_session").as_deref(), Some("abc"));
        assert_eq!(cookie(&headers, "b").as_deref(), Some("2"));
        assert_eq!(cookie(&headers, "missing"), None);
    }

    #[test]
    fn tokens_are_unique() {
        let tokens: std::collections::HashSet<_> = (0..1000).map(|_| new_token()).collect();
        assert_eq!(tokens.len(), 1000);
    }

    #[tokio::test]
    async fn serves_on_free_ports() {
        let mock = serve(MockConfig { port: 0, lookalike_port: 0, slow_delay_ms: 0 }).await.unwrap();
        assert!(mock.base_url.starts_with("https://localhost:"));
        assert_ne!(mock.base_url, mock.lookalike_url);
        assert!(!mock.certificate_der.is_empty());
        mock.shutdown();
    }
}
