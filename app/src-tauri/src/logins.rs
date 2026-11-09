//! What the Open button and the "Open windows" panel do (M5, M6).
//!
//! The work happens in `netlaunch-sessions`, which launches Edge, runs the
//! login, watches 2FA hand-overs and releases or closes windows. This file
//! connects it to the app:
//!
//! - [`open_site`] reads the site from the vault, records the attempt in
//!   `login_log` and builds the request. The password is read from the
//!   vault only when the engine fills the password field: that closure is
//!   the one place the app calls `reveal_password`.
//! - [`TauriHooks`] passes the sessions' news on: `session-changed` and
//!   `session-ended` events for the panel, and results for `login_log`.
//!
//! Neither the events nor the command results ever carry a password.

use std::sync::Arc;

use netlaunch_driver_pw::{LaunchOptions, Profile};
use netlaunch_sessions::{Ended, Hooks, OpenRequest, OpenResult, SessionView, Sessions};
use netlaunch_vault::{LoginOutcome, ProfileMode};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::SharedVault;

/// Sends session news to the window and login results to the vault.
pub struct TauriHooks {
    pub app: AppHandle,
    pub vault: SharedVault,
}

/// The `session-ended` event.
#[derive(Clone, Serialize)]
struct EndedEvent {
    site_id: i64,
    why: Ended,
}

impl Hooks for TauriHooks {
    fn changed(&self, view: &SessionView) {
        let _ = self.app.emit("session-changed", view);
    }

    fn ended(&self, site_id: i64, why: Ended) {
        let _ = self.app.emit("session-ended", EndedEvent { site_id, why });
    }

    fn record(&self, log_id: i64, outcome: LoginOutcome, message: &str) {
        if let Err(e) = self.vault.with(|vault| vault.finish_login(log_id, outcome, Some(message))) {
            tracing::warn!("couldn't record the login result: {e}");
        }
    }
}

/// The Open button. Resolves when the login has finished; a 2FA hand-over
/// is then watched in the background.
pub async fn open_site(vault: &SharedVault, sessions: &Sessions, site_id: i64) -> Result<OpenResult, String> {
    if sessions.list().iter().any(|view| view.site_id == site_id) {
        return Err("This site's window is already open. Use Close or Retry under Open windows.".into());
    }
    let site = vault.with(|v| v.get_site(site_id))?;
    let recipe = netlaunch_engine::builtin(&site.recipe_id)
        .ok_or_else(|| format!("This site uses an unknown login recipe '{}'. Edit it and pick one.", site.recipe_id))?;
    let browser = netlaunch_driver_pw::find_edge()
        .ok_or("Microsoft Edge wasn't found on this PC. NetLaunch opens sites in Edge.")?;
    let profile = match site.profile_mode {
        ProfileMode::Fresh => Profile::Fresh,
        ProfileMode::Persistent => Profile::Persistent(
            netlaunch_driver_pw::persistent_profile_dir(site_id)
                .ok_or("LOCALAPPDATA isn't set, so there's nowhere to keep this site's profile.")?,
        ),
    };

    let log_id = vault.with(|v| v.start_login(site_id))?;
    let request = OpenRequest {
        site_id,
        site_name: site.name.clone(),
        login_url: site.login_url.clone(),
        username: site.username.clone(),
        recipe,
        window_mode: site.window_mode,
        launch: LaunchOptions {
            browser,
            profile,
            accept_self_signed: site.allow_self_signed,
            headless: false,
            extra_args: Vec::new(),
        },
        log_id,
    };
    // The only call to reveal_password in the app, made at fill time.
    let password_vault = vault.clone();
    let password = Box::new(move || password_vault.with(|v| v.reveal_password(site_id)));

    match sessions.open(request, password).await {
        Ok(result) => Ok(result),
        Err(message) => {
            // Refused before anything started (already open).
            let _ = vault.with(|v| v.finish_login(log_id, LoginOutcome::Failed, Some(&message)));
            Err(message)
        }
    }
}

/// Builds the app's session list, sending its news through `hooks`.
pub fn new_sessions(hooks: TauriHooks) -> Sessions {
    Sessions::new(Arc::new(hooks), netlaunch_sessions::Settings::default())
}
