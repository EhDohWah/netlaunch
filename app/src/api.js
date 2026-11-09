// The frontend's only way into Rust: the commands registered in
// src-tauri/src/main.rs. None of them returns a password.
import { invoke } from '@tauri-apps/api/core';

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
