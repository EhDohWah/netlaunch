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

/** @returns {Promise<{id: string, name: string}[]>} the built-in login recipes */
export const listRecipes = () => invoke('list_recipes');

/**
 * Launches Edge and logs in to the site. Resolves when the login has
 * finished, one way or another:
 * `{ outcome, message, warnings }`, where `outcome` is one of
 * success | mfa_handoff | failed | timeout | origin_mismatch.
 */
export const openSite = (id) => invoke('open_site', { id });

/**
 * Ticks the site's "self-signed certificate" setting (after Edge rejected
 * its certificate). Resolves to the saved site.
 */
export const trustCertificate = (id) => invoke('trust_certificate', { id });

/**
 * Calls `handler({ site_id, stage })` as logins progress. `stage` is
 * launching | loading | filling | waiting. Resolves to an unlisten function.
 */
export const onLoginProgress = (handler) => listen('login-progress', (event) => handler(event.payload));
