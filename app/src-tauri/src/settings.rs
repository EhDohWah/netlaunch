//! The app's settings: which browser opens sites, and whether to look for
//! updates at start-up. Kept in `%LOCALAPPDATA%\NetLaunch\settings.json`,
//! next to the vault. Nothing secret goes in here.

use std::path::{Path, PathBuf};

use netlaunch_driver_pw::BrowserKind;
use serde::{Deserialize, Serialize};

/// Which browser opens sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserChoice {
    #[default]
    Edge,
    Chrome,
    /// Another Chromium-based browser (Brave, Vivaldi...), by the path of
    /// its exe in `browser_path`.
    Custom,
}

/// `#[serde(default)]`: a field missing from the file (one added in a
/// later version) takes its default instead of failing the whole file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub browser: BrowserChoice,
    /// Only used when `browser` is `custom`.
    pub browser_path: Option<String>,
    /// Look for a new version when NetLaunch starts.
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { browser: BrowserChoice::Edge, browser_path: None, check_updates: true }
    }
}

/// An installed browser, for the Settings dropdown.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserInfo {
    pub id: BrowserChoice,
    pub name: &'static str,
    pub path: String,
}

/// `%LOCALAPPDATA%\NetLaunch\settings.json`.
pub fn default_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("NetLaunch").join("settings.json"))
}

/// Reads the settings. A missing or unreadable file gives the defaults:
/// settings must never stop the app from starting.
pub fn load(path: &Path) -> Settings {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!("settings.json couldn't be read ({e}); using the defaults");
            Settings::default()
        }),
        Err(_) => Settings::default(),
    }
}

/// Checks and tidies settings from the UI, and writes them.
pub fn save(path: Option<&Path>, mut settings: Settings) -> Result<Settings, String> {
    settings.browser_path = settings.browser_path.map(|p| p.trim().trim_matches('"').to_string()).filter(|p| !p.is_empty());
    // Fails with a clear message if the chosen browser isn't there.
    browser_path(&settings)?;
    let path = path.ok_or("LOCALAPPDATA isn't set, so there's nowhere to keep the settings.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("Couldn't save the settings: {e}"))?;
    Ok(settings)
}

/// The browsers found on this PC.
pub fn installed_browsers() -> Vec<BrowserInfo> {
    BrowserKind::ALL
        .into_iter()
        .filter_map(|kind| {
            let path = netlaunch_driver_pw::find_browser(kind)?;
            let id = match kind {
                BrowserKind::Edge => BrowserChoice::Edge,
                BrowserKind::Chrome => BrowserChoice::Chrome,
            };
            Some(BrowserInfo { id, name: kind.name(), path: path.display().to_string() })
        })
        .collect()
}

/// The exe of the browser the settings choose, or why it can't be used.
pub fn browser_path(settings: &Settings) -> Result<PathBuf, String> {
    let kind = match settings.browser {
        BrowserChoice::Edge => BrowserKind::Edge,
        BrowserChoice::Chrome => BrowserKind::Chrome,
        BrowserChoice::Custom => {
            let path = settings.browser_path.as_deref().ok_or("Enter the path of the browser's .exe file.")?;
            let path = PathBuf::from(path);
            return if path.is_file() {
                Ok(path)
            } else {
                Err(format!("There's no file at {}. Check the browser path in Settings.", path.display()))
            };
        }
    };
    netlaunch_driver_pw::find_browser(kind)
        .ok_or_else(|| format!("{} wasn't found on this PC. Pick another browser in Settings.", kind.name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("netlaunch-settings-{}-{name}", std::process::id())).join("settings.json")
    }

    #[test]
    fn a_missing_or_broken_file_gives_the_defaults() {
        let path = temp_file("missing");
        assert_eq!(load(&path), Settings::default());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Settings::default());
        // A file from an older version: missing fields take their defaults.
        std::fs::write(&path, r#"{ "check_updates": false }"#).unwrap();
        assert_eq!(load(&path), Settings { check_updates: false, ..Settings::default() });
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_custom_browser_must_exist_and_is_saved() {
        let path = temp_file("custom");
        let missing = Settings {
            browser: BrowserChoice::Custom,
            browser_path: Some("/no/such/browser.exe".into()),
            check_updates: true,
        };
        assert!(save(Some(&path), missing).is_err());
        assert!(!path.exists(), "nothing is written when the settings are refused");
        let no_path = Settings { browser: BrowserChoice::Custom, browser_path: Some("  ".into()), check_updates: true };
        assert!(save(Some(&path), no_path).is_err());

        // Any existing file will do as a stand-in for a browser; quotes
        // (from "Copy as path" in Explorer) and spaces are trimmed.
        let exe = std::env::current_exe().unwrap();
        let quoted = Settings {
            browser: BrowserChoice::Custom,
            browser_path: Some(format!(" \"{}\" ", exe.display())),
            check_updates: false,
        };
        let saved = save(Some(&path), quoted).unwrap();
        assert_eq!(saved.browser_path.as_deref(), Some(exe.to_str().unwrap()));
        assert_eq!(load(&path), saved);
        assert_eq!(browser_path(&saved).unwrap(), exe);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_json_uses_lowercase_browser_names() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains(r#""browser":"edge""#), "{json}");
        let chrome: Settings = serde_json::from_str(r#"{ "browser": "chrome" }"#).unwrap();
        assert_eq!(chrome.browser, BrowserChoice::Chrome);
    }
}
