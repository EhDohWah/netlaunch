//! M1 browser spike. Throwaway code: it answers questions, then the answers
//! go into `crates/driver-pw`.
//!
//! What it does, in order:
//! 1. Finds the installed Edge (or uses `--browser`).
//! 2. Launches it itself, with its own profile folder and
//!    `--remote-debugging-port=0`, and reads the port Edge picked.
//! 3. Attaches with playwright-rs `connect_over_cdp`.
//! 4. Logs in to a plain HTML form and a React-style controlled form.
//! 5. Checks that the log does not contain the password.
//! 6. Released mode: disconnects and checks Edge is still running.
//!    Managed mode: kills Edge and checks nothing is left.
//!
//! Run `spike help` for the options.

mod edge;
mod logcheck;
mod pages;

use std::path::{Path, PathBuf};
use std::process::{Child, ExitCode};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use playwright_rs::protocol::{WaitForOptions, WaitForState};
use playwright_rs::{Browser, Page, Playwright};

const USAGE: &str = "\
NetLaunch M1 spike

USAGE:
    spike find-edge
    spike run [OPTIONS]
    spike help

RUN OPTIONS:
    --mode released|managed   What happens after login (default: released)
                                released: disconnect and leave Edge open
                                managed:  close Edge when you press Enter
    --release-via close|shutdown
                              How to disconnect in released mode (default: close)
                                close:    Browser::close(), then stop the driver
                                shutdown: only stop the Playwright driver
    --close-after SECONDS     Managed mode: close after N seconds instead of
                              waiting for Enter
    --observe SECONDS         Wait this long after login before checking for
                              extra tabs, so sign-in, sync and extension
                              installs have time to show up (default: 15)
    --no-isolation            Launch Edge without the isolation switches and
                              preferences, to compare with the default
    --log-level LEVEL         info (default), debug or trace. Set here, never
                              from RUST_LOG
    --browser PATH            Browser executable (default: installed Edge)
    --browser-arg ARG         Extra browser argument; repeatable
    --headless                Run the browser headless (for CI-style checks)
    --driver-path DIR         Use this Playwright driver folder (holds node.exe
                              and package/cli.js); sets PLAYWRIGHT_DRIVER_PATH
";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Released,
    Managed,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum ReleaseVia {
    Close,
    Shutdown,
}

struct Options {
    mode: Mode,
    release_via: ReleaseVia,
    close_after: Option<u64>,
    observe: u64,
    isolate: bool,
    log_level: String,
    browser: Option<PathBuf>,
    browser_args: Vec<String>,
    headless: bool,
    driver_path: Option<PathBuf>,
}

fn parse_run_options(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        mode: Mode::Released,
        release_via: ReleaseVia::Close,
        close_after: None,
        observe: 15,
        isolate: true,
        log_level: "info".to_string(),
        browser: None,
        browser_args: Vec::new(),
        headless: false,
        driver_path: None,
    };
    while let Some(raw) = args.next() {
        // Accept both `--name value` and `--name=value`.
        let (arg, mut inline) = match raw.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name.to_string(), Some(value.to_string())),
            _ => (raw, None),
        };
        let mut value = |name: &str| {
            inline.take().or_else(|| args.next()).ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--mode" => {
                options.mode = match value("--mode")?.as_str() {
                    "released" => Mode::Released,
                    "managed" => Mode::Managed,
                    other => return Err(format!("unknown mode '{other}'")),
                }
            }
            "--release-via" => {
                options.release_via = match value("--release-via")?.as_str() {
                    "close" => ReleaseVia::Close,
                    "shutdown" => ReleaseVia::Shutdown,
                    other => return Err(format!("unknown release method '{other}'")),
                }
            }
            "--close-after" => {
                let raw = value("--close-after")?;
                options.close_after = Some(raw.parse().map_err(|_| format!("bad seconds '{raw}'"))?);
            }
            "--observe" => {
                let raw = value("--observe")?;
                options.observe = raw.parse().map_err(|_| format!("bad seconds '{raw}'"))?;
            }
            "--no-isolation" => options.isolate = false,
            "--log-level" => {
                let level = value("--log-level")?;
                if !matches!(level.as_str(), "info" | "debug" | "trace") {
                    return Err(format!("log level must be info, debug or trace, not '{level}'"));
                }
                options.log_level = level;
            }
            "--browser" => options.browser = Some(PathBuf::from(value("--browser")?)),
            "--browser-arg" => options.browser_args.push(value("--browser-arg")?),
            "--headless" => options.headless = true,
            "--driver-path" => options.driver_path = Some(PathBuf::from(value("--driver-path")?)),
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    Ok(options)
}

#[derive(PartialEq)]
enum Status {
    Pass,
    Fail,
    Info,
}

/// The table printed at the end, which is what to send back.
#[derive(Default)]
struct Report {
    rows: Vec<(Status, String, String)>,
}

impl Report {
    fn add(&mut self, status: Status, check: &str, detail: impl Into<String>) {
        let detail = detail.into();
        let tag = match status {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Info => "INFO",
        };
        println!("[{tag}] {check}: {detail}");
        self.rows.push((status, check.to_string(), detail));
    }

    fn check(&mut self, ok: bool, check: &str, detail: impl Into<String>) {
        self.add(if ok { Status::Pass } else { Status::Fail }, check, detail);
    }

    fn failed(&self) -> bool {
        self.rows.iter().any(|(status, _, _)| *status == Status::Fail)
    }

    fn print_summary(&self) {
        println!("\n===== M1 spike summary (copy everything below) =====");
        println!("os: {} {}", std::env::consts::OS, std::env::consts::ARCH);
        println!("playwright server: {}", playwright_rs::PLAYWRIGHT_VERSION);
        for (status, check, detail) in &self.rows {
            let tag = match status {
                Status::Pass => "PASS",
                Status::Fail => "FAIL",
                Status::Info => "INFO",
            };
            println!("{tag:4}  {check:<26} {detail}");
        }
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("find-edge") => match edge::find_edge() {
            Some(path) => {
                println!("{}", path.display());
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("Edge not found (registry App Paths and standard folders checked).");
                ExitCode::FAILURE
            }
        },
        Some("run") => match parse_run_options(args) {
            Ok(options) => run(options),
            Err(message) => {
                eprintln!("error: {message}\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        Some("help") | Some("--help") | Some("-h") | None => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown command '{other}'\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: Options) -> ExitCode {
    // Environment changes must happen before any other thread exists, so
    // before the tokio runtime starts.
    if let Some(dir) = &options.driver_path {
        // SAFETY: single-threaded at this point; no other thread reads the
        // environment concurrently.
        unsafe { std::env::set_var("PLAYWRIGHT_DRIVER_PATH", dir) };
    }

    let work_dir = std::env::temp_dir().join("netlaunch-spike");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let profile_dir = work_dir.join(format!("profile-{stamp}"));
    let log_file = work_dir.join(format!("spike-{stamp}.log"));
    if let Err(e) = std::fs::create_dir_all(&profile_dir) {
        eprintln!("error: cannot create {}: {e}", profile_dir.display());
        return ExitCode::FAILURE;
    }
    let capture = match logcheck::install(&options.log_level, &log_file) {
        Ok(capture) => capture,
        Err(e) => {
            eprintln!("error: cannot create log file {}: {e}", log_file.display());
            return ExitCode::FAILURE;
        }
    };

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut report = Report::default();
    let outcome = runtime.block_on(spike(&options, &profile_dir, &capture, &mut report));
    if let Err(message) = outcome {
        report.add(Status::Fail, "spike aborted", message);
    }
    report.add(Status::Info, "log file", log_file.display().to_string());
    report.print_summary();

    // Do not wait for the driver's background tasks; the browser is not
    // ours to stop here in released mode.
    runtime.shutdown_timeout(Duration::from_secs(2));
    if report.failed() { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

async fn spike(options: &Options, profile_dir: &Path, capture: &logcheck::Capture, report: &mut Report) -> Result<(), String> {
    // 1. Find the browser.
    let browser_exe = match options.browser.clone().or_else(edge::find_edge) {
        Some(path) => path,
        None => return Err("Edge not found; pass --browser <path to msedge.exe>".into()),
    };
    report.add(Status::Info, "browser", browser_exe.display().to_string());
    report.add(
        Status::Info,
        "driver override",
        std::env::var("PLAYWRIGHT_DRIVER_PATH").unwrap_or_else(|_| "(none: compiled-in driver path)".into()),
    );

    let policies = edge::edge_policies();
    if policies.is_empty() {
        report.add(Status::Info, "edge policies", "none of the checked policies are set");
    }
    for (name, value) in policies {
        report.add(Status::Info, "edge policy", format!("{name} = {value}"));
    }

    // 2. Launch it ourselves and read the port it picked.
    if options.isolate {
        edge::seed_preferences(profile_dir).map_err(|e| format!("could not write starting preferences: {e}"))?;
    }
    report.add(
        Status::Info,
        "isolation",
        if options.isolate { "on (switches + preferences)" } else { "OFF (--no-isolation)" },
    );
    let mut child = edge::launch(&browser_exe, profile_dir, options.isolate, options.headless, &options.browser_args)
        .map_err(|e| format!("could not start {}: {e}", browser_exe.display()))?;
    report.add(Status::Info, "profile folder", profile_dir.display().to_string());

    let port = match edge::wait_for_debug_port(profile_dir, Duration::from_secs(20)) {
        Ok(port) => port,
        Err(message) => {
            let _ = child.kill();
            report.check(false, "debug port", message.clone());
            return Err(message);
        }
    };
    report.check(true, "debug port", format!("127.0.0.1:{port}"));

    // 3..5 run inside a helper so any error still reaches the cleanup below.
    let attach = attach_and_login(port, capture, options, report).await;
    let (playwright, browser, base) = match attach {
        Ok(attached) => attached,
        Err(message) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(message);
        }
    };

    // Give sign-in, sync and extension installs time to happen, then look
    // for any tab the spike did not open.
    if options.observe > 0 {
        println!("\nWatching the window for {}s (sign-in, sync, extension tabs)...", options.observe);
        tokio::time::sleep(Duration::from_secs(options.observe)).await;
    }
    check_extra_tabs(&browser, &base, report);

    // 6. Hand the window over, or close it.
    match options.mode {
        Mode::Released => {
            release(playwright, browser, port, profile_dir, options.release_via, report).await;
            // Edge is still running, so its preference files may lag behind.
            report_profile(profile_dir, false, report);
        }
        Mode::Managed => {
            close_managed(playwright, browser, child, port, options.close_after, report).await;
            report_profile(profile_dir, true, report);
            // If any Edge process were still running on this profile,
            // Windows would keep its files locked and the delete would fail.
            match edge::remove_profile(profile_dir) {
                Ok(()) => report.check(true, "managed: profile removed", "no process holds the profile folder"),
                Err(e) => {
                    report.check(false, "managed: profile removed", format!("{e} (an Edge process may still be running)"))
                }
            }
        }
    }
    Ok(())
}

/// Every tab should be the spike's own test page or `about:blank`. Anything
/// else (an extension's welcome page, a sign-in prompt) means the window is
/// not isolated. Only scheme and host are printed, never full URLs.
fn check_extra_tabs(browser: &Browser, base: &str, report: &mut Report) {
    let mut extra = Vec::new();
    for context in browser.contexts() {
        for page in context.pages() {
            let url = page.url();
            if url == "about:blank" || url.starts_with(base) {
                continue;
            }
            let short = match url.split_once("://") {
                Some((scheme, rest)) => format!("{scheme}://{}", rest.split('/').next().unwrap_or("")),
                None => url,
            };
            extra.push(short);
        }
    }
    report.check(
        extra.is_empty(),
        "no extra tabs",
        if extra.is_empty() { "only the spike's tab is open".to_string() } else { extra.join(", ") },
    );
}

/// Reports what the profile folder says about sign-in, extensions and
/// password saving. They are checks only after a clean shutdown; while
/// Edge runs the files may be out of date, so they are info.
fn report_profile(profile_dir: &Path, clean_shutdown: bool, report: &mut Report) {
    let facts = match edge::read_profile_facts(profile_dir) {
        Ok(facts) => facts,
        Err(e) => {
            report.add(Status::Info, "profile facts", format!("not readable: {e}"));
            return;
        }
    };

    let mut row = |ok: bool, check: &str, detail: String| {
        if clean_shutdown {
            report.check(ok, check, detail);
        } else {
            report.add(Status::Info, check, format!("{detail} (read while Edge runs; may lag)"));
        }
    };
    row(
        facts.account_emails == 0,
        "no account signed in",
        format!("{} email field(s) found in the profile", facts.account_emails),
    );
    row(
        facts.user_extensions.is_empty(),
        "no user extensions",
        if facts.user_extensions.is_empty() { "none".to_string() } else { facts.user_extensions.join(", ") },
    );
    row(
        facts.password_saving == Some(false),
        "password saving off",
        format!("credentials_enable_service = {:?}", facts.password_saving),
    );
}

async fn attach_and_login(
    port: u16,
    capture: &logcheck::Capture,
    options: &Options,
    report: &mut Report,
) -> Result<(Playwright, Browser, String), String> {
    let playwright = Playwright::launch()
        .await
        .map_err(|e| format!("could not start the Playwright driver: {e}"))?;
    report.check(true, "driver started", "Playwright::launch ok");

    let endpoint = format!("http://127.0.0.1:{port}");
    let browser = playwright
        .chromium()
        .connect_over_cdp(&endpoint, None)
        .await
        .map_err(|e| format!("connect_over_cdp({endpoint}) failed: {e}"))?;
    report.check(true, "connect_over_cdp", format!("{} {}", browser.name(), browser.version()));

    // Use the browser's own default context and its existing tab. A context
    // made with new_context() belongs to Playwright and may be closed when
    // Playwright disconnects, taking the window with it.
    let contexts = browser.contexts();
    report.add(Status::Info, "contexts after attach", contexts.len().to_string());
    let context = contexts
        .into_iter()
        .next()
        .ok_or("no default context after connect_over_cdp")?;
    // Pick the about:blank tab Edge was started with, not just the first
    // tab: an extension may already have opened a welcome page.
    let pages = context.pages();
    report.add(Status::Info, "tabs at attach", pages.len().to_string());
    let page = match pages.into_iter().find(|p| p.url() == "about:blank") {
        Some(page) => {
            report.add(Status::Info, "tab", "reused the existing about:blank tab");
            page
        }
        None => {
            report.add(Status::Info, "tab", "no about:blank tab; opened one in the default context");
            context.new_page().await.map_err(|e| format!("new_page failed: {e}"))?
        }
    };

    let base = pages::serve().map_err(|e| format!("could not start the test page server: {e}"))?;
    for (check, path) in [("plain form login", "/plain"), ("controlled form login", "/controlled")] {
        match login(&page, &format!("{base}{path}")).await {
            Ok(text) => report.check(text == format!("Logged in as {}", pages::USERNAME), check, text),
            Err(e) => report.check(false, check, e),
        }
    }

    // The 2FA handoff will bring the window forward like this.
    if let Err(e) = page.bring_to_front().await {
        report.add(Status::Info, "bring_to_front", format!("failed: {e}"));
    }

    let leaked = capture.contains(pages::PASSWORD);
    report.check(
        !leaked,
        "password absent from log",
        if leaked {
            format!("password FOUND in log at level {}", options.log_level)
        } else {
            format!("not found at level {}", options.log_level)
        },
    );

    Ok((playwright, browser, base))
}

/// Fills and submits one form, then returns the text of `#result`.
async fn login(page: &Page, url: &str) -> Result<String, String> {
    page.goto(url, None).await.map_err(|e| format!("goto: {e}"))?;
    page.locator("#username").fill(pages::USERNAME, None).await.map_err(|e| format!("fill username: {e}"))?;
    page.locator("#password").fill(pages::PASSWORD, None).await.map_err(|e| format!("fill password: {e}"))?;
    page.locator("button[type=submit]").click(None).await.map_err(|e| format!("click: {e}"))?;

    let result = page.locator("#result");
    let wait = WaitForOptions::builder().state(WaitForState::Visible).timeout(10_000.0).build();
    result.wait_for(wait).await.map_err(|e| format!("no result shown: {e}"))?;
    let text = result.text_content().await.map_err(|e| format!("read result: {e}"))?;
    Ok(text.unwrap_or_default().trim().to_string())
}

async fn release(
    playwright: Playwright,
    browser: Browser,
    port: u16,
    profile_dir: &Path,
    via: ReleaseVia,
    report: &mut Report,
) {
    if via == ReleaseVia::Close {
        match browser.close().await {
            Ok(()) => report.add(Status::Info, "Browser::close", "returned ok"),
            Err(e) => report.add(Status::Info, "Browser::close", format!("error: {e}")),
        }
    }
    if let Err(e) = playwright.shutdown().await {
        report.add(Status::Info, "driver shutdown", format!("error: {e}"));
    }

    tokio::time::sleep(Duration::from_secs(2)).await;
    report.check(
        edge::debug_endpoint_alive(port),
        "released: Edge survives",
        format!("after {via:?} + driver shutdown, DevTools endpoint still answering"),
    );
    println!(
        "\nReleased mode: the spike exits now and leaves Edge open.\n\
         -> Check the Edge window is still open and you can click around in it.\n\
         -> Then close it and delete {}",
        profile_dir.display()
    );
}

async fn close_managed(
    playwright: Playwright,
    browser: Browser,
    mut child: Child,
    port: u16,
    close_after: Option<u64>,
    report: &mut Report,
) {
    match close_after {
        Some(seconds) => {
            println!("\nManaged mode: closing Edge in {seconds}s.");
            tokio::time::sleep(Duration::from_secs(seconds)).await;
        }
        None => {
            println!("\nManaged mode: press Enter to close the Edge window.");
            let _ = tokio::task::spawn_blocking(|| {
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
            })
            .await;
        }
    }

    // Ask Edge to quit cleanly first (the CDP `Browser.close` command, not
    // playwright-rs's Browser::close, which only disconnects an attached
    // browser). A clean quit saves the profile, which persistent profiles
    // need. Kill the process only if it does not exit in time.
    let graceful = match browser.new_browser_cdp_session().await {
        // The connection drops as Edge quits, so an error here is normal.
        Ok(session) => {
            let _ = session.send("Browser.close", None).await;
            tokio::task::block_in_place(|| edge::wait_for_exit(&mut child, Duration::from_secs(10)))
        }
        Err(_) => false,
    };
    if graceful {
        report.check(true, "managed: clean quit", "Edge exited after CDP Browser.close");
    } else {
        report.check(false, "managed: clean quit", "Edge did not exit within 10s; killed it");
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = playwright.shutdown().await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    report.check(!edge::debug_endpoint_alive(port), "managed: Edge closed", "DevTools endpoint no longer answering");
}
