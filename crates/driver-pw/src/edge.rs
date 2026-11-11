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

/// The browsers NetLaunch can find by itself. Both are Chromium-based:
/// NetLaunch drives the window over the DevTools protocol, which Firefox
/// doesn't offer in a way playwright-rs can attach to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserKind {
    Edge,
    Chrome,
}

impl BrowserKind {
    pub const ALL: [BrowserKind; 2] = [BrowserKind::Edge, BrowserKind::Chrome];

    pub fn name(self) -> &'static str {
        match self {
            BrowserKind::Edge => "Microsoft Edge",
            BrowserKind::Chrome => "Google Chrome",
        }
    }

    /// (exe name, install folder under Program Files or LOCALAPPDATA).
    #[cfg(windows)]
    fn install(self) -> (&'static str, [&'static str; 3]) {
        match self {
            BrowserKind::Edge => ("msedge.exe", ["Microsoft", "Edge", "Application"]),
            BrowserKind::Chrome => ("chrome.exe", ["Google", "Chrome", "Application"]),
        }
    }
}

/// Locates an installed browser: the `App Paths` registry key (machine,
/// then user), then the standard install folders. `None` when it isn't
/// installed, and always on other systems.
pub fn find_browser(kind: BrowserKind) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let (exe, folders) = kind.install();
        let app_paths = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        for root in [windows_registry::LOCAL_MACHINE, windows_registry::CURRENT_USER] {
            if let Ok(key) = root.open(&app_paths)
                && let Ok(value) = key.get_string("")
            {
                let path = PathBuf::from(value.trim_matches('"'));
                if path.is_file() {
                    return Some(path);
                }
            }
        }

        let suffix: PathBuf = folders.iter().collect::<PathBuf>().join(exe);
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
        let _ = kind;
        None
    }
}

/// Locates the installed `msedge.exe`.
pub fn find_edge() -> Option<PathBuf> {
    find_browser(BrowserKind::Edge)
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
        // Edge-only (Chrome ignores switches and features it doesn't
        // know): don't force a sign-in on the new profile, and don't
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
            return Err("The browser exited straight away. If this site's window is still open from an earlier \
                        launch, close it and try again."
                .into());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "The browser didn't finish starting within {} s. If this keeps happening, remote debugging \
                 may be blocked by policy: check edge://policy (or chrome://policy) for RemoteDebuggingAllowed.",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// True if Edge's DevTools endpoint on `port` still answers. This is how
/// NetLaunch tells whether the browser is still there when the process it
/// started has exited: Edge can hand over to a new process (after an
/// update, for example) and keep the window open. Blocking, but quick: a
/// closed port refuses at once.
pub fn debug_endpoint_alive(port: u16) -> bool {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let request = format!("GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    match stream.read(&mut head) {
        Ok(n) => String::from_utf8_lossy(&head[..n]).starts_with("HTTP/1.1 200"),
        Err(_) => false,
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

/// Where in the profile's preferences an account's email address is
/// stored, such as `account_info[0].email`: the sign of Edge's implicit
/// sign-in to the Windows account (decision 9). Only the JSON paths are
/// returned, never the addresses.
///
/// Edge writes `Default/Preferences` a few seconds after a change, so
/// while it runs this can lag behind; call it again later.
pub fn signed_in_account_paths(profile_dir: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    let Ok(text) = fs::read_to_string(profile_dir.join("Default").join("Preferences")) else {
        return paths;
    };
    // Edge may be halfway through rewriting the file; a parse error just
    // means "ask again later".
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
        collect_email_paths(&json, String::new(), &mut paths);
    }
    paths
}

/// Records the JSON path of every `email` field holding an address.
fn collect_email_paths(value: &serde_json::Value, path: String, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map {
                let child = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                match v {
                    serde_json::Value::String(s) if key == "email" && s.contains('@') => out.push(child),
                    other => collect_email_paths(other, child, out),
                }
            }
        }
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                collect_email_paths(item, format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// The folder the installer puts the Playwright driver in, next to the
/// app's exe: `<exe folder>\playwright-driver` (holding `node.exe` and
/// `package\cli.js`). `None` when it isn't there, as in development, where
/// playwright-rs uses the driver its build script downloaded.
pub fn bundled_driver_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("playwright-driver");
    let node = if cfg!(windows) { "node.exe" } else { "node" };
    (dir.join(node).is_file() && dir.join("package").join("cli.js").is_file()).then_some(dir)
}

/// Points playwright-rs at the installer's bundled driver, unless
/// `PLAYWRIGHT_DRIVER_PATH` is already set. Returns the folder used.
///
/// # Safety
///
/// Changes an environment variable, so call it first thing in `main`,
/// before any other thread exists (Rust 2024 marks `set_var` unsafe for
/// that reason).
pub unsafe fn use_bundled_driver() -> Option<PathBuf> {
    if std::env::var_os("PLAYWRIGHT_DRIVER_PATH").is_some() {
        return None;
    }
    let dir = bundled_driver_dir()?;
    // SAFETY: the caller promises no other thread is running.
    unsafe { std::env::set_var("PLAYWRIGHT_DRIVER_PATH", &dir) };
    Some(dir)
}

/// Deletes fresh-profile folders left behind by earlier runs: windows
/// that were released (Edge kept running, so the folder couldn't be
/// deleted then) and since closed, or NetLaunch exiting abruptly.
/// Returns how many it deleted.
///
/// A folder still in use is skipped: Edge holds its `lockfile` open
/// without sharing, so deleting that file fails. Folders younger than
/// `min_age` are skipped too, in case another NetLaunch is starting a
/// window right now. Windows only; elsewhere it does nothing.
pub fn sweep_fresh_profiles(min_age: Duration) -> usize {
    #[cfg(windows)]
    {
        sweep_fresh_profiles_in(&std::env::temp_dir().join("NetLaunch"), min_age)
    }
    #[cfg(not(windows))]
    {
        let _ = min_age;
        0
    }
}

#[cfg(windows)]
fn sweep_fresh_profiles_in(root: &Path, min_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(root) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_profile = entry.file_name().to_string_lossy().starts_with("profile-") && path.is_dir();
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= min_age);
        if !is_profile || !old_enough {
            continue;
        }
        let lock = path.join("lockfile");
        // Locked: Edge is still using this profile (a released window).
        if lock.exists() && fs::remove_file(&lock).is_err() {
            continue;
        }
        if remove_profile(&path).is_ok() {
            removed += 1;
        }
    }
    removed
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
    fn finds_a_signed_in_account_without_reading_the_address() {
        let dir = fresh_profile_dir();
        assert!(signed_in_account_paths(&dir).is_empty(), "no profile yet");
        seed_preferences(&dir).unwrap();
        assert!(signed_in_account_paths(&dir).is_empty(), "seeded profile is signed out");

        fs::write(
            dir.join("Default").join("Preferences"),
            r#"{ "account_info": [ { "email": "someone@example.com", "gaia": "x" } ],
                 "other": { "email": "not-an-address" } }"#,
        )
        .unwrap();
        assert_eq!(signed_in_account_paths(&dir), vec!["account_info[0].email".to_string()]);

        fs::write(dir.join("Default").join("Preferences"), "{ half written").unwrap();
        assert!(signed_in_account_paths(&dir).is_empty());
        remove_profile(&dir).unwrap();
    }

    /// Edge on Windows holds `lockfile` open without sharing while the
    /// profile is in use. The sweep must leave such a folder alone.
    #[cfg(windows)]
    #[test]
    fn sweep_skips_profiles_in_use_and_young_ones() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = std::env::temp_dir().join(format!("netlaunch-sweep-test-{}", std::process::id()));
        let make = |name: &str| {
            let dir = root.join(name);
            fs::create_dir_all(dir.join("Default")).unwrap();
            fs::write(dir.join("lockfile"), "").unwrap();
            dir
        };
        let closed = make("profile-closed");
        let in_use = make("profile-in-use");
        let not_ours = make("something-else");
        let held = fs::OpenOptions::new().read(true).write(true).share_mode(0).open(in_use.join("lockfile")).unwrap();

        // Everything is brand new: nothing is old enough yet.
        assert_eq!(sweep_fresh_profiles_in(&root, Duration::from_secs(3600)), 0);

        assert_eq!(sweep_fresh_profiles_in(&root, Duration::ZERO), 1);
        assert!(!closed.exists(), "a closed profile is deleted");
        assert!(in_use.join("Default").exists(), "a profile in use is left alone");
        assert!(not_ours.exists(), "only profile-* folders are touched");

        drop(held);
        assert_eq!(sweep_fresh_profiles_in(&root, Duration::ZERO), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fresh_profile_folders_are_unique() {
        assert_ne!(fresh_profile_dir(), fresh_profile_dir());
    }
}
