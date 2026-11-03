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

/// Starts the browser as an independent process with its own profile folder
/// and a debugging port the browser picks itself (`--remote-debugging-port=0`).
///
/// Recent Chrome/Edge versions refuse remote debugging on the default
/// profile, so `profile_dir` must never be the user's normal Edge profile.
pub fn launch(browser: &Path, profile_dir: &Path, headless: bool, extra_args: &[String]) -> std::io::Result<Child> {
    let mut cmd = Command::new(browser);
    cmd.arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--remote-debugging-port=0")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .args(extra_args);
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
