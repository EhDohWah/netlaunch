//! Finding and launching Edge, ported from the M1 spike (`spike/src/edge.rs`),
//! which proved each piece on Windows.
//!
//! NetLaunch starts Edge itself rather than letting playwright-rs do it.
//! The browser is then not a child of the Playwright driver, so a
//! "released" window keeps running after NetLaunch disconnects.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Locates the installed `msedge.exe`: the `App Paths` registry key
/// (machine, then user), then the standard install folders. `None` on
/// other systems.
pub fn find_edge() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe";
        for root in [windows_registry::LOCAL_MACHINE, windows_registry::CURRENT_USER] {
            if let Ok(key) = root.open(APP_PATHS)
                && let Ok(value) = key.get_string("")
            {
                let path = PathBuf::from(value.trim_matches('"'));
                if path.is_file() {
                    return Some(path);
                }
            }
        }

        let suffix = Path::new("Microsoft").join("Edge").join("Application").join("msedge.exe");
        for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                let path = PathBuf::from(base).join(&suffix);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        None
    }

    #[cfg(not(windows))]
    {
        None
    }
}

/// Switches that keep the login window apart from the admin's own Edge.
///
/// Without them, Edge on a work PC signed a brand-new profile in to the
/// Windows account, synced favorites and installed the synced extensions,
/// which then ran next to the password field (first spike run). The list
/// follows Playwright 1.64's `chromiumSwitches.ts`, keeping only the
/// switches about isolation.
pub fn isolation_args() -> Vec<String> {
    [
        // No sync: favorites, extensions, history and passwords stay out of
        // this profile, and admin console URLs stay out of the account.
        "--disable-sync",
        // No user, synced or default extensions in the window that receives
        // the password.
        "--disable-extensions",
        "--disable-component-extensions-with-background-pages",
        "--disable-default-apps",
        // Edge-only: don't force a sign-in on the new profile, and don't
        // move intranet URLs (such as 10.x consoles) into the admin's
        // "Work" profile window, away from the automated one.
        "--disable-features=msForceBrowserSignIn,SwitchIntranetSitesToWorkProfile",
        // Edge can relaunch itself, which would replace the process we
        // track (and close later) with one we don't know about.
        "--edge-skip-compat-layer-relaunch",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Writes a starting `Default/Preferences` so Edge never offers to save the
/// password it sees typed: the vault is the only place for it.
///
/// Only when the file doesn't exist yet. A persistent profile keeps the
/// file Edge wrote last time, which already has these values.
pub fn seed_preferences(profile_dir: &Path) -> std::io::Result<()> {
    let default_dir = profile_dir.join("Default");
    let file = default_dir.join("Preferences");
    if file.exists() {
        return Ok(());
    }
    fs::create_dir_all(&default_dir)?;
    let prefs = r#"{
  "credentials_enable_service": false,
  "profile": { "password_manager_enabled": false },
  "autofill": { "profile_enabled": false, "credit_card_enabled": false }
}"#;
    fs::write(file, prefs)
}

/// Starts the browser as an independent process, with its own profile
/// folder and a debugging port it picks itself (`--remote-debugging-port=0`).
///
/// Recent Chrome/Edge versions refuse remote debugging on the default
/// profile, so `profile_dir` must never be the admin's normal profile.
pub fn launch(browser: &Path, profile_dir: &Path, extra_args: &[String]) -> std::io::Result<Child> {
    // A persistent profile still holds the port file from last time. Remove
    // it, or we could read the old port before Edge writes the new one.
    let _ = fs::remove_file(profile_dir.join("DevToolsActivePort"));

    let mut cmd = Command::new(browser);
    cmd.arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--remote-debugging-port=0")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .args(isolation_args())
        .args(extra_args)
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Detach from our console and Ctrl+C group, so the browser isn't torn
    // down with NetLaunch (a released window must survive it).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    cmd.spawn()
}

/// Waits for the browser to write `DevToolsActivePort` and returns the port
/// from its first line.
pub async fn wait_for_debug_port(profile_dir: &Path, child: &mut Child, timeout: Duration) -> Result<u16, String> {
    let file = profile_dir.join("DevToolsActivePort");
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(text) = fs::read_to_string(&file)
            && let Some(port) = text.lines().next().and_then(|l| l.trim().parse::<u16>().ok())
            && port != 0
        {
            return Ok(port);
        }
        // Edge hands the request to a running Edge that already uses this
        // profile folder, then exits at once. That happens when a persistent
        // profile is still open in a released window.
        if let Ok(Some(_)) = child.try_wait() {
            return Err("Edge exited straight away. If this site's window is still open from an earlier \
                        launch, close it and try again."
                .into());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Edge didn't open its debugging port within {} s. Remote debugging may be blocked by \
                 policy: check edge://policy for RemoteDebuggingAllowed.",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits up to `timeout` for the browser process to exit on its own.
/// Blocking: call it from `spawn_blocking`.
pub fn wait_for_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Deletes a profile folder, retrying briefly because Windows keeps files
/// locked for a moment after the browser exits. Blocking.
pub fn remove_profile(profile_dir: &Path) -> std::io::Result<()> {
    let mut last = Ok(());
    for _ in 0..20 {
        last = fs::remove_dir_all(profile_dir);
        if last.is_ok() || !profile_dir.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    last
}

/// A new, unique folder for a fresh profile under the system temp folder.
/// It's deleted when the window is closed from NetLaunch.
pub fn fresh_profile_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir()
        .join("NetLaunch")
        .join(format!("profile-{}-{nanos}", std::process::id()))
}

/// `%LOCALAPPDATA%\NetLaunch\profiles\<site id>`: where a site with a
/// persistent profile keeps its cookies between launches.
pub fn persistent_profile_dir(site_id: i64) -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("NetLaunch").join("profiles").join(site_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolation_args_include_sync_and_extensions() {
        let args = isolation_args();
        assert!(args.iter().any(|a| a == "--disable-sync"));
        assert!(args.iter().any(|a| a == "--disable-extensions"));
    }

    #[test]
    fn seeding_keeps_an_existing_preferences_file() {
        let dir = fresh_profile_dir();
        seed_preferences(&dir).unwrap();
        let file = dir.join("Default").join("Preferences");
        assert!(fs::read_to_string(&file).unwrap().contains("\"credentials_enable_service\": false"));

        fs::write(&file, "{\"edge wrote\": true}").unwrap();
        seed_preferences(&dir).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "{\"edge wrote\": true}");
        remove_profile(&dir).unwrap();
    }

    #[test]
    fn fresh_profile_folders_are_unique() {
        assert_ne!(fresh_profile_dir(), fresh_profile_dir());
    }
}
