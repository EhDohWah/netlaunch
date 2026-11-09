//! NetLaunch desktop app: the Tauri shell around the vault (M4), the login
//! engine (M5) and the open windows (M6).
//!
//! The commands below are the frontend's only way into Rust. None of them
//! returns a password; see `sites.rs`, `logins.rs` and the rules in
//! CLAUDE.md.

// In release builds, don't open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod logins;
mod sites;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use netlaunch_sessions::{OpenResult, SessionView, Sessions};
use netlaunch_vault::{Site, Vault, VaultError};
use serde::Serialize;
use tauri::{Manager, RunEvent, State};

/// The vault, opened once at start-up. If it couldn't be opened, the error
/// is kept so every command (and the banner in the UI) can report it.
///
/// Shared (`Arc`) because the window watchers record 2FA results in the
/// login history from background tasks.
#[derive(Clone)]
pub struct SharedVault(Arc<Mutex<Result<Vault, String>>>);

impl SharedVault {
    /// Runs `f` with the vault, turning errors into strings for the UI.
    /// `VaultError` messages never contain a password. The lock is held
    /// only while `f` runs, never across an `await`.
    pub fn with<T>(&self, f: impl FnOnce(&mut Vault) -> Result<T, VaultError>) -> Result<T, String> {
        let mut guard = self.0.lock().map_err(|_| "the vault is unavailable after an earlier error".to_string())?;
        match guard.as_mut() {
            Ok(vault) => f(vault).map_err(|e| e.to_string()),
            Err(message) => Err(message.clone()),
        }
    }

    fn error(&self) -> Option<String> {
        self.0.lock().ok().and_then(|guard| guard.as_ref().err().cloned())
    }
}

struct AppState {
    vault: SharedVault,
    path: Option<PathBuf>,
    /// The browser windows logins opened.
    sessions: Sessions,
}

/// Opens `%LOCALAPPDATA%\NetLaunch\vault.db` with its key protected by
/// DPAPI.
#[cfg(windows)]
fn open_vault() -> (Result<Vault, String>, Option<PathBuf>) {
    let Some(path) = netlaunch_vault::default_path() else {
        return (Err("LOCALAPPDATA is not set, so there's nowhere to keep the vault".into()), None);
    };
    let vault = Vault::open(&path, &netlaunch_vault::Dpapi).map_err(|e| e.to_string());
    (vault, Some(path))
}

/// The vault's key is protected with Windows DPAPI, so the app only works
/// on Windows. Elsewhere it still builds and starts, and says why it can't
/// open the vault.
#[cfg(not(windows))]
fn open_vault() -> (Result<Vault, String>, Option<PathBuf>) {
    (Err("NetLaunch runs on Windows only: the vault key is protected with Windows DPAPI.".into()), None)
}

#[derive(Serialize)]
struct VaultStatus {
    ok: bool,
    path: Option<String>,
    error: Option<String>,
}

#[tauri::command]
fn vault_status(state: State<'_, AppState>) -> VaultStatus {
    let error = state.vault.error();
    VaultStatus {
        ok: error.is_none(),
        path: state.path.as_ref().map(|p| p.display().to_string()),
        error,
    }
}

#[tauri::command]
fn list_sites(state: State<'_, AppState>) -> Result<Vec<Site>, String> {
    state.vault.with(|vault| vault.list_sites())
}

#[tauri::command]
fn add_site(state: State<'_, AppState>, site: sites::SiteInput) -> Result<Site, String> {
    state.vault.with(|vault| sites::add(vault, site))
}

#[tauri::command]
fn update_site(state: State<'_, AppState>, id: i64, site: sites::SiteInput) -> Result<Site, String> {
    state.vault.with(|vault| sites::update(vault, id, site))
}

/// Deletes a site, closing its window first if one is open.
#[tauri::command]
async fn delete_site(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    if state.sessions.list().iter().any(|view| view.site_id == id) {
        state.sessions.close(id).await?;
    }
    state.vault.with(|vault| vault.delete_site(id))
}

/// The "Trust this certificate" button: ticks the site's self-signed
/// setting. The frontend then opens the site again.
#[tauri::command]
fn trust_certificate(state: State<'_, AppState>, id: i64) -> Result<Site, String> {
    state.vault.with(|vault| sites::trust_certificate(vault, id))
}

#[derive(Serialize)]
struct RecipeInfo {
    id: String,
    name: String,
}

/// The built-in login recipes, for the recipe dropdown.
#[tauri::command]
fn list_recipes() -> Vec<RecipeInfo> {
    netlaunch_engine::builtin_list().into_iter().map(|(id, name)| RecipeInfo { id, name }).collect()
}

/// The Open button: launches Edge and logs in. Progress arrives as
/// `session-changed` events; this resolves with the result.
///
/// `async`, so the login (up to a minute) runs without freezing the window.
#[tauri::command]
async fn open_site(state: State<'_, AppState>, id: i64) -> Result<OpenResult, String> {
    logins::open_site(&state.vault, &state.sessions, id).await
}

/// The "Open windows" panel's rows.
#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Vec<SessionView> {
    state.sessions.list()
}

/// Release: hand the window over; Edge keeps running without NetLaunch.
#[tauri::command]
async fn release_window(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.sessions.release(id).await
}

/// Close: close the window (and delete a fresh profile).
#[tauri::command]
async fn close_window(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.sessions.close(id).await
}

/// The log file: `%LOCALAPPDATA%\NetLaunch\logs\netlaunch.log`. When it
/// passes 5 MB it's renamed to `netlaunch.old.log` (replacing the older
/// one) at the next start.
fn log_file() -> Option<std::fs::File> {
    let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("NetLaunch").join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("netlaunch.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
        let _ = std::fs::rename(&path, dir.join("netlaunch.old.log"));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Logs go to the log file and to stderr (the `cargo tauri dev` console).
/// The level is set here, never from `RUST_LOG`, and playwright-rs is
/// capped below `trace`, where it would log the password being typed.
fn init_logging() {
    use tracing_subscriber::fmt::writer::MakeWriterExt;

    let filter = tracing_subscriber::EnvFilter::new(netlaunch_engine::log_directives("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false);
    let _ = match log_file() {
        Some(file) => builder.with_writer(std::io::stderr.and(Mutex::new(file))).try_init(),
        None => builder.try_init(),
    };
}

/// `netlaunch --self-check <report file>`: checks the installed app can
/// drive Edge, without opening its window. Launches Edge headless through
/// the Playwright driver (the installer's copy when present), attaches,
/// closes it, and writes a short report. CI runs it on the installed app.
fn self_check(report: &std::path::Path, bundled: Option<&std::path::Path>) -> std::process::ExitCode {
    let mut lines = vec![format!(
        "bundled driver: {}",
        bundled.map(|d| d.display().to_string()).unwrap_or_else(|| "none".into())
    )];
    let result = tauri::async_runtime::block_on(async {
        let browser = netlaunch_driver_pw::find_edge().ok_or("Edge not found")?;
        lines.push(format!("edge: {}", browser.display()));
        let session = netlaunch_driver_pw::Session::launch(netlaunch_driver_pw::LaunchOptions {
            browser,
            profile: netlaunch_driver_pw::Profile::Fresh,
            accept_self_signed: false,
            headless: true,
            extra_args: Vec::new(),
        })
        .await?;
        let clean = session.close().await;
        Ok::<bool, String>(clean)
    });
    let ok = match result {
        Ok(clean) => {
            lines.push(format!("launched and attached; clean quit: {clean}"));
            lines.push("OK".into());
            true
        }
        Err(e) => {
            lines.push(format!("FAILED: {e}"));
            false
        }
    };
    let _ = std::fs::write(report, lines.join("\n") + "\n");
    if ok { std::process::ExitCode::SUCCESS } else { std::process::ExitCode::FAILURE }
}

fn main() -> std::process::ExitCode {
    // Use the Playwright driver the installer put next to the exe. First
    // thing, while no other thread exists.
    // SAFETY: nothing else is running yet.
    let bundled = unsafe { netlaunch_driver_pw::use_bundled_driver() };

    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--self-check") {
        let report = args.get(i + 1).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("netlaunch-self-check.txt"));
        return self_check(&report, bundled.as_deref());
    }

    init_logging();
    tracing::info!(bundled_driver = ?bundled, "NetLaunch starting");
    // Fresh profiles of windows released earlier, and since closed.
    let swept = netlaunch_driver_pw::sweep_fresh_profiles(std::time::Duration::from_secs(10 * 60));
    if swept > 0 {
        tracing::info!(swept, "deleted leftover fresh profile folders");
    }
    let (vault, path) = open_vault();
    let vault = SharedVault(Arc::new(Mutex::new(vault)));

    let app = tauri::Builder::default()
        // The sessions send events, so they're made once the app exists.
        .setup(move |app| {
            let hooks = logins::TauriHooks { app: app.handle().clone(), vault: vault.clone() };
            app.manage(AppState { vault, path, sessions: logins::new_sessions(hooks) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_status,
            list_sites,
            add_site,
            update_site,
            delete_site,
            trust_certificate,
            list_recipes,
            open_site,
            list_sessions,
            release_window,
            close_window
        ])
        .build(tauri::generate_context!())
        .expect("error while building NetLaunch");

    app.run(|handle, event| {
        // Managed windows close with the app (spec: "Closing the app
        // closes the window"). Released windows aren't in the list.
        if let RunEvent::Exit = event {
            let sessions = handle.state::<AppState>().sessions.clone();
            tauri::async_runtime::block_on(sessions.close_all());
        }
    });
    std::process::ExitCode::SUCCESS
}
