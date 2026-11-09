//! NetLaunch desktop app: the Tauri shell around the vault (M4) and the
//! login engine (M5).
//!
//! The commands below are the frontend's only way into Rust. None of them
//! returns a password; see `sites.rs`, `logins.rs` and the rules in
//! CLAUDE.md.

// In release builds, don't open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod logins;
mod sites;

use std::path::PathBuf;
use std::sync::Mutex;

use netlaunch_vault::{Site, Vault, VaultError};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

/// The vault, opened once at start-up. If it couldn't be opened, the error
/// is kept so every command (and the banner in the UI) can report it.
struct AppState {
    vault: Mutex<Result<Vault, String>>,
    path: Option<PathBuf>,
    /// Browser windows opened by logins.
    sessions: logins::Sessions,
}

impl AppState {
    /// Runs `f` with the vault, turning errors into strings for the UI.
    /// `VaultError` messages never contain a password.
    fn with_vault<T>(&self, f: impl FnOnce(&mut Vault) -> Result<T, VaultError>) -> Result<T, String> {
        let mut guard = self.vault.lock().map_err(|_| "the vault is unavailable after an earlier error".to_string())?;
        match guard.as_mut() {
            Ok(vault) => f(vault).map_err(|e| e.to_string()),
            Err(message) => Err(message.clone()),
        }
    }
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
    let error = state.vault.lock().ok().and_then(|guard| guard.as_ref().err().cloned());
    VaultStatus {
        ok: error.is_none(),
        path: state.path.as_ref().map(|p| p.display().to_string()),
        error,
    }
}

#[tauri::command]
fn list_sites(state: State<'_, AppState>) -> Result<Vec<Site>, String> {
    state.with_vault(|vault| vault.list_sites())
}

#[tauri::command]
fn add_site(state: State<'_, AppState>, site: sites::SiteInput) -> Result<Site, String> {
    state.with_vault(|vault| sites::add(vault, site))
}

#[tauri::command]
fn update_site(state: State<'_, AppState>, id: i64, site: sites::SiteInput) -> Result<Site, String> {
    state.with_vault(|vault| sites::update(vault, id, site))
}

#[tauri::command]
fn delete_site(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.with_vault(|vault| vault.delete_site(id))
}

/// The "Trust this certificate" button: ticks the site's self-signed
/// setting. The frontend then opens the site again.
#[tauri::command]
fn trust_certificate(state: State<'_, AppState>, id: i64) -> Result<Site, String> {
    state.with_vault(|vault| sites::trust_certificate(vault, id))
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
/// `login-progress` events; this resolves with the result.
///
/// `async`, so the login (up to a minute) runs without freezing the window.
#[tauri::command]
async fn open_site(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<logins::OpenResult, String> {
    logins::open_site(&state, id, move |event| {
        let _ = app.emit("login-progress", event);
    })
    .await
}

/// Logs go to stderr (the `cargo tauri dev` console). The level is set
/// here, never from `RUST_LOG`, and playwright-rs is capped below `trace`,
/// where it would log the password being typed.
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::new(netlaunch_engine::log_directives("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).try_init();
}

fn main() {
    init_logging();
    let (vault, path) = open_vault();
    let app = tauri::Builder::default()
        .manage(AppState { vault: Mutex::new(vault), path, sessions: logins::Sessions::default() })
        .invoke_handler(tauri::generate_handler![
            vault_status,
            list_sites,
            add_site,
            update_site,
            delete_site,
            list_recipes,
            open_site,
            trust_certificate
        ])
        .build(tauri::generate_context!())
        .expect("error while building NetLaunch");

    app.run(|handle, event| {
        // Managed windows close with the app (spec: "Closing the app
        // closes the window"). Released windows aren't in the list.
        if let RunEvent::Exit = event {
            let sessions = handle.state::<AppState>().sessions.take_all();
            if !sessions.is_empty() {
                tauri::async_runtime::block_on(async {
                    for session in sessions {
                        session.close().await;
                    }
                });
            }
        }
    });
}
