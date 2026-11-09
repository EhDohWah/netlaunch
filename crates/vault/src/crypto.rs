//! Password encryption with XChaCha20-Poly1305.
//!
//! XChaCha's 24-byte nonce is large enough to pick at random on every save
//! without worrying about repeats, which is why it was chosen over plain
//! ChaCha20-Poly1305 (12 bytes).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use crate::{Result, VaultError};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;

/// The 256-bit data key. `Zeroizing` wipes it when it is dropped, and the
/// box keeps it in one place in memory instead of being copied on moves.
pub struct DataKey(Box<Zeroizing<[u8; KEY_LEN]>>);

impl DataKey {
    pub fn generate() -> Result<Self> {
        let mut key = Box::new(Zeroizing::new([0u8; KEY_LEN]));
        getrandom::fill(key.as_mut_slice()).map_err(|e| VaultError::KeyProtection(format!("random key: {e}")))?;
        Ok(Self(key))
    }

    /// Takes a key unwrapped by a `KeyProtector`. The caller's buffer is
    /// zeroized by its own `Zeroizing` wrapper.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != KEY_LEN {
            return Err(VaultError::KeyProtection(format!("unwrapped key has {} bytes, expected {KEY_LEN}", bytes.len())));
        }
        let mut key = Box::new(Zeroizing::new([0u8; KEY_LEN]));
        key.copy_from_slice(bytes);
        Ok(Self(key))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        // The cipher keeps its own copy of the key and wipes it on drop
        // (chacha20poly1305 implements ZeroizeOnDrop).
        XChaCha20Poly1305::new(Key::from_slice(self.as_bytes()))
    }
}

/// The associated data for a site's password. Binding the row id means a
/// ciphertext copied onto another row fails to decrypt; the prefix means
/// it can't be reused for a different kind of secret added later.
pub fn password_aad(site_id: i64) -> Vec<u8> {
    format!("sites.password:{site_id}").into_bytes()
}

/// Encrypts `password` with a fresh random nonce. Returns (ciphertext, nonce).
pub fn encrypt(key: &DataKey, aad: &[u8], password: &SecretString) -> Result<(Vec<u8>, [u8; NONCE_LEN])> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|e| VaultError::KeyProtection(format!("random nonce: {e}")))?;
    let payload = Payload { msg: password.expose_secret().as_bytes(), aad };
    let ciphertext = key
        .cipher()
        .encrypt(XNonce::from_slice(&nonce), payload)
        .map_err(|_| VaultError::InvalidInput("encryption failed".into()))?;
    Ok((ciphertext, nonce))
}

/// Decrypts a password. Any failure (wrong key, wrong row, changed bytes)
/// is reported as `Decrypt` with no further detail.
pub fn decrypt(key: &DataKey, aad: &[u8], ciphertext: &[u8], nonce: &[u8], site_id: i64) -> Result<SecretString> {
    if nonce.len() != NONCE_LEN {
        return Err(VaultError::Decrypt { site_id });
    }
    let plaintext = Zeroizing::new(
        key.cipher()
            .decrypt(XNonce::from_slice(nonce), Payload { msg: ciphertext, aad })
            .map_err(|_| VaultError::Decrypt { site_id })?,
    );
    // Copy into the SecretString's own buffer; `plaintext` is wiped when it
    // goes out of scope at the end of this function.
    let text = std::str::from_utf8(&plaintext).map_err(|_| VaultError::Decrypt { site_id })?;
    Ok(SecretString::from(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    #[test]
    fn round_trip() {
        let key = DataKey::generate().unwrap();
        let (ct, nonce) = encrypt(&key, &password_aad(1), &secret("hunter2-ü")).unwrap();
        assert_eq!(decrypt(&key, &password_aad(1), &ct, &nonce, 1).unwrap().expose_secret(), "hunter2-ü");
    }

    #[test]
    fn every_save_uses_a_new_nonce() {
        let key = DataKey::generate().unwrap();
        let (ct1, n1) = encrypt(&key, &password_aad(1), &secret("same")).unwrap();
        let (ct2, n2) = encrypt(&key, &password_aad(1), &secret("same")).unwrap();
        assert_ne!(n1, n2);
        assert_ne!(ct1, ct2);
    }

    #[test]
    fn wrong_row_wrong_key_or_changed_bytes_fail() {
        let key = DataKey::generate().unwrap();
        let (ct, nonce) = encrypt(&key, &password_aad(1), &secret("pw")).unwrap();

        assert!(matches!(decrypt(&key, &password_aad(2), &ct, &nonce, 2), Err(VaultError::Decrypt { site_id: 2 })));

        let other = DataKey::generate().unwrap();
        assert!(decrypt(&other, &password_aad(1), &ct, &nonce, 1).is_err());

        let mut tampered = ct.clone();
        tampered[0] ^= 1;
        assert!(decrypt(&key, &password_aad(1), &tampered, &nonce, 1).is_err());

        assert!(decrypt(&key, &password_aad(1), &ct, &nonce[..12], 1).is_err());
    }

    #[test]
    fn errors_never_contain_the_password() {
        let key = DataKey::generate().unwrap();
        let (ct, nonce) = encrypt(&key, &password_aad(1), &secret("Sup3r-Secret")).unwrap();
        let err = decrypt(&key, &password_aad(9), &ct, &nonce, 9).unwrap_err();
        assert!(!format!("{err} {err:?}").contains("Sup3r-Secret"));
    }
}
