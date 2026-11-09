//! One site session: an Edge process NetLaunch started, the Playwright
//! connection to it, and the tab the login runs in.
//!
//! The steps follow "How the browser runs" in docs/SPEC.md:
//! launch Edge with its own profile and `--remote-debugging-port=0`, read
//! the port, attach with `connect_over_cdp`, and use the default context's
//! `about:blank` tab. Then either release the window (disconnect, Edge
//! keeps running) or close it (CDP `Browser.close`, kill after 10 s).

use std::path::PathBuf;
use std::process::Child;
use std::sync::Mutex;
use std::time::Duration;

use netlaunch_engine::{BrowserDriver, DriverError, DriverResult};
use playwright_rs::protocol::{GotoOptions, WaitForOptions, WaitForState};
use playwright_rs::{Browser, Page, Playwright};
use secrecy::{ExposeSecret, SecretString};

use crate::edge;

/// How long actions (fill, click...) wait for their element.
const ACTION_TIMEOUT_MS: f64 = 15_000.0;
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);
const PORT_TIMEOUT: Duration = Duration::from_secs(20);
/// Managed close: how long Edge gets to quit cleanly before it's killed.
const CLOSE_GRACE: Duration = Duration::from_secs(10);

/// Which profile folder the window uses (spec decision 6).
#[derive(Debug, Clone)]
pub enum Profile {
    /// A new empty folder, deleted when the window is closed.
    Fresh,
    /// A folder kept between launches, so the site's cookies survive.
    Persistent(PathBuf),
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// `msedge.exe`, usually from [`edge::find_edge`].
    pub browser: PathBuf,
    pub profile: Profile,
    /// Adds `--ignore-certificate-errors`. It applies to the whole Edge
    /// process, which is why every site gets its own process.
    pub accept_self_signed: bool,
    /// Tests only: no visible window.
    pub headless: bool,
    /// Tests only: more switches (for example `--no-sandbox` in a Linux
    /// container).
    pub extra_args: Vec<String>,
}

pub struct Session {
    playwright: Playwright,
    browser: Browser,
    page: Page,
    /// In a Mutex so `&self` methods can check whether Edge is still
    /// running.
    child: Mutex<Child>,
    profile_dir: PathBuf,
    /// Delete the profile folder when the window is closed.
    fresh: bool,
}

impl Session {
    /// Starts Edge and attaches to it. On failure, nothing is left running.
    pub async fn launch(options: LaunchOptions) -> Result<Session, String> {
        let (profile_dir, fresh) = match &options.profile {
            Profile::Fresh => (edge::fresh_profile_dir(), true),
            Profile::Persistent(dir) => (dir.clone(), false),
        };
        std::fs::create_dir_all(&profile_dir)
            .map_err(|e| format!("Couldn't create the profile folder {}: {e}", profile_dir.display()))?;
        edge::seed_preferences(&profile_dir).map_err(|e| format!("Couldn't write the profile's preferences: {e}"))?;

        let mut args = options.extra_args.clone();
        if options.accept_self_signed {
            args.push("--ignore-certificate-errors".into());
        }
        if options.headless {
            args.push("--headless=new".into());
        }
        let mut child = edge::launch(&options.browser, &profile_dir, &args)
            .map_err(|e| format!("Couldn't start {}: {e}", options.browser.display()))?;

        match Self::attach(&profile_dir, &mut child).await {
            Ok((playwright, browser, page)) => Ok(Session {
                playwright,
                browser,
                page,
                child: Mutex::new(child),
                profile_dir,
                fresh,
            }),
            Err(message) => {
                let _ = child.kill();
                let _ = child.wait();
                if fresh {
                    let dir = profile_dir.clone();
                    let _ = tokio::task::spawn_blocking(move || edge::remove_profile(&dir)).await;
                }
                Err(message)
            }
        }
    }

    async fn attach(profile_dir: &std::path::Path, child: &mut Child) -> Result<(Playwright, Browser, Page), String> {
        let port = edge::wait_for_debug_port(profile_dir, child, PORT_TIMEOUT).await?;

        let playwright = Playwright::launch().await.map_err(|e| {
            format!(
                "Couldn't start the Playwright driver: {e}. If PLAYWRIGHT_DRIVER_PATH is set, check it points \
                 at a folder holding node(.exe) and package/cli.js."
            )
        })?;
        let endpoint = format!("http://127.0.0.1:{port}");
        let browser = match playwright.chromium().connect_over_cdp(&endpoint, None).await {
            Ok(browser) => browser,
            Err(e) => {
                let _ = playwright.shutdown().await;
                return Err(format!("Couldn't attach to Edge on {endpoint}: {e}"));
            }
        };

        // The browser's own default context and its about:blank tab. A
        // context from new_context() belongs to Playwright and could close
        // with it, taking the window along. Pick the tab by URL: something
        // else may have opened a tab first.
        let Some(context) = browser.contexts().into_iter().next() else {
            let _ = playwright.shutdown().await;
            return Err("Edge has no default browser context after attaching.".into());
        };
        let page = match context.pages().into_iter().find(|p| p.url() == "about:blank") {
            Some(page) => page,
            None => match context.new_page().await {
                Ok(page) => page,
                Err(e) => {
                    let _ = playwright.shutdown().await;
                    return Err(format!("Couldn't open a tab: {e}"));
                }
            },
        };
        page.set_default_timeout(ACTION_TIMEOUT_MS).await;
        Ok((playwright, browser, page))
    }

    /// The profile folder this window uses.
    pub fn profile_dir(&self) -> &std::path::Path {
        &self.profile_dir
    }

    /// The Edge process id (tests use it to check a released window
    /// survives, and to clean it up afterwards).
    pub fn pid(&self) -> Option<u32> {
        self.child.lock().ok().map(|child| child.id())
    }

    /// True when Edge has signed this window in to an account, which on a
    /// work PC is its implicit sign-in to the Windows account (decision 9:
    /// accept and warn). Reads the profile's preferences, which Edge writes
    /// a few seconds after signing in, so ask again later if it's false.
    pub fn signed_in(&self) -> bool {
        !edge::signed_in_account_paths(&self.profile_dir).is_empty()
    }

    /// True while the Edge process NetLaunch started is running.
    pub fn is_running(&self) -> bool {
        match self.child.lock() {
            Ok(mut child) => matches!(child.try_wait(), Ok(None)),
            Err(_) => false,
        }
    }

    /// Checks the window really is isolated. Edge policies override the
    /// command-line switches, so this looks at the result: extensions
    /// running, or tabs NetLaunch didn't open. Returns one sentence per
    /// problem; empty means isolated. Only origins are reported, never full
    /// URLs.
    pub async fn isolation_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        // Every tab except ours.
        let mut others: Vec<String> = Vec::new();
        let ours = self.page.url();
        let mut skipped_ours = false;
        for context in self.browser.contexts() {
            for page in context.pages() {
                let url = page.url();
                if !skipped_ours && url == ours {
                    skipped_ours = true;
                    continue;
                }
                if url != "about:blank" {
                    others.push(short_origin(&url));
                }
            }
        }
        if !others.is_empty() {
            warnings.push(format!("Other tabs opened in the login window: {}.", others.join(", ")));
        }

        // The ground truth for extensions: what the browser is running
        // right now. An explicit empty filter means "every target".
        let targets = match self.browser.new_browser_cdp_session().await {
            Ok(session) => session.send("Target.getTargets", Some(serde_json::json!({ "filter": [{}] }))).await,
            Err(e) => Err(e),
        };
        if let Ok(targets) = targets {
            let mut running: Vec<String> = Vec::new();
            // playwright-rs wraps the CDP reply as {"result": {...}}.
            let infos = targets.pointer("/result/targetInfos").or_else(|| targets.get("targetInfos"));
            for info in infos.and_then(|v| v.as_array()).into_iter().flatten() {
                let url = info.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(rest) = url.strip_prefix("chrome-extension://") {
                    let id = rest.split('/').next().unwrap_or("").to_string();
                    if !running.contains(&id) {
                        running.push(id);
                    }
                }
            }
            if !running.is_empty() {
                warnings.push(format!(
                    "Extensions are running in the login window ({}), probably required by an Edge policy.",
                    running.join(", ")
                ));
            }
        }
        warnings
    }

    /// Hands the window to the admin: disconnects and stops the Playwright
    /// driver. Edge keeps running on its own. For an attached browser,
    /// playwright-rs's `Browser::close()` only disconnects.
    pub async fn release(self) {
        let _ = self.browser.close().await;
        let _ = self.playwright.shutdown().await;
        tracing::info!("window released");
    }

    /// Closes the window: asks Edge to quit cleanly (CDP `Browser.close`,
    /// which saves a persistent profile), kills it if it's still running
    /// after 10 s, then deletes a fresh profile folder. Returns whether the
    /// quit was clean.
    pub async fn close(self) -> bool {
        let Session { playwright, browser, child, profile_dir, fresh, .. } = self;
        let mut child = child.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Ok(session) = browser.new_browser_cdp_session().await {
            // The connection drops as Edge quits, so an error here is normal.
            let _ = session.send("Browser.close", None).await;
        }
        let (mut child, clean) = tokio::task::spawn_blocking(move || {
            let clean = edge::wait_for_exit(&mut child, CLOSE_GRACE);
            (child, clean)
        })
        .await
        .expect("wait task");
        if !clean {
            tracing::warn!("Edge didn't quit within {} s; killing it", CLOSE_GRACE.as_secs());
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = playwright.shutdown().await;

        if fresh {
            let removed = tokio::task::spawn_blocking(move || edge::remove_profile(&profile_dir)).await;
            if !matches!(removed, Ok(Ok(()))) {
                tracing::warn!("couldn't delete the fresh profile folder");
            }
        }
        tracing::info!(clean, "window closed");
        clean
    }

    fn field(&self, selector: &str) -> playwright_rs::protocol::Locator {
        // The first *visible* match: generic selectors may match several
        // inputs, and hidden decoy fields must be skipped.
        self.page.locator(selector).visible().first()
    }
}

/// `scheme://host` of a URL, for messages.
fn short_origin(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => format!("{scheme}://{}", rest.split('/').next().unwrap_or("")),
        None => url.split(':').next().unwrap_or("").to_string() + ":",
    }
}

/// Sorts playwright-rs errors into what the engine cares about.
fn map_error(error: playwright_rs::Error) -> DriverError {
    use playwright_rs::Error as E;
    match error {
        E::Timeout(m) | E::AssertionTimeout(m) => DriverError::Timeout(m),
        E::NavigationTimeout { url, duration_ms } => {
            DriverError::Timeout(format!("loading {url} took over {duration_ms} ms"))
        }
        E::TargetClosed { target_type, .. } => DriverError::Closed(target_type),
        E::ChannelClosed => DriverError::Closed("connection to the browser closed".into()),
        E::TransportError(m) | E::ConnectionFailed(m) => DriverError::Closed(m),
        other => {
            let text = trim_stack(&other.to_string());
            // Some closes arrive as plain protocol errors.
            if text.contains("has been closed") || text.contains("Target closed") {
                DriverError::Closed(text)
            } else if text.contains("Timeout") && text.contains("exceeded") {
                DriverError::Timeout(text)
            } else {
                DriverError::Other(text)
            }
        }
    }
}

/// Playwright errors end with a JavaScript stack trace (`at
/// _FrameSession._navigate (C:\Users\...\coreBundle.js:...)`) and
/// sometimes a call log. Neither helps the admin, and the trace shows file
/// paths, so both are cut.
fn trim_stack(text: &str) -> String {
    let mut end = text.len();
    for marker in ["\n", " at _", " at async ", "Call log:"] {
        if let Some(i) = text.find(marker) {
            end = end.min(i);
        }
    }
    text[..end].trim().to_string()
}

impl Session {
    fn check_open(&self) -> DriverResult<()> {
        if self.page.is_closed() {
            Err(DriverError::Closed("the tab was closed".into()))
        } else if !self.is_running() {
            Err(DriverError::Closed("Edge exited".into()))
        } else {
            Ok(())
        }
    }
}

impl BrowserDriver for Session {
    async fn goto(&self, url: &str) -> DriverResult<()> {
        self.check_open()?;
        self.page
            .goto(url, GotoOptions::new().timeout(NAVIGATION_TIMEOUT))
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    async fn wait_visible(&self, selector: &str, timeout_ms: u64) -> DriverResult<()> {
        self.check_open()?;
        let options = WaitForOptions::builder().state(WaitForState::Visible).timeout(timeout_ms as f64).build();
        self.field(selector).wait_for(options).await.map_err(map_error)
    }

    async fn frame_url_of(&self, selector: &str) -> DriverResult<String> {
        self.check_open()?;
        // `page.locator()` never looks inside iframes (that needs a frame
        // locator, which recipes don't have). So any field the engine can
        // fill is in the main frame, and the main frame's URL is the one to
        // check. It comes from the browser's navigation events, which the
        // page's scripts can't fake.
        self.field(selector).wait_for(None).await.map_err(map_error)?;
        Ok(self.page.url())
    }

    async fn fill(&self, selector: &str, text: &SecretString) -> DriverResult<()> {
        self.check_open()?;
        // The text is exposed here, to be handed to the browser, and
        // nowhere else. playwright-rs logs it only at `trace`, which the
        // app's log filter never allows.
        self.field(selector).fill(text.expose_secret(), None).await.map_err(map_error)
    }

    async fn click(&self, selector: &str) -> DriverResult<()> {
        self.check_open()?;
        self.field(selector).click(None).await.map_err(map_error)
    }

    async fn select(&self, selector: &str, value: &str) -> DriverResult<()> {
        self.check_open()?;
        self.field(selector).select_option(value, None).await.map(|_| ()).map_err(map_error)
    }

    async fn press(&self, selector: &str, key: &str) -> DriverResult<()> {
        self.check_open()?;
        self.field(selector).press(key, None).await.map_err(map_error)
    }

    async fn current_url(&self) -> DriverResult<String> {
        self.check_open()?;
        Ok(self.page.url())
    }

    async fn is_visible(&self, selector: &str) -> DriverResult<bool> {
        self.check_open()?;
        self.field(selector).is_visible().await.map_err(map_error)
    }

    async fn page_text(&self) -> DriverResult<String> {
        self.check_open()?;
        self.page.locator("body").inner_text().await.map_err(map_error)
    }

    async fn bring_to_front(&self) -> DriverResult<()> {
        self.check_open()?;
        self.page.bring_to_front().await.map_err(map_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_traces_are_cut() {
        let raw = "Protocol error: net::ERR_CERT_AUTHORITY_INVALID at https://localhost:8443/simple/login \
                   Error: net::ERR_CERT_AUTHORITY_INVALID at https://localhost:8443/simple/login \
                   at _FrameSession._navigate (C:\\Users\\x\\coreBundle.js:39087:17) at async _Frame.gotoImpl";
        let trimmed = trim_stack(raw);
        assert!(trimmed.contains("net::ERR_CERT_AUTHORITY_INVALID"));
        assert!(!trimmed.contains("coreBundle"), "{trimmed}");
        assert_eq!(trim_stack("a\nCall log:\n - waiting"), "a");
    }

    #[test]
    fn short_origin_drops_paths() {
        assert_eq!(short_origin("https://a.example:8443/x/y?z"), "https://a.example:8443");
        assert_eq!(short_origin("about:blank"), "about:");
    }

    #[test]
    fn closed_and_timeout_errors_are_recognized() {
        assert!(matches!(map_error(playwright_rs::Error::ChannelClosed), DriverError::Closed(_)));
        assert!(matches!(map_error(playwright_rs::Error::Timeout("x".into())), DriverError::Timeout(_)));
        assert!(matches!(
            map_error(playwright_rs::Error::ProtocolError("Target page, context or browser has been closed".into())),
            DriverError::Closed(_)
        ));
        assert!(matches!(map_error(playwright_rs::Error::ProtocolError("other".into())), DriverError::Other(_)));
    }
}
