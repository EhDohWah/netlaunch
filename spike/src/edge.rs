//! Finding, launching and probing Edge without letting playwright-rs own the
//! process. Owning the process ourselves is what makes "released" windows
//! possible: the browser is not a child of the Playwright driver, so
//! disconnecting the driver leaves it running.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Locates the installed `msedge.exe`.
///
/// Order: the `App Paths` registry key (machine, then user), then the
/// standard install folders. Returns `None` on non-Windows systems; pass
/// `--browser` there.
pub fn find_edge() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe";
        for root in [windows_registry::LOCAL_MACHINE, windows_registry::CURRENT_USER] {
            if let Ok(key) = root.open(APP_PATHS) {
                if let Ok(value) = key.get_string("") {
                    let path = PathBuf::from(value.trim_matches('"'));
                    if path.is_file() {
                        return Some(path);
                    }
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
/// The first Windows run showed why they are needed. Without them, a brand
/// new profile on a work PC signed itself in to the Windows account,
/// synced favorites and installed the synced extensions, which then run
/// next to the password field. Playwright adds most of these when it
/// launches a browser itself; NetLaunch launches Edge itself, so it must
/// add them too. The list follows Playwright 1.64's `chromiumSwitches.ts`,
/// keeping only the ones about isolation, not the test-only ones.
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
        // Edge-only, both: stop Edge forcing a sign-in on the new profile,
        // and stop it moving intranet URLs (such as 10.x consoles) into the
        // admin's "Work" profile window, away from the automated one.
        "--disable-features=msForceBrowserSignIn,SwitchIntranetSitesToWorkProfile",
        // Edge on Windows can relaunch itself, which would replace the
        // process we track (and close later) with one we do not know about.
        "--edge-skip-compat-layer-relaunch",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Writes a starting `Default/Preferences` file so Edge does not offer to
/// save the password it just saw typed: the vault is the only place that
/// password should be stored. A machine policy can still override this,
/// which is why the spike reads the value back after the run.
pub fn seed_preferences(profile_dir: &Path) -> std::io::Result<()> {
    let default_dir = profile_dir.join("Default");
    fs::create_dir_all(&default_dir)?;
    let prefs = r#"{
  "credentials_enable_service": false,
  "profile": { "password_manager_enabled": false },
  "autofill": { "profile_enabled": false, "credit_card_enabled": false }
}"#;
    fs::write(default_dir.join("Preferences"), prefs)
}

/// Starts the browser as an independent process with its own profile folder
/// and a debugging port the browser picks itself (`--remote-debugging-port=0`).
///
/// Recent Chrome/Edge versions refuse remote debugging on the default
/// profile, so `profile_dir` must never be the user's normal Edge profile.
pub fn launch(
    browser: &Path,
    profile_dir: &Path,
    isolate: bool,
    headless: bool,
    extra_args: &[String],
) -> std::io::Result<Child> {
    let mut cmd = Command::new(browser);
    cmd.arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--remote-debugging-port=0")
        .arg("--no-first-run")
        .arg("--no-default-browser-check");
    if isolate {
        cmd.args(isolation_args());
    }
    cmd.args(extra_args);
    if headless {
        cmd.arg("--headless=new");
    }
    cmd.arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Detach from our console and Ctrl+C group, so the browser is not torn
    // down with the spike when it exits.
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

/// Waits for the browser to write `DevToolsActivePort` into the profile
/// folder and returns the port from its first line.
pub fn wait_for_debug_port(profile_dir: &Path, timeout: Duration) -> Result<u16, String> {
    let file = profile_dir.join("DevToolsActivePort");
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(text) = fs::read_to_string(&file) {
            if let Some(port) = text.lines().next().and_then(|l| l.trim().parse::<u16>().ok()) {
                if port != 0 {
                    return Ok(port);
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{} did not appear within {}s. Is remote debugging blocked by policy? Check edge://policy for RemoteDebuggingAllowed.",
                file.display(),
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// True if the DevTools HTTP endpoint on `port` still answers. Used to check
/// whether the browser survived a disconnect, and that it is gone after a
/// managed close.
pub fn debug_endpoint_alive(port: u16) -> bool {
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

/// Deletes a profile folder, retrying briefly because Windows keeps files
/// locked for a moment after the browser exits.
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

/// Waits up to `timeout` for the browser process to exit on its own.
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

/// What a profile folder says about sign-in, extensions and password
/// saving. Read from `Default/Preferences` and `Default/Secure Preferences`,
/// which Edge only writes completely when it shuts down cleanly.
#[derive(Debug, Default)]
pub struct ProfileFacts {
    /// JSON paths of `email` fields that hold an address, such as
    /// `account_info[0].email`. The addresses themselves are never kept.
    pub account_email_paths: Vec<String>,
    /// Every non-component entry under `extensions.settings`.
    pub extensions: Vec<ExtensionEntry>,
    /// The `credentials_enable_service` pref, if present.
    pub password_saving: Option<bool>,
}

/// One entry under `extensions.settings`. An entry is not proof that the
/// extension runs: Edge keeps entries for built-in and disabled ones too.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionEntry {
    pub id: String,
    pub name: Option<String>,
    pub location: Option<u64>,
}

pub fn read_profile_facts(profile_dir: &Path) -> Result<ProfileFacts, String> {
    let default_dir = profile_dir.join("Default");
    let mut facts = ProfileFacts::default();
    let mut read_any = false;

    for name in ["Preferences", "Secure Preferences"] {
        let Ok(text) = fs::read_to_string(default_dir.join(name)) else {
            continue;
        };
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
        read_any = true;

        collect_email_paths(&json, String::new(), &mut facts.account_email_paths);

        if let Some(value) = json.get("credentials_enable_service").and_then(|v| v.as_bool()) {
            facts.password_saving = Some(value);
        }

        // Extension locations from Chromium's ManifestLocation enum:
        // 5 = component, 10 = external component (both built in to Edge).
        if let Some(settings) = json.pointer("/extensions/settings").and_then(|v| v.as_object()) {
            for (id, entry) in settings {
                let location = entry.get("location").and_then(|v| v.as_u64());
                if matches!(location, Some(5) | Some(10)) {
                    continue;
                }
                let name = entry.pointer("/manifest/name").and_then(|v| v.as_str()).map(str::to_string);
                // The same id can appear in both files; keep the entry with
                // the most information.
                match facts.extensions.iter_mut().find(|e| e.id == *id) {
                    Some(existing) => {
                        existing.name = existing.name.take().or(name);
                        existing.location = existing.location.or(location);
                    }
                    None => facts.extensions.push(ExtensionEntry { id: id.clone(), name, location }),
                }
            }
        }
    }

    if read_any { Ok(facts) } else { Err("no Preferences file in the profile".into()) }
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

/// Edge policies set on this machine that decide whether isolation can
/// work. Command-line switches cannot override a policy, so these explain
/// a failed isolation check. Only names and simple values are read.
pub fn edge_policies() -> Vec<(String, String)> {
    #[cfg(windows)]
    {
        const KEY: &str = r"SOFTWARE\Policies\Microsoft\Edge";
        const VALUES: &[&str] = &[
            "BrowserSignin",
            "ImplicitSignInEnabled",
            "SyncDisabled",
            "ForceSync",
            "PasswordManagerEnabled",
            "RemoteDebuggingAllowed",
            "ExtensionInstallBlocklist",
        ];
        const SUBKEYS: &[&str] = &["ExtensionInstallForcelist", "ManagedFavorites"];

        let mut found = Vec::new();
        for (hive_name, hive) in [("HKLM", windows_registry::LOCAL_MACHINE), ("HKCU", windows_registry::CURRENT_USER)] {
            let Ok(key) = hive.open(KEY) else { continue };
            for name in VALUES {
                if let Ok(value) = key.get_u32(name) {
                    found.push((format!("{hive_name} {name}"), value.to_string()));
                } else if let Ok(value) = key.get_string(name) {
                    found.push((format!("{hive_name} {name}"), value));
                }
            }
            for name in SUBKEYS {
                if key.open(name).is_ok() {
                    found.push((format!("{hive_name} {name}"), "set".to_string()));
                }
            }
        }
        found
    }

    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_profile_facts_without_exposing_emails() {
        let dir = std::env::temp_dir().join(format!("netlaunch-facts-{}", std::process::id()));
        let default_dir = dir.join("Default");
        fs::create_dir_all(&default_dir).unwrap();
        fs::write(
            default_dir.join("Preferences"),
            r#"{ "credentials_enable_service": false,
                 "account_info": [ { "email": "someone@example.com" } ] }"#,
        )
        .unwrap();
        fs::write(
            default_dir.join("Secure Preferences"),
            r#"{ "extensions": { "settings": {
                   "aaa": { "location": 5,  "manifest": { "name": "Built in" } },
                   "bbb": { "location": 1,  "manifest": { "name": "Translator" } },
                   "ccc": { "location": 10 } } } }"#,
        )
        .unwrap();

        let facts = read_profile_facts(&dir).unwrap();
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(facts.account_email_paths, vec!["account_info[0].email".to_string()]);
        let ids: Vec<_> = facts.extensions.iter().map(|e| (e.id.as_str(), e.name.as_deref(), e.location)).collect();
        assert_eq!(ids, vec![("bbb", Some("Translator"), Some(1))]);
        assert_eq!(facts.password_saving, Some(false));
    }

    #[test]
    fn isolation_args_include_sync_and_extensions() {
        let args = isolation_args();
        assert!(args.iter().any(|a| a == "--disable-sync"));
        assert!(args.iter().any(|a| a == "--disable-extensions"));
    }
}
