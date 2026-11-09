//! A fake browser tab for the engine's tests: a URL, a set of visible
//! selectors, and hooks that change them when the engine clicks or fills.
//! It records what was filled so tests can check it.

use std::collections::HashSet;
use std::sync::Mutex;

use secrecy::{ExposeSecret, SecretString};

use crate::driver::{BrowserDriver, DriverError, DriverResult};

pub const USER: &str = "input[autocomplete=username], input[type=email], input[type=text]";
pub const PASS: &str = "input[type=password]";
pub const SUBMIT: &str = "button[type=submit], input[type=submit]";
pub const ALERT: &str = "[role=alert]";
pub const OTP: &str = "input[autocomplete=one-time-code]";

type Hook = Box<dyn FnMut(&mut Page, &str) + Send>;

/// What the fake tab shows.
#[derive(Default)]
pub struct Page {
    pub url: String,
    pub visible: HashSet<String>,
    pub text: String,
    pub closed: bool,
}

#[derive(Default)]
struct State {
    page: Page,
    /// (selector, text typed). Tests only: the real driver keeps nothing.
    fills: Vec<(String, String)>,
    clicks: Vec<String>,
    brought_to_front: bool,
    on_goto: Option<Hook>,
    on_click: Option<Hook>,
    on_fill: Option<Hook>,
    fill_error: Option<DriverError>,
    goto_error: Option<DriverError>,
}

#[derive(Default)]
pub struct FakeDriver {
    state: Mutex<State>,
}

impl FakeDriver {
    /// A tab where `goto` lands on `url` with the generic login form.
    pub fn login_form_at(url: &str) -> Self {
        let driver = FakeDriver::default();
        let url = url.to_string();
        driver.on_goto(move |page, _| {
            page.url = url.clone();
            page.visible = [USER, PASS, SUBMIT].map(String::from).into();
        });
        driver
    }

    pub fn on_goto(&self, hook: impl FnMut(&mut Page, &str) + Send + 'static) {
        self.state.lock().unwrap().on_goto = Some(Box::new(hook));
    }

    pub fn on_click(&self, hook: impl FnMut(&mut Page, &str) + Send + 'static) {
        self.state.lock().unwrap().on_click = Some(Box::new(hook));
    }

    pub fn on_fill(&self, hook: impl FnMut(&mut Page, &str) + Send + 'static) {
        self.state.lock().unwrap().on_fill = Some(Box::new(hook));
    }

    /// Test helpers that act as the admin would: navigate or click
    /// without going through the engine. Errors are ignored.
    pub async fn goto_now(&self, url: &str) {
        let _ = BrowserDriver::goto(self, url).await;
    }

    pub async fn click_now(&self, selector: &str) {
        self.state.lock().unwrap().page.visible.insert(selector.to_string());
        let _ = BrowserDriver::click(self, selector).await;
    }

    pub fn fail_goto_with(&self, error: DriverError) {
        self.state.lock().unwrap().goto_error = Some(error);
    }

    pub fn fail_fills_with(&self, error: DriverError) {
        self.state.lock().unwrap().fill_error = Some(error);
    }

    pub fn fills(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().fills.clone()
    }

    pub fn clicks(&self) -> Vec<String> {
        self.state.lock().unwrap().clicks.clone()
    }

    pub fn brought_to_front(&self) -> bool {
        self.state.lock().unwrap().brought_to_front
    }

    /// Runs a hook stored in the state. It's taken out first, because the
    /// hook needs `&mut Page`, which lives in the same state.
    fn fire(&self, which: fn(&mut State) -> &mut Option<Hook>, arg: &str) {
        let mut state = self.state.lock().unwrap();
        if let Some(mut hook) = which(&mut state).take() {
            hook(&mut state.page, arg);
            *which(&mut state) = Some(hook);
        }
    }

    fn check_open(&self) -> DriverResult<()> {
        if self.state.lock().unwrap().page.closed {
            Err(DriverError::Closed("fake window closed".into()))
        } else {
            Ok(())
        }
    }

    fn require_visible(&self, selector: &str) -> DriverResult<()> {
        self.check_open()?;
        if self.state.lock().unwrap().page.visible.contains(selector) {
            Ok(())
        } else {
            Err(DriverError::Timeout(format!("{selector} not visible")))
        }
    }
}

impl BrowserDriver for FakeDriver {
    async fn goto(&self, url: &str) -> DriverResult<()> {
        self.check_open()?;
        if let Some(error) = self.state.lock().unwrap().goto_error.clone() {
            return Err(error);
        }
        self.state.lock().unwrap().page.url = url.to_string();
        self.fire(|s| &mut s.on_goto, url);
        Ok(())
    }

    async fn wait_visible(&self, selector: &str, _timeout_ms: u64) -> DriverResult<()> {
        self.require_visible(selector)
    }

    async fn frame_url_of(&self, selector: &str) -> DriverResult<String> {
        self.require_visible(selector)?;
        Ok(self.state.lock().unwrap().page.url.clone())
    }

    async fn fill(&self, selector: &str, text: &SecretString) -> DriverResult<()> {
        self.require_visible(selector)?;
        if let Some(error) = self.state.lock().unwrap().fill_error.clone() {
            return Err(error);
        }
        self.state.lock().unwrap().fills.push((selector.to_string(), text.expose_secret().to_string()));
        self.fire(|s| &mut s.on_fill, selector);
        Ok(())
    }

    async fn click(&self, selector: &str) -> DriverResult<()> {
        self.require_visible(selector)?;
        self.state.lock().unwrap().clicks.push(selector.to_string());
        self.fire(|s| &mut s.on_click, selector);
        Ok(())
    }

    async fn select(&self, selector: &str, _value: &str) -> DriverResult<()> {
        self.require_visible(selector)
    }

    async fn press(&self, selector: &str, _key: &str) -> DriverResult<()> {
        self.require_visible(selector)
    }

    async fn current_url(&self) -> DriverResult<String> {
        self.check_open()?;
        Ok(self.state.lock().unwrap().page.url.clone())
    }

    async fn is_visible(&self, selector: &str) -> DriverResult<bool> {
        self.check_open()?;
        Ok(self.state.lock().unwrap().page.visible.contains(selector))
    }

    async fn page_text(&self) -> DriverResult<String> {
        self.check_open()?;
        Ok(self.state.lock().unwrap().page.text.clone())
    }

    async fn bring_to_front(&self) -> DriverResult<()> {
        self.state.lock().unwrap().brought_to_front = true;
        Ok(())
    }
}
