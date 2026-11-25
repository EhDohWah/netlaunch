//! The `Vault`: opens the SQLite file, unlocks the data key, and reads and
//! writes sites.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use secrecy::{ExposeSecret, SecretString};

use crate::crypto::{self, DataKey};
use crate::model::{LoginOutcome, LoginRecord, NewSite, ProfileMode, Site, SiteUpdate, WindowMode};
use crate::origin::normalize_origin;
use crate::protect::KeyProtector;
use crate::schema;
use crate::{Result, VaultError};

/// `%LOCALAPPDATA%\NetLaunch\vault.db`, or `None` if `LOCALAPPDATA` isn't
/// set (it always is on Windows).
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("NetLaunch").join("vault.db"))
}

/// Current UTC time as ISO 8601 with milliseconds, computed by SQLite so
/// the crate needs no date library.
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

const KEY_VERSION: &str = "1";

pub struct Vault {
    pub(crate) conn: Connection,
    pub(crate) key: DataKey,
}

impl Vault {
    /// Opens (or creates) the vault file, brings its schema up to date and
    /// unlocks the data key with `protector`. A new vault gets a new random
    /// key, wrapped by `protector`.
    pub fn open(path: &Path, protector: &dyn KeyProtector) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        Self::setup(Connection::open(path)?, protector)
    }

    /// A vault that lives only in memory. For tests of code that uses the
    /// vault (the login engine), not for the app.
    pub fn open_in_memory(protector: &dyn KeyProtector) -> Result<Self> {
        Self::setup(Connection::open_in_memory()?, protector)
    }

    fn setup(mut conn: Connection, protector: &dyn KeyProtector) -> Result<Self> {
        // foreign_keys: SQLite ignores REFERENCES unless this is on, and the
        //   login_log cascade depends on it.
        // secure_delete: overwrite deleted and replaced data with zeros, so
        //   old ciphertexts don't linger in free pages of the file.
        // journal_mode=DELETE: one file on disk, plus a short-lived journal.
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA secure_delete = ON;
             PRAGMA journal_mode = DELETE;",
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&mut conn)?;
        let key = load_or_create_key(&mut conn, protector)?;
        Ok(Self { conn, key })
    }

    // ----- sites -----

    /// Saves a new site and returns its id.
    pub fn add_site(&mut self, site: NewSite) -> Result<i64> {
        let origin = validate(&site.name, &site.login_url, &site.username, &site.recipe_id)?;
        if site.password.expose_secret().is_empty() {
            return Err(VaultError::InvalidInput("password is empty".into()));
        }

        // The associated data includes the row id, which only exists after
        // the insert. So: insert with an empty ciphertext, encrypt, update,
        // all in one transaction so the empty value is never committed.
        let tx = self.conn.transaction()?;
        let group = settle_group(&tx, site.group_name, None)?;
        tx.execute(
            &format!(
                "INSERT INTO sites (name, group_name, login_url, origin, username, password_ct, password_nonce,
                                    recipe_id, allow_self_signed, profile_mode, window_mode, notes,
                                    created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, x'', x'', ?6, ?7, ?8, ?9, ?10, {NOW}, {NOW})"
            ),
            params![
                site.name.trim(),
                group,
                site.login_url.trim(),
                origin,
                site.username,
                site.recipe_id,
                site.allow_self_signed,
                site.profile_mode.as_str(),
                site.window_mode.as_str(),
                clean(site.notes),
            ],
        )?;
        let id = tx.last_insert_rowid();
        let (ciphertext, nonce) = crypto::encrypt(&self.key, &crypto::password_aad(id), &site.password)?;
        tx.execute(
            "UPDATE sites SET password_ct = ?1, password_nonce = ?2 WHERE id = ?3",
            params![ciphertext, nonce.as_slice(), id],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Changes a site. `update.password: None` keeps the stored password;
    /// `Some` replaces it (with a new nonce).
    pub fn update_site(&mut self, id: i64, update: SiteUpdate) -> Result<()> {
        let origin = validate(&update.name, &update.login_url, &update.username, &update.recipe_id)?;
        let new_password = match &update.password {
            Some(p) if p.expose_secret().is_empty() => return Err(VaultError::InvalidInput("password is empty".into())),
            Some(p) => Some(crypto::encrypt(&self.key, &crypto::password_aad(id), p)?),
            None => None,
        };

        let tx = self.conn.transaction()?;
        let group = settle_group(&tx, update.group_name, Some(id))?;
        let changed = tx.execute(
            &format!(
                "UPDATE sites SET name = ?1, group_name = ?2, login_url = ?3, origin = ?4, username = ?5,
                                  recipe_id = ?6, allow_self_signed = ?7, profile_mode = ?8, window_mode = ?9,
                                  notes = ?10, updated_at = {NOW}
                 WHERE id = ?11"
            ),
            params![
                update.name.trim(),
                group,
                update.login_url.trim(),
                origin,
                update.username,
                update.recipe_id,
                update.allow_self_signed,
                update.profile_mode.as_str(),
                update.window_mode.as_str(),
                clean(update.notes),
                id,
            ],
        )?;
        if changed == 0 {
            return Err(VaultError::NotFound(id));
        }
        if let Some((ciphertext, nonce)) = new_password {
            tx.execute(
                "UPDATE sites SET password_ct = ?1, password_nonce = ?2 WHERE id = ?3",
                params![ciphertext, nonce.as_slice(), id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes a site and (by cascade) its login history.
    pub fn delete_site(&mut self, id: i64) -> Result<()> {
        match self.conn.execute("DELETE FROM sites WHERE id = ?1", params![id])? {
            0 => Err(VaultError::NotFound(id)),
            _ => Ok(()),
        }
    }

    // ----- groups -----
    //
    // A group is the `group_name` label its sites share, not a row of its
    // own. So a group exists while at least one site carries its name,
    // and renaming or removing one means changing that label on every
    // site in it. Both do so in one transaction: all of the sites change
    // or none do.

    /// Renames a group and returns how many sites moved. If another group
    /// already has the new name (ignoring capitals), the two become one,
    /// under that group's spelling.
    pub fn rename_group(&mut self, from: &str, to: &str) -> Result<usize> {
        let to = to.trim();
        check_group_name(to)?;
        let tx = self.conn.transaction()?;
        let other: Option<String> = tx
            .query_row(
                "SELECT group_name FROM sites
                 WHERE group_name = ?1 COLLATE NOCASE AND group_name <> ?2 COLLATE NOCASE
                 ORDER BY id LIMIT 1",
                params![to, from],
                |row| row.get(0),
            )
            .optional()?;
        let target = other.as_deref().unwrap_or(to);
        let moved = tx.execute(
            &format!("UPDATE sites SET group_name = ?1, updated_at = {NOW} WHERE group_name = ?2 COLLATE NOCASE"),
            params![target, from],
        )?;
        if moved == 0 {
            return Err(VaultError::InvalidInput(format!("there is no group called \"{from}\"")));
        }
        tx.commit()?;
        Ok(moved)
    }

    /// Removes a group: its sites are kept and have no group afterwards.
    /// Returns how many sites that was.
    pub fn remove_group(&mut self, name: &str) -> Result<usize> {
        let moved = self.conn.execute(
            &format!("UPDATE sites SET group_name = NULL, updated_at = {NOW} WHERE group_name = ?1 COLLATE NOCASE"),
            params![name],
        )?;
        if moved == 0 {
            return Err(VaultError::InvalidInput(format!("there is no group called \"{name}\"")));
        }
        Ok(moved)
    }

    /// All sites, grouped (sites without a group last), then by name.
    pub fn list_sites(&self) -> Result<Vec<Site>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {SITE_COLUMNS} FROM sites
             ORDER BY group_name IS NULL, group_name COLLATE NOCASE, name COLLATE NOCASE, id"
        ))?;
        let sites = statement.query_map([], site_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        sites.into_iter().map(|row| row.into_site()).collect()
    }

    pub fn get_site(&self, id: i64) -> Result<Site> {
        self.conn
            .query_row(&format!("SELECT {SITE_COLUMNS} FROM sites WHERE id = ?1"), params![id], site_from_row)
            .optional()?
            .ok_or(VaultError::NotFound(id))?
            .into_site()
    }

    /// Decrypts a site's password.
    ///
    /// **Only the login engine may call this**, at fill time. It must never
    /// back a Tauri command: no command returns a password.
    pub fn reveal_password(&self, id: i64) -> Result<SecretString> {
        let (ciphertext, nonce): (Vec<u8>, Vec<u8>) = self
            .conn
            .query_row("SELECT password_ct, password_nonce FROM sites WHERE id = ?1", params![id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?
            .ok_or(VaultError::NotFound(id))?;
        crypto::decrypt(&self.key, &crypto::password_aad(id), &ciphertext, &nonce, id)
    }

    // ----- login history -----

    /// Records the start of a login attempt (outcome `running`) and marks
    /// the site as used. Returns the log row id for `finish_login`.
    pub fn start_login(&mut self, site_id: i64) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let changed = tx.execute(&format!("UPDATE sites SET last_used_at = {NOW} WHERE id = ?1"), params![site_id])?;
        if changed == 0 {
            return Err(VaultError::NotFound(site_id));
        }
        tx.execute(
            &format!("INSERT INTO login_log (site_id, started_at, outcome) VALUES (?1, {NOW}, 'running')"),
            params![site_id],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// Sets how an attempt ended. `detail` is shown to the admin and must
    /// never contain a password.
    pub fn finish_login(&self, log_id: i64, outcome: LoginOutcome, detail: Option<&str>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE login_log SET outcome = ?1, detail = ?2 WHERE id = ?3",
            params![outcome.as_str(), detail, log_id],
        )?;
        if changed == 0 {
            return Err(VaultError::NotFound(log_id));
        }
        Ok(())
    }

    /// The newest `limit` attempts for a site, newest first.
    pub fn recent_logins(&self, site_id: i64, limit: u32) -> Result<Vec<LoginRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT id, site_id, started_at, outcome, detail FROM login_log
             WHERE site_id = ?1 ORDER BY started_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![site_id, limit], |row| {
                Ok((row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?, row.get::<_, String>(3)?, row.get(4)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, site_id, started_at, outcome, detail)| {
                Ok(LoginRecord { id, site_id, started_at, outcome: LoginOutcome::parse(&outcome)?, detail })
            })
            .collect()
    }
}

/// Unwraps the stored data key, or creates, wraps and stores a new one.
fn load_or_create_key(conn: &mut Connection, protector: &dyn KeyProtector) -> Result<DataKey> {
    // IMMEDIATE takes the write lock up front, so two NetLaunch processes
    // opening a brand-new vault at once can't both create a key.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let read = |key: &str| -> rusqlite::Result<Option<Vec<u8>>> {
        tx.query_row("SELECT value FROM app_meta WHERE key = ?1", params![key], |row| row.get(0)).optional()
    };

    if let Some(wrapped) = read("wrapped_data_key")? {
        let kind = read("key_protector")?.map(|k| String::from_utf8_lossy(&k).into_owned()).unwrap_or_default();
        if kind != protector.kind() {
            return Err(VaultError::KeyProtection(format!(
                "this vault's key is protected by '{kind}', but '{}' was used to open it",
                protector.kind()
            )));
        }
        let unwrapped = protector.unprotect(&wrapped)?;
        return DataKey::from_bytes(&unwrapped);
    }

    let key = DataKey::generate()?;
    let wrapped = protector.protect(key.as_bytes())?;
    for (name, value) in [
        ("wrapped_data_key", wrapped),
        ("key_protector", protector.kind().as_bytes().to_vec()),
        ("key_version", KEY_VERSION.as_bytes().to_vec()),
    ] {
        tx.execute("INSERT INTO app_meta (key, value) VALUES (?1, ?2)", params![name, value])?;
    }
    tx.commit()?;
    Ok(key)
}

/// Checks the fields every save needs and returns the normalized origin.
fn validate(name: &str, login_url: &str, username: &str, recipe_id: &str) -> Result<String> {
    if name.trim().is_empty() {
        return Err(VaultError::InvalidInput("name is empty".into()));
    }
    // An empty username is allowed: some devices only ask for a password.
    // The app checks it against the site's recipe (sites.rs).
    let _ = username;
    if recipe_id.trim().is_empty() {
        return Err(VaultError::InvalidInput("recipe is empty".into()));
    }
    normalize_origin(login_url)
}

/// The longest group name, in characters.
pub const MAX_GROUP_NAME: usize = 40;

/// Checks a (trimmed) group name.
fn check_group_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(VaultError::InvalidInput("the group name is empty".into()));
    }
    if name.chars().count() > MAX_GROUP_NAME {
        return Err(VaultError::InvalidInput(format!("a group name can be at most {MAX_GROUP_NAME} characters")));
    }
    // The list shows sites without a group under this heading.
    if name.eq_ignore_ascii_case("no group") {
        return Err(VaultError::InvalidInput("\"No group\" is where sites without a group are listed; pick another name".into()));
    }
    Ok(())
}

/// The group to store for a site: `None` for no group. If another site
/// already has this group spelled with different capitals, that spelling
/// is used, so "switches" and "Switches" can't become two groups.
/// `except` is the site being edited, whose own old spelling doesn't count.
///
/// (`COLLATE NOCASE` compares ASCII letters without regard to case, which
/// is all SQLite does without extra libraries.)
fn settle_group(conn: &Connection, raw: Option<String>, except: Option<i64>) -> Result<Option<String>> {
    let Some(name) = clean(raw) else { return Ok(None) };
    check_group_name(&name)?;
    let existing: Option<String> = conn
        .query_row(
            "SELECT group_name FROM sites WHERE group_name = ?1 COLLATE NOCASE AND id IS NOT ?2 ORDER BY id LIMIT 1",
            params![name, except],
            |row| row.get(0),
        )
        .optional()?;
    Ok(Some(existing.unwrap_or(name)))
}

/// Optional text: trimmed, and empty becomes NULL.
fn clean(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

const SITE_COLUMNS: &str = "id, name, group_name, login_url, origin, username, recipe_id, allow_self_signed,
                            profile_mode, window_mode, notes, created_at, updated_at, last_used_at";

/// A `sites` row as read from SQLite. The enum columns are parsed
/// afterwards, because rusqlite's row closure can't return our error type.
struct SiteRow {
    site: Site,
    profile_mode: String,
    window_mode: String,
}

impl SiteRow {
    fn into_site(mut self) -> Result<Site> {
        self.site.profile_mode = ProfileMode::parse(&self.profile_mode)?;
        self.site.window_mode = WindowMode::parse(&self.window_mode)?;
        Ok(self.site)
    }
}

fn site_from_row(row: &Row<'_>) -> rusqlite::Result<SiteRow> {
    Ok(SiteRow {
        site: Site {
            id: row.get(0)?,
            name: row.get(1)?,
            group_name: row.get(2)?,
            login_url: row.get(3)?,
            origin: row.get(4)?,
            username: row.get(5)?,
            recipe_id: row.get(6)?,
            allow_self_signed: row.get(7)?,
            profile_mode: ProfileMode::default(),
            window_mode: WindowMode::default(),
            notes: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            last_used_at: row.get(13)?,
        },
        profile_mode: row.get(8)?,
        window_mode: row.get(9)?,
    })
}
