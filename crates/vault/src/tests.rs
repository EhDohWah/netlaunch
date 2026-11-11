//! Vault tests. They use `InsecureTestProtector` so they run anywhere; the
//! DPAPI tests live in `protect.rs` and run only on Windows.

use std::path::{Path, PathBuf};

use rusqlite::params;
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use crate::*;

fn secret(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

/// A temporary folder removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("netlaunch-vault-{name}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn db(&self) -> PathBuf {
        self.0.join("vault.db")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(path: &Path) -> Vault {
    Vault::open(path, &InsecureTestProtector).unwrap()
}

fn unifi(password: &str) -> NewSite {
    let mut site = NewSite::new("UniFi Controller", "https://10.100.100.18:8443/manage/account/login", "netadmin", secret(password));
    site.group_name = Some("Wireless".into());
    site
}

#[test]
fn add_list_and_reveal() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    let id = vault.add_site(unifi("Unifi-Pa55")).unwrap();

    let sites = vault.list_sites().unwrap();
    assert_eq!(sites.len(), 1);
    let site = &sites[0];
    assert_eq!(site.id, id);
    assert_eq!(site.origin, "https://10.100.100.18:8443");
    assert_eq!(site.recipe_id, "generic-form");
    assert_eq!(site.profile_mode, ProfileMode::Fresh);
    assert_eq!(site.window_mode, WindowMode::Managed);
    assert!(!site.allow_self_signed);
    assert!(site.created_at.ends_with('Z'));
    assert_eq!(site.last_used_at, None);

    assert_eq!(vault.reveal_password(id).unwrap().expose_secret(), "Unifi-Pa55");
    assert_eq!(vault.get_site(id).unwrap(), *site);
}

#[test]
fn the_site_list_never_carries_a_password() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    vault.add_site(unifi("Never-In-Json-1")).unwrap();
    let json = serde_json::to_string(&vault.list_sites().unwrap()).unwrap();
    assert!(!json.contains("Never-In-Json-1"));
    assert!(!json.to_lowercase().contains("password"));
    assert!(json.contains(r#""profile_mode":"fresh""#));
}

#[test]
fn inputs_never_print_their_password() {
    let site = unifi("Debug-Secret-9");
    assert!(!format!("{site:?}").contains("Debug-Secret-9"));
}

#[test]
fn passwords_survive_a_restart_with_the_same_key() {
    let dir = TempDir::new("restart");
    let id = open(&dir.db()).add_site(unifi("Restart-Pa55")).unwrap();
    let wrapped_before: Vec<u8> = read_meta(&dir.db(), "wrapped_data_key");

    let vault = open(&dir.db());
    assert_eq!(vault.reveal_password(id).unwrap().expose_secret(), "Restart-Pa55");
    assert_eq!(read_meta(&dir.db(), "wrapped_data_key"), wrapped_before, "the key must not be regenerated");
}

fn read_meta(db: &Path, key: &str) -> Vec<u8> {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.query_row("SELECT value FROM app_meta WHERE key = ?1", params![key], |row| row.get(0)).unwrap()
}

/// M3's "done when": the file on disk shows no plaintext password, not even
/// one that was replaced or deleted.
#[test]
fn the_database_file_contains_no_plaintext_password() {
    let dir = TempDir::new("plaintext");
    {
        let mut vault = open(&dir.db());
        let kept = vault.add_site(unifi("Kept-Pa55-aaaa")).unwrap();
        let replaced = vault.add_site(unifi("Old-Pa55-bbbb")).unwrap();
        let deleted = vault.add_site(unifi("Deleted-Pa55-cccc")).unwrap();

        let mut update = SiteUpdate::from_site(&vault.get_site(replaced).unwrap());
        update.password = Some(secret("New-Pa55-dddd"));
        vault.update_site(replaced, update).unwrap();
        vault.delete_site(deleted).unwrap();
        assert_eq!(vault.reveal_password(kept).unwrap().expose_secret(), "Kept-Pa55-aaaa");
    }

    let mut bytes = Vec::new();
    for entry in std::fs::read_dir(&dir.0).unwrap() {
        bytes.extend(std::fs::read(entry.unwrap().path()).unwrap());
    }
    for password in ["Kept-Pa55-aaaa", "Old-Pa55-bbbb", "Deleted-Pa55-cccc", "New-Pa55-dddd"] {
        assert!(!contains(&bytes, password.as_bytes()), "{password} found in the vault file");
    }
    // Usernames are stored readable on purpose (decision 8).
    assert!(contains(&bytes, b"netadmin"));
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn a_ciphertext_copied_to_another_row_does_not_decrypt() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    let a = vault.add_site(unifi("Row-A-Pa55")).unwrap();
    let b = vault.add_site(unifi("Row-B-Pa55")).unwrap();
    vault
        .conn
        .execute(
            "UPDATE sites SET (password_ct, password_nonce) = (SELECT password_ct, password_nonce FROM sites WHERE id = ?1)
             WHERE id = ?2",
            params![a, b],
        )
        .unwrap();
    assert!(matches!(vault.reveal_password(b), Err(VaultError::Decrypt { site_id }) if site_id == b));
    assert_eq!(vault.reveal_password(a).unwrap().expose_secret(), "Row-A-Pa55");
}

#[test]
fn a_changed_ciphertext_does_not_decrypt() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    let id = vault.add_site(unifi("Tamper-Pa55")).unwrap();
    vault
        .conn
        .execute(
            "UPDATE sites SET password_ct = CAST(substr(password_ct, 2) || substr(password_ct, 1, 1) AS BLOB) WHERE id = ?1",
            params![id],
        )
        .unwrap();
    assert!(matches!(vault.reveal_password(id), Err(VaultError::Decrypt { .. })));
}

#[test]
fn editing_without_a_password_keeps_it_and_with_one_replaces_it() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    let id = vault.add_site(unifi("First-Pa55")).unwrap();
    let nonce = |vault: &Vault| -> Vec<u8> {
        vault.conn.query_row("SELECT password_nonce FROM sites WHERE id = ?1", params![id], |r| r.get(0)).unwrap()
    };
    let first_nonce = nonce(&vault);

    let mut update = SiteUpdate::from_site(&vault.get_site(id).unwrap());
    update.name = "UniFi (site B)".into();
    update.login_url = "https://unifi.example:8443/".into();
    update.window_mode = WindowMode::Released;
    vault.update_site(id, update).unwrap();

    let site = vault.get_site(id).unwrap();
    assert_eq!(site.name, "UniFi (site B)");
    assert_eq!(site.origin, "https://unifi.example:8443");
    assert_eq!(site.window_mode, WindowMode::Released);
    assert_eq!(vault.reveal_password(id).unwrap().expose_secret(), "First-Pa55");
    assert_eq!(nonce(&vault), first_nonce);

    let mut update = SiteUpdate::from_site(&site);
    update.password = Some(secret("Second-Pa55"));
    vault.update_site(id, update).unwrap();
    assert_eq!(vault.reveal_password(id).unwrap().expose_secret(), "Second-Pa55");
    assert_ne!(nonce(&vault), first_nonce);
}

#[test]
fn missing_sites_are_reported() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    let id = vault_add(&mut vault);
    let update = SiteUpdate::from_site(&vault.get_site(id).unwrap());
    assert!(matches!(vault.update_site(999, update), Err(VaultError::NotFound(999))));
    assert!(matches!(vault.delete_site(999), Err(VaultError::NotFound(999))));
    assert!(matches!(vault.get_site(999), Err(VaultError::NotFound(999))));
    assert!(matches!(vault.reveal_password(999), Err(VaultError::NotFound(999))));
    assert!(matches!(vault.start_login(999), Err(VaultError::NotFound(999))));
}

fn vault_add(vault: &mut Vault) -> i64 {
    vault.add_site(unifi("Any-Pa55")).unwrap()
}

#[test]
fn invalid_input_is_rejected() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    let cases: Vec<(&str, NewSite)> = vec![
        ("empty name", NewSite::new("  ", "https://a.example", "u", secret("p"))),
        ("empty password", NewSite::new("n", "https://a.example", "u", secret(""))),
        ("ftp url", NewSite::new("n", "ftp://a.example", "u", secret("p"))),
        ("not a url", NewSite::new("n", "router", "u", secret("p"))),
    ];
    for (label, site) in cases {
        assert!(matches!(vault.add_site(site), Err(VaultError::InvalidInput(_))), "{label} was accepted");
    }
    assert!(vault.list_sites().unwrap().is_empty());

    let id = vault_add(&mut vault);
    let mut update = SiteUpdate::from_site(&vault.get_site(id).unwrap());
    update.password = Some(secret(""));
    assert!(matches!(vault.update_site(id, update), Err(VaultError::InvalidInput(_))));
}

#[test]
fn optional_text_is_trimmed_and_empty_becomes_none() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    let mut site = unifi("Trim-Pa55");
    site.group_name = Some("   ".into());
    site.notes = Some("  rack 3  ".into());
    let id = vault.add_site(site).unwrap();
    let site = vault.get_site(id).unwrap();
    assert_eq!(site.group_name, None);
    assert_eq!(site.notes.as_deref(), Some("rack 3"));
}

#[test]
fn sites_are_listed_by_group_then_name_with_ungrouped_last() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    for (name, group) in [("zeta", Some("Servers")), ("beta", None), ("alpha", Some("servers")), ("gamma", Some("Wireless"))] {
        let mut site = NewSite::new(name, "https://h.example", "u", secret("p"));
        site.group_name = group.map(String::from);
        vault.add_site(site).unwrap();
    }
    let names: Vec<_> = vault.list_sites().unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["alpha", "zeta", "gamma", "beta"]);
}

#[test]
fn login_history_records_attempts_and_cascades_on_delete() {
    let mut vault = Vault::open_in_memory(&InsecureTestProtector).unwrap();
    let id = vault_add(&mut vault);

    let first = vault.start_login(id).unwrap();
    vault.finish_login(first, LoginOutcome::MfaHandoff, Some("2FA screen detected")).unwrap();
    let second = vault.start_login(id).unwrap();

    assert!(vault.get_site(id).unwrap().last_used_at.is_some());
    let history = vault.recent_logins(id, 10).unwrap();
    assert_eq!(history.iter().map(|r| r.id).collect::<Vec<_>>(), [second, first]);
    assert_eq!(history[0].outcome, LoginOutcome::Running);
    assert_eq!(history[1].outcome, LoginOutcome::MfaHandoff);
    assert_eq!(history[1].detail.as_deref(), Some("2FA screen detected"));
    assert_eq!(vault.recent_logins(id, 1).unwrap().len(), 1);

    vault.delete_site(id).unwrap();
    let left: i64 = vault.conn.query_row("SELECT COUNT(*) FROM login_log", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0, "deleting a site must delete its history (foreign keys on)");
}

#[test]
fn reopening_does_not_rerun_migrations() {
    let dir = TempDir::new("migrate");
    open(&dir.db());
    open(&dir.db());
    let version = String::from_utf8(read_meta(&dir.db(), "schema_version")).unwrap();
    assert_eq!(version, schema::LATEST.to_string());
}

#[test]
fn a_vault_from_a_newer_version_is_refused() {
    let dir = TempDir::new("newer");
    open(&dir.db());
    let conn = rusqlite::Connection::open(dir.db()).unwrap();
    conn.execute("UPDATE app_meta SET value = '99' WHERE key = 'schema_version'", []).unwrap();
    drop(conn);
    assert!(matches!(Vault::open(&dir.db(), &InsecureTestProtector), Err(VaultError::NewerSchema { found: 99, .. })));
}

/// A protector of another kind, to check the vault refuses to mix them.
struct OtherProtector;

impl KeyProtector for OtherProtector {
    fn kind(&self) -> &'static str {
        "other"
    }
    fn protect(&self, key: &[u8]) -> Result<Vec<u8>> {
        Ok(key.to_vec())
    }
    fn unprotect(&self, _wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        Err(VaultError::KeyProtection("other protector can't unwrap".into()))
    }
}

#[test]
fn the_key_only_opens_with_the_protector_that_wrapped_it() {
    let dir = TempDir::new("protector");
    open(&dir.db());
    let err = Vault::open(&dir.db(), &OtherProtector).err().unwrap();
    assert!(matches!(err, VaultError::KeyProtection(ref m) if m.contains("insecure-test")), "{err}");
}

#[test]
fn a_key_that_fails_to_unwrap_stops_the_open() {
    let dir = TempDir::new("unwrap");
    open(&dir.db());
    let conn = rusqlite::Connection::open(dir.db()).unwrap();
    conn.execute("UPDATE app_meta SET value = x'00' WHERE key = 'wrapped_data_key'", []).unwrap();
    drop(conn);
    assert!(matches!(Vault::open(&dir.db(), &InsecureTestProtector), Err(VaultError::KeyProtection(_))));
}

#[test]
fn opening_creates_the_folder() {
    let dir = TempDir::new("folder");
    let nested = dir.0.join("NetLaunch").join("vault.db");
    open(&nested);
    assert!(nested.exists());
}

#[test]
fn default_path_is_under_localappdata() {
    if let Some(path) = default_path() {
        assert!(path.ends_with(Path::new("NetLaunch").join("vault.db")));
    }
}

/// The real combination on Windows: a vault on disk protected by DPAPI.
/// Unlike the test protector, DPAPI must leave neither the passwords nor
/// the raw data key readable in the file.
#[cfg(windows)]
#[test]
fn with_dpapi_the_file_holds_neither_passwords_nor_the_key() {
    let dir = TempDir::new("dpapi");
    let (id, raw_key) = {
        let mut vault = Vault::open(&dir.db(), &Dpapi).unwrap();
        let id = vault.add_site(unifi("Dpapi-Pa55-eeee")).unwrap();
        (id, vault.key.as_bytes().to_vec())
    };

    let vault = Vault::open(&dir.db(), &Dpapi).unwrap();
    assert_eq!(vault.reveal_password(id).unwrap().expose_secret(), "Dpapi-Pa55-eeee");
    drop(vault);

    let bytes = std::fs::read(dir.db()).unwrap();
    assert!(!contains(&bytes, b"Dpapi-Pa55-eeee"));
    assert!(!contains(&bytes, &raw_key), "the raw data key is in the file");
    assert!(matches!(Vault::open(&dir.db(), &InsecureTestProtector), Err(VaultError::KeyProtection(_))));
}
