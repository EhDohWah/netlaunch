// The frontend's only way into Rust: the commands registered in
// src-tauri/src/main.rs. None of them returns a password.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/** @returns {Promise<{ok: boolean, path: string|null, error: string|null}>} */
export const vaultStatus = () => invoke('vault_status');

export const listSites = () => invoke('list_sites');

/**
 * Saves a new site. `site.password` is required.
 * Resolves to the saved site (without its password).
 */
export const addSite = (site) => invoke('add_site', { site });

/**
 * Saves changes. An empty or null `site.password` keeps the stored one.
 * Resolves to the saved site (without its password).
 */
export const updateSite = (id, site) => invoke('update_site', { id, site });

export const deleteSite = (id) => invoke('delete_site', { id });

/** @returns {Promise<{id: string, name: string, needs_username: boolean}[]>} the built-in login recipes */
export const listRecipes = () => invoke('list_recipes');

/**
 * Launches the browser and logs in to the site. Resolves when the login has
 * finished, one way or another: `{ outcome, message, warnings, fix }`,
 * where `outcome` is one of
 * success | mfa_handoff | failed | timeout | origin_mismatch.
 * A 2FA hand-over is then watched in the background (see onSessionChanged).
 */
export const openSite = (id) => invoke('open_site', { id });

/**
 * The windows NetLaunch is looking after: SessionView[]
 * `{ site_id, site_name, state, message, warnings, window_mode }`.
 */
export const listSessions = () => invoke('list_sessions');

/** Hands the window over: Edge keeps running without NetLaunch. */
export const releaseWindow = (id) => invoke('release_window', { id });

/** Closes the window (and deletes a fresh profile). */
export const closeWindow = (id) => invoke('close_window', { id });

/** Calls `handler(view)` when a window's state changes or a window opens. */
export const onSessionChanged = (handler) => listen('session-changed', (event) => handler(event.payload));

/**
 * Calls `handler({ site_id, why })` when a window leaves the list. `why` is
 * closed | released | window_closed | no_window.
 */
export const onSessionEnded = (handler) => listen('session-ended', (event) => handler(event.payload));

/**
 * Ticks the site's "self-signed certificate" setting (after Edge rejected
 * its certificate). Resolves to the saved site.
 */
export const trustCertificate = (id) => invoke('trust_certificate', { id });


/**
 * Settings and the browsers found on this PC:
 * `{ settings: { browser, browser_path, check_updates }, browsers: [{ id, name, path }], version, can_update }`.
 * `browser` is edge | chrome | custom.
 */
export const getSettings = () => invoke('get_settings');

/** Saves the settings; rejects with a message if the browser isn't there. */
export const saveSettings = (settings) => invoke('save_settings', { settings });

/**
 * Asks GitHub for a newer release:
 * `{ current, can_update, available: { version, notes } | null }`.
 * `can_update` is false for development builds.
 */
export const checkUpdate = () => invoke('check_update');

/**
 * Downloads and installs the newer version. The app then restarts, so
 * this only settles when something went wrong.
 */
export const installUpdate = () => invoke('install_update');

/** Calls `handler({ downloaded, total })` (bytes) while an update downloads. */
export const onUpdateProgress = (handler) => listen('update-progress', (event) => handler(event.payload));
