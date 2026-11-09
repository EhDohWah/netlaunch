//! The `BrowserDriver` trait: everything the engine needs from a browser
//! tab, and nothing more.
//!
//! The engine never imports playwright-rs. `netlaunch-driver-pw`
//! implements this trait with it, and the engine's own tests use a fake.
//! Keeping the trait small keeps the automation library swappable.
//!
//! **Selectors** are CSS. Every method acts on the *first visible* element
//! that matches, so a generic selector such as
//! `input[type=text], input[type=email]` works on pages with several
//! inputs and skips hidden decoy fields.

use std::future::Future;

use secrecy::SecretString;

/// Why a driver call failed.
#[derive(Debug, Clone, PartialEq)]
pub enum DriverError {
    /// The element didn't appear, or the action didn't finish, in time.
    Timeout(String),
    /// The tab or browser is gone (the admin closed the window).
    Closed(String),
    /// Anything else, as the library describes it.
    Other(String),
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverError::Timeout(m) => write!(f, "timed out: {m}"),
            DriverError::Closed(m) => write!(f, "the browser window was closed: {m}"),
            DriverError::Other(m) => write!(f, "{m}"),
        }
    }
}

pub type DriverResult<T> = Result<T, DriverError>;

/// One browser tab the engine drives.
///
/// The methods return `impl Future + Send` rather than being `async fn`,
/// so the compiler checks every implementation's future can move between
/// threads. Tauri runs commands on a multi-threaded runtime and needs
/// that. Implementations may still write `async fn`.
pub trait BrowserDriver: Send + Sync {
    /// Navigates to `url` and waits for the page to load.
    fn goto(&self, url: &str) -> impl Future<Output = DriverResult<()>> + Send;

    /// Waits until a match for `selector` is visible.
    fn wait_visible(&self, selector: &str, timeout_ms: u64) -> impl Future<Output = DriverResult<()>> + Send;

    /// The URL of the frame that holds the first visible match for
    /// `selector`. The engine checks its origin before every fill.
    ///
    /// Must come from the browser's own record of the frame, not from
    /// JavaScript in the page, which the page could fake.
    fn frame_url_of(&self, selector: &str) -> impl Future<Output = DriverResult<String>> + Send;

    /// Types `text` into the first visible match, with real input events.
    ///
    /// `text` stays a `SecretString` until the last moment: implementations
    /// expose it only to hand it to the browser, and never log it.
    fn fill(&self, selector: &str, text: &SecretString) -> impl Future<Output = DriverResult<()>> + Send;

    fn click(&self, selector: &str) -> impl Future<Output = DriverResult<()>> + Send;

    /// Chooses the `<option>` with this value.
    fn select(&self, selector: &str, value: &str) -> impl Future<Output = DriverResult<()>> + Send;

    /// Presses `key` (`Enter`, `Tab`...) with the match focused.
    fn press(&self, selector: &str, key: &str) -> impl Future<Output = DriverResult<()>> + Send;

    /// The current URL of the tab (main frame). Same-page URL changes made
    /// by single-page apps (`history.pushState`) count.
    fn current_url(&self) -> impl Future<Output = DriverResult<String>> + Send;

    /// True if a match for `selector` is visible right now. Doesn't wait.
    fn is_visible(&self, selector: &str) -> impl Future<Output = DriverResult<bool>> + Send;

    /// The page's visible text, for `text` rules. Only searched, never
    /// logged or stored.
    fn page_text(&self) -> impl Future<Output = DriverResult<String>> + Send;

    /// Brings the window to the front, for the hand-over.
    fn bring_to_front(&self) -> impl Future<Output = DriverResult<()>> + Send;
}
