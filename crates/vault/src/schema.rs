//! Numbered schema migrations, tracked by `app_meta.schema_version`.
//!
//! To change the schema, append a new entry to `MIGRATIONS`; never edit one
//! that has shipped. Each runs in its own transaction together with the
//! version bump, so a failed migration leaves the file as it was.

use rusqlite::{Connection, OptionalExtension, params};

use crate::{Result, VaultError};

const MIGRATIONS: &[&str] = &[
    // 1: the spec's three tables.
    r#"
    CREATE TABLE app_meta (
      key   TEXT PRIMARY KEY,
      value BLOB NOT NULL
    );

    CREATE TABLE sites (
      id                 INTEGER PRIMARY KEY,
      name               TEXT NOT NULL,
      group_name         TEXT,
      login_url          TEXT NOT NULL,
      origin             TEXT NOT NULL,
      username           TEXT NOT NULL,
      password_ct        BLOB NOT NULL,
      password_nonce     BLOB NOT NULL,
      recipe_id          TEXT NOT NULL DEFAULT 'generic-form',
      allow_self_signed  INTEGER NOT NULL DEFAULT 0,
      profile_mode       TEXT NOT NULL DEFAULT 'fresh'   CHECK (profile_mode IN ('fresh', 'persistent')),
      window_mode        TEXT NOT NULL DEFAULT 'managed' CHECK (window_mode IN ('managed', 'released')),
      notes              TEXT,
      created_at         TEXT NOT NULL,
      updated_at         TEXT NOT NULL,
      last_used_at       TEXT
    );

    CREATE TABLE login_log (
      id          INTEGER PRIMARY KEY,
      site_id     INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
      started_at  TEXT NOT NULL,
      outcome     TEXT NOT NULL CHECK (outcome IN
                    ('running', 'success', 'mfa_handoff', 'failed', 'timeout', 'origin_mismatch')),
      detail      TEXT
    );

    CREATE INDEX login_log_site ON login_log(site_id, started_at);
    "#,
];

/// The newest schema this build knows.
pub const LATEST: i64 = MIGRATIONS.len() as i64;

pub fn current_version(conn: &Connection) -> Result<i64> {
    let has_meta: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'app_meta')",
        [],
        |row| row.get(0),
    )?;
    if !has_meta {
        return Ok(0);
    }
    let version: Option<String> = conn
        .query_row("SELECT CAST(value AS TEXT) FROM app_meta WHERE key = 'schema_version'", [], |row| row.get(0))
        .optional()?;
    version
        .map(|v| v.parse::<i64>().map_err(|_| VaultError::InvalidInput(format!("bad schema_version '{v}'"))))
        .unwrap_or(Ok(0))
}

/// Brings the file up to `LATEST`.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let found = current_version(conn)?;
    if found > LATEST {
        return Err(VaultError::NewerSchema { found, known: LATEST });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let version = index as i64 + 1;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO app_meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            // Stored as bytes like every other app_meta value (the column
            // is BLOB); `current_version` reads it back with CAST AS TEXT.
            params![version.to_string().into_bytes()],
        )?;
        tx.commit()?;
    }
    Ok(())
}
