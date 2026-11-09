//! NetLaunch credential vault (milestone M3).
//!
//! One SQLite file holds the site list. Only the password column is
//! encrypted (decision 8: usernames stay readable):
//!
//! - a random 256-bit **data key** encrypts each password with
//!   XChaCha20-Poly1305, a fresh 24-byte nonce per save, and
//!   `"sites.password:<id>"` as associated data, so a ciphertext copied onto
//!   another row fails to decrypt;
//! - the data key itself is stored **wrapped** by a [`KeyProtector`]. On
//!   Windows that is DPAPI ([`Dpapi`]), so only the same Windows account can
//!   unwrap it. There is no master password (decision 4).
//!
//! The vault never hands a password to anything but [`Vault::reveal_password`],
//! which only the login engine may call. [`Site`] has no password field, so
//! the site list can be sent to the UI as it is.

mod crypto;
mod model;
mod origin;
mod protect;
mod schema;
mod vault;

#[cfg(test)]
mod tests;

pub use model::{LoginOutcome, LoginRecord, NewSite, ProfileMode, Site, SiteUpdate, WindowMode};
pub use origin::normalize_origin;
#[cfg(windows)]
pub use protect::Dpapi;
#[cfg(any(test, feature = "insecure-test-protector"))]
pub use protect::InsecureTestProtector;
pub use protect::KeyProtector;
pub use vault::{Vault, default_path};

/// Everything that can go wrong in the vault. No variant ever carries a
/// password: decryption failures say only that decryption failed.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("could not create the vault folder: {0}")]
    Io(#[from] std::io::Error),

    /// DPAPI (or the test protector) could not wrap or unwrap the data key,
    /// for example because the vault was copied from another Windows account.
    #[error("could not protect or unlock the data key: {0}")]
    KeyProtection(String),

    /// The stored password did not decrypt: wrong key, wrong row, or the
    /// data was changed.
    #[error("the stored password could not be decrypted for site {site_id}")]
    Decrypt { site_id: i64 },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("site {0} does not exist")]
    NotFound(i64),

    /// The file was written by a newer NetLaunch. Refuse rather than risk
    /// damaging data this version doesn't understand.
    #[error("the vault was created by a newer NetLaunch (schema {found}, this version knows up to {known})")]
    NewerSchema { found: i64, known: i64 },
}

pub type Result<T> = std::result::Result<T, VaultError>;
