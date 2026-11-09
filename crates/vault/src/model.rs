//! The vault's data types.
//!
//! `Site` deliberately has no password field and derives `Serialize`, so
//! the site list can go to the Tauri frontend as it is. Inputs that carry a
//! password (`NewSite`, `SiteUpdate`) hold it in a `SecretString`, whose
//! `Debug` prints `[REDACTED]`, and are never serialized.

use secrecy::SecretString;
use serde::Serialize;

use crate::{Result, VaultError};

/// Whether each launch starts with an empty profile or reuses one per site
/// (spec decision 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileMode {
    #[default]
    Fresh,
    Persistent,
}

/// What happens to the window after login (spec decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    #[default]
    Managed,
    Released,
}

/// How a login attempt ended (`login_log.outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginOutcome {
    Running,
    Success,
    MfaHandoff,
    Failed,
    Timeout,
    OriginMismatch,
}

macro_rules! text_enum {
    ($ty:ty { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $ty {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            pub fn parse(text: &str) -> Result<Self> {
                match text {
                    $($text => Ok(Self::$variant),)+
                    other => Err(VaultError::InvalidInput(format!("unknown {} '{other}'", stringify!($ty)))),
                }
            }
        }
    };
}

text_enum!(ProfileMode { Fresh => "fresh", Persistent => "persistent" });
text_enum!(WindowMode { Managed => "managed", Released => "released" });
text_enum!(LoginOutcome {
    Running => "running",
    Success => "success",
    MfaHandoff => "mfa_handoff",
    Failed => "failed",
    Timeout => "timeout",
    OriginMismatch => "origin_mismatch",
});

/// A saved site, without its password.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Site {
    pub id: i64,
    pub name: String,
    pub group_name: Option<String>,
    pub login_url: String,
    /// Normalized `scheme://host:port`, derived from `login_url` on save.
    pub origin: String,
    pub username: String,
    pub recipe_id: String,
    pub allow_self_signed: bool,
    pub profile_mode: ProfileMode,
    pub window_mode: WindowMode,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
}

/// Everything needed to add a site. The password is required.
#[derive(Debug)]
pub struct NewSite {
    pub name: String,
    pub group_name: Option<String>,
    pub login_url: String,
    pub username: String,
    pub password: SecretString,
    pub recipe_id: String,
    pub allow_self_signed: bool,
    pub profile_mode: ProfileMode,
    pub window_mode: WindowMode,
    pub notes: Option<String>,
}

impl NewSite {
    /// A site with the spec's defaults: `generic-form` recipe, fresh
    /// profile, managed window, certificate checks on.
    pub fn new(name: &str, login_url: &str, username: &str, password: SecretString) -> Self {
        Self {
            name: name.to_string(),
            group_name: None,
            login_url: login_url.to_string(),
            username: username.to_string(),
            password,
            recipe_id: "generic-form".to_string(),
            allow_self_signed: false,
            profile_mode: ProfileMode::default(),
            window_mode: WindowMode::default(),
            notes: None,
        }
    }
}

/// An edit to a site. `password: None` keeps the stored password, matching
/// the edit screen: the field opens empty, and leaving it empty changes
/// nothing.
#[derive(Debug)]
pub struct SiteUpdate {
    pub name: String,
    pub group_name: Option<String>,
    pub login_url: String,
    pub username: String,
    pub password: Option<SecretString>,
    pub recipe_id: String,
    pub allow_self_signed: bool,
    pub profile_mode: ProfileMode,
    pub window_mode: WindowMode,
    pub notes: Option<String>,
}

impl SiteUpdate {
    /// Starts an edit from the site as it is now, with the password kept.
    pub fn from_site(site: &Site) -> Self {
        Self {
            name: site.name.clone(),
            group_name: site.group_name.clone(),
            login_url: site.login_url.clone(),
            username: site.username.clone(),
            password: None,
            recipe_id: site.recipe_id.clone(),
            allow_self_signed: site.allow_self_signed,
            profile_mode: site.profile_mode,
            window_mode: site.window_mode,
            notes: site.notes.clone(),
        }
    }
}

/// One row of `login_log`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoginRecord {
    pub id: i64,
    pub site_id: i64,
    pub started_at: String,
    pub outcome: LoginOutcome,
    /// Never contains secrets.
    pub detail: Option<String>,
}
