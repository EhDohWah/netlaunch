//! `probe`: opens a real login page in Edge, the way NetLaunch does, and
//! prints what a recipe needs to know about it. The spec says a site's
//! selectors must be read from its real login page, never guessed; this
//! is how.
//!
//! For each snapshot it prints the visible fields (type, id, name,
//! autocomplete...), buttons with their labels, headings and alerts,
//! frames, and whether the built-in recipes' selectors and success rule
//! match. **It never reads what's typed in a field.**
//!
//! You log in by hand in the window, pressing Enter here at each page
//! (the login page, a 2FA page, the dashboard) to take a snapshot.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use netlaunch_driver_pw::{LaunchOptions, Profile, Session};
use netlaunch_engine::recipe::{Rule, Step};
use netlaunch_engine::{BrowserDriver, Recipe, builtin};

const USAGE: &str = "\
NetLaunch probe: print a login page's form, to write a recipe for it

USAGE:
    probe <login URL> [--self-signed] [--browser PATH] [--headless] [--once]

    --self-signed   accept the site's self-signed certificate
    --browser PATH  browser to use (default: the installed Edge)
    --headless      no window (for testing the probe itself)
    --once          take the first snapshot and exit

Log in by hand in the Edge window. Press Enter here on each page (2FA,
dashboard) to print it; type q and Enter to finish. Nothing typed into the
page is read or printed.
";

struct Options {
    url: String,
    self_signed: bool,
    browser: Option<PathBuf>,
    headless: bool,
    once: bool,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options { url: String::new(), self_signed: false, browser: None, headless: false, once: false };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--self-signed" => options.self_signed = true,
            "--headless" => options.headless = true,
            "--once" => options.once = true,
            "--browser" => options.browser = Some(PathBuf::from(args.next().ok_or("--browser needs a path")?)),
            "-h" | "--help" | "help" => return Err(String::new()),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            url if options.url.is_empty() => options.url = url.to_string(),
            extra => return Err(format!("unexpected argument {extra}")),
        }
    }
    if options.url.is_empty() {
        return Err("give the login page URL".into());
    }
    Ok(options)
}

fn main() -> ExitCode {
    // Same as the app: use the installer's driver if there is one. Nothing
    // else runs yet.
    // SAFETY: no other thread exists at this point.
    unsafe { netlaunch_driver_pw::use_bundled_driver() };

    let options = match parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("error: {message}\n");
            }
            eprint!("{USAGE}");
            return if message.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE };
        }
    };
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match runtime.block_on(probe(options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Every selector the built-in recipes use, to report which ones match.
fn recipe_selectors(recipes: &[Recipe]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for recipe in recipes {
        for step in &recipe.steps {
            match step {
                Step::Goto {} => {}
                Step::Wait { selector, .. }
                | Step::Fill { selector, .. }
                | Step::Click { selector }
                | Step::Select { selector, .. }
                | Step::Press { selector, .. } => {
                    set.insert(selector.clone());
                }
            }
        }
        for rule in recipe.success.any.iter().chain(&recipe.mfa.any).chain(&recipe.failure.any) {
            if let Rule::Selector(selector) = rule {
                set.insert(selector.clone());
            }
        }
    }
    set.into_iter().collect()
}

async fn probe(options: Options) -> Result<(), String> {
    let browser = options
        .browser
        .clone()
        .or_else(netlaunch_driver_pw::find_edge)
        .ok_or("Edge not found; pass --browser <path to msedge.exe>")?;
    let recipes: Vec<Recipe> = netlaunch_engine::builtin_list().iter().filter_map(|(id, _)| builtin(id)).collect();
    let selectors = recipe_selectors(&recipes);

    let mut extra_args = Vec::new();
    if cfg!(target_os = "linux") && options.headless {
        extra_args.push("--no-sandbox".to_string());
    }
    let session = Session::launch(LaunchOptions {
        browser,
        profile: Profile::Fresh,
        accept_self_signed: options.self_signed,
        headless: options.headless,
        extra_args,
    })
    .await?;

    let result = run(&session, &options, &recipes, &selectors).await;
    session.close().await;
    result
}

async fn run(session: &Session, options: &Options, recipes: &[Recipe], selectors: &[String]) -> Result<(), String> {
    if let Err(e) = session.goto(&options.url).await {
        let hint = if e.to_string().contains("ERR_CERT") { " (add --self-signed for a self-signed certificate)" } else { "" };
        return Err(format!("the page didn't load: {e}{hint}"));
    }
    // Give a single-page app time to draw its form.
    let _ = session.wait_visible("input", 15_000).await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!("NetLaunch probe. Paste everything between the ===== lines back to Claude.");
    println!("=====");
    println!("login URL given: {}", options.url);
    let mut number = 1;
    loop {
        print_snapshot(session, number, &options.url, recipes, selectors).await;
        if options.once {
            break;
        }
        println!("\n-> Log in by hand in the Edge window. Press Enter here on each new page");
        println!("   (2FA page, dashboard) to print it. Type q and Enter when you're done.");
        let line = tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).map(|_| line)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        if line.trim().eq_ignore_ascii_case("q") || line.is_empty() {
            break;
        }
        number += 1;
    }
    println!("=====");
    Ok(())
}

async fn print_snapshot(session: &Session, number: usize, login_url: &str, recipes: &[Recipe], selectors: &[String]) {
    println!("\n--- snapshot {number} ---");
    let snap = match session.snapshot(selectors).await {
        Ok(snap) => snap,
        Err(e) => {
            println!("couldn't read the page: {e}");
            return;
        }
    };
    let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string();
    println!("page: {}", text(&snap, "url"));
    println!("title: {}", text(&snap, "title"));

    let section = |name: &str| snap.get(name).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for (name, heading) in [("fields", "visible fields"), ("buttons", "buttons"), ("headings", "headings and alerts")] {
        let items = section(name);
        println!("{heading}: {}", if items.is_empty() { "none" } else { "" });
        for item in items {
            // Compact `key=value` pairs, in a stable order.
            let pairs: Vec<String> = item
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| format!("{k}={}", v.as_str().map(|s| format!("{s:?}")).unwrap_or_else(|| v.to_string())))
                .collect();
            println!("  - {}", pairs.join(" "));
        }
    }
    let frames = section("frames");
    if !frames.is_empty() {
        println!("frames: {}", frames.iter().map(|f| f.as_str().unwrap_or("?")).collect::<Vec<_>>().join(", "));
    }

    println!("recipe selectors (visible matches):");
    if let Some(matches) = snap.get("matches").and_then(|v| v.as_object()) {
        for (selector, count) in matches {
            println!("  {count}  {selector}");
        }
    }
    for recipe in recipes {
        let logged_in = match netlaunch_engine::logged_in(session, recipe, login_url).await {
            Ok(true) => "yes",
            Ok(false) => "no",
            Err(_) => "window closed",
        };
        println!("success rule of {}: {logged_in}", recipe.id);
    }
}
