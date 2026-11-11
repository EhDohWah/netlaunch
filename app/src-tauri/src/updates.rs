//! In-app updates (Settings > Updates).
//!
//! The Release workflow (`.github/workflows/release.yml`) builds the
//! installer, signs it with the owner's private key and publishes it as a
//! GitHub release, with a `latest.json` that names the version. The app
//! reads that file, and installs a newer version only if its signature
//! matches the public key built into this copy of the app. So a file
//! swapped on the way, or a release made without the key, is refused.
//!
//! The work is done by `tauri-plugin-updater`, called from Rust only: the
//! window gets no updater permissions (capabilities stay `core:default`).

use netlaunch_sessions::Sessions;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// What Settings > Updates shows.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    /// This copy's version.
    pub current: String,
    /// False for builds without a public key (development builds): they
    /// can't check signatures, so they don't update themselves.
    pub can_update: bool,
    /// The newer version, when there is one.
    pub available: Option<Available>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Available {
    pub version: String,
    /// The release's description on GitHub.
    pub notes: Option<String>,
}

/// The `update-progress` event, sent while the installer downloads.
#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

/// True if this build carries the public key that signed releases are
/// checked against (`plugins.updater.pubkey`, set by the Release workflow).
pub fn can_update(app: &AppHandle) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("pubkey"))
        .and_then(|key| key.as_str())
        .is_some_and(|key| !key.trim().is_empty())
}

fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Asks GitHub whether a newer release exists.
pub async fn check(app: &AppHandle) -> Result<UpdateStatus, String> {
    let current = current_version(app);
    if !can_update(app) {
        return Ok(UpdateStatus { current, can_update: false, available: None });
    }
    let update = find(app).await?;
    Ok(UpdateStatus {
        current,
        can_update: true,
        available: update.map(|u| Available { version: u.version.clone(), notes: u.body.clone() }),
    })
}

async fn find(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let updater = app.updater().map_err(|e| format!("The updater couldn't start: {e}"))?;
    updater.check().await.map_err(|e| {
        format!("Couldn't check for updates: {e}. NetLaunch needs to reach github.com for this.")
    })
}

/// Downloads the newer version, checks its signature, closes the windows
/// NetLaunch manages and runs the installer. On Windows the app exits
/// here and the installer starts it again, so this only returns on
/// failure (or when there's nothing to install).
pub async fn install(app: &AppHandle, sessions: &Sessions) -> Result<(), String> {
    if !can_update(app) {
        return Err("This copy of NetLaunch can't update itself. Install a released version first.".into());
    }
    let update = find(app).await?.ok_or("NetLaunch is already up to date.")?;
    tracing::info!(version = %update.version, "downloading an update");

    let mut downloaded = 0u64;
    let progress_app = app.clone();
    // `download` also checks the signature; a bad one is an error.
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit("update-progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| format!("The update couldn't be downloaded or didn't pass its signature check: {e}"))?;

    // The app is about to exit without the usual shutdown, so close the
    // managed windows now (released ones aren't in the list).
    sessions.close_all().await;
    tracing::info!(version = %update.version, "installing the update");
    update.install(bytes).map_err(|e| format!("The update couldn't be installed: {e}"))
}
