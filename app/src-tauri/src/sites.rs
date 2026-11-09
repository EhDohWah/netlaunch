//! What the site commands do, as plain functions over a `Vault`, so they
//! can be tested without Tauri or a window. `main.rs` only wraps them.
//!
//! The rule from the spec: the frontend may send a password when saving,
//! but nothing sends one back. Every function here returns a `Site` (which
//! has no password field) or nothing.

use netlaunch_vault::{NewSite, ProfileMode, Site, SiteUpdate, Vault, VaultError, WindowMode};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

/// The add/edit form, as the frontend sends it. Field names match `Site`'s
/// JSON (snake_case).
///
/// `password` goes straight into a `SecretString`: its `Debug` prints
/// `[REDACTED]`, it's wiped when dropped, and it can't be serialized.
#[derive(Debug, Deserialize)]
pub struct SiteInput {
    pub name: String,
    #[serde(default)]
    pub group_name: Option<String>,
    pub login_url: String,
    pub username: String,
    /// Required when adding. When editing, empty or missing keeps the
    /// stored password, like the edit screen's empty password field.
    #[serde(default)]
    pub password: Option<SecretString>,
    #[serde(default = "default_recipe")]
    pub recipe_id: String,
    #[serde(default)]
    pub allow_self_signed: bool,
    #[serde(default)]
    pub profile_mode: ProfileMode,
    #[serde(default)]
    pub window_mode: WindowMode,
    #[serde(default)]
    pub notes: Option<String>,
}

fn default_recipe() -> String {
    "generic-form".to_string()
}

/// `None` for a missing or empty password.
fn non_empty(password: Option<SecretString>) -> Option<SecretString> {
    password.filter(|p| !p.expose_secret().is_empty())
}

pub fn add(vault: &mut Vault, input: SiteInput) -> Result<Site, VaultError> {
    let password =
        non_empty(input.password).ok_or_else(|| VaultError::InvalidInput("a password is required".into()))?;
    let mut site = NewSite::new(&input.name, &input.login_url, &input.username, password);
    site.group_name = input.group_name;
    site.recipe_id = input.recipe_id;
    site.allow_self_signed = input.allow_self_signed;
    site.profile_mode = input.profile_mode;
    site.window_mode = input.window_mode;
    site.notes = input.notes;
    let id = vault.add_site(site)?;
    vault.get_site(id)
}

pub fn update(vault: &mut Vault, id: i64, input: SiteInput) -> Result<Site, VaultError> {
    let update = SiteUpdate {
        name: input.name,
        group_name: input.group_name,
        login_url: input.login_url,
        username: input.username,
        password: non_empty(input.password),
        recipe_id: input.recipe_id,
        allow_self_signed: input.allow_self_signed,
        profile_mode: input.profile_mode,
        window_mode: input.window_mode,
        notes: input.notes,
    };
    vault.update_site(id, update)?;
    vault.get_site(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use netlaunch_vault::InsecureTestProtector;

    fn vault() -> Vault {
        Vault::open_in_memory(&InsecureTestProtector).unwrap()
    }

    /// Parses the JSON the frontend sends, as Tauri would.
    fn input(json: &str) -> SiteInput {
        serde_json::from_str(json).unwrap()
    }

    const NEW: &str = r#"{
        "name": "UniFi", "group_name": "Wireless",
        "login_url": "https://10.100.100.18:8443/manage", "username": "netadmin",
        "password": "Ui-Secret-1111", "recipe_id": "generic-form",
        "allow_self_signed": true, "profile_mode": "fresh", "window_mode": "released", "notes": null
    }"#;

    #[test]
    fn add_returns_the_site_without_its_password() {
        let mut vault = vault();
        let site = add(&mut vault, input(NEW)).unwrap();
        assert_eq!(site.origin, "https://10.100.100.18:8443");
        assert!(site.allow_self_signed);
        assert_eq!(site.window_mode, WindowMode::Released);

        let json = serde_json::to_string(&site).unwrap();
        assert!(!json.contains("Ui-Secret-1111"));
        assert_eq!(vault.reveal_password(site.id).unwrap().expose_secret(), "Ui-Secret-1111");
    }

    /// Every value a site command returns, serialized as Tauri would send
    /// it to the frontend, must be free of the password.
    #[test]
    fn no_command_output_contains_a_password() {
        let mut vault = vault();
        let added = add(&mut vault, input(NEW)).unwrap();
        let edit = NEW.replace("Ui-Secret-1111", "Ui-Secret-2222");
        let updated = update(&mut vault, added.id, input(&edit)).unwrap();
        let listed = vault.list_sites().unwrap();

        let outputs = [
            serde_json::to_string(&added).unwrap(),
            serde_json::to_string(&updated).unwrap(),
            serde_json::to_string(&listed).unwrap(),
        ];
        for output in outputs {
            assert!(!output.contains("Ui-Secret"), "a password leaked: {output}");
            assert!(!output.contains("password"), "a password field appeared: {output}");
        }
    }

    #[test]
    fn add_requires_a_password() {
        let mut vault = vault();
        for without in [
            NEW.replace(r#""password": "Ui-Secret-1111","#, ""),
            NEW.replace(r#""Ui-Secret-1111""#, "null"),
            NEW.replace("Ui-Secret-1111", ""),
        ] {
            assert!(matches!(add(&mut vault, input(&without)), Err(VaultError::InvalidInput(_))));
        }
    }

    #[test]
    fn editing_with_an_empty_password_keeps_the_stored_one() {
        let mut vault = vault();
        let site = add(&mut vault, input(NEW)).unwrap();

        for keep in [NEW.replace("Ui-Secret-1111", ""), NEW.replace(r#""Ui-Secret-1111""#, "null")] {
            let edited = update(&mut vault, site.id, input(&keep.replace("UniFi", "UniFi 2"))).unwrap();
            assert_eq!(edited.name, "UniFi 2");
            assert_eq!(vault.reveal_password(site.id).unwrap().expose_secret(), "Ui-Secret-1111");
        }

        update(&mut vault, site.id, input(&NEW.replace("Ui-Secret-1111", "Ui-Secret-3333"))).unwrap();
        assert_eq!(vault.reveal_password(site.id).unwrap().expose_secret(), "Ui-Secret-3333");
    }

    #[test]
    fn defaults_fill_in_optional_fields() {
        let mut vault = vault();
        let minimal = input(r#"{"name":"Router","login_url":"http://192.168.1.1/","username":"admin","password":"x"}"#);
        let site = add(&mut vault, minimal).unwrap();
        assert_eq!(site.recipe_id, "generic-form");
        assert_eq!(site.profile_mode, ProfileMode::Fresh);
        assert_eq!(site.window_mode, WindowMode::Managed);
        assert!(!site.allow_self_signed);
    }

    #[test]
    fn the_input_never_prints_its_password() {
        assert!(!format!("{:?}", input(NEW)).contains("Ui-Secret-1111"));
    }

    #[test]
    fn errors_from_the_vault_reach_the_caller() {
        let mut vault = vault();
        let bad_url = input(&NEW.replace("https://10.100.100.18:8443/manage", "ftp://x"));
        assert!(matches!(add(&mut vault, bad_url), Err(VaultError::InvalidInput(_))));
        assert!(matches!(update(&mut vault, 42, input(NEW)), Err(VaultError::NotFound(42))));
    }
}
