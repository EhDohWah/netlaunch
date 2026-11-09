//! Wrapping the data key so it is never stored in the clear.
//!
//! The vault takes the protector as a parameter instead of calling DPAPI
//! directly. That keeps the vault testable on Linux CI, where DPAPI doesn't
//! exist, and leaves room for decision 4 (a master password) later.

use crate::Result;
#[cfg(any(test, feature = "insecure-test-protector"))]
use crate::VaultError;
use zeroize::Zeroizing;

/// Wraps and unwraps the vault's data key.
pub trait KeyProtector {
    /// Short name stored next to the wrapped key, so a vault opened with
    /// the wrong kind of protector fails with a clear message.
    fn kind(&self) -> &'static str;
    fn protect(&self, key: &[u8]) -> Result<Vec<u8>>;
    fn unprotect(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>>;
}

/// Windows DPAPI, current-user scope: only the same Windows account on the
/// same PC can unwrap the key (`CryptProtectData` / `CryptUnprotectData`).
#[cfg(windows)]
pub struct Dpapi;

/// Extra input DPAPI mixes into the wrapping. Another program calling
/// `CryptUnprotectData` on the stored blob also needs this value. It is
/// not a secret (it ships in the binary); it only stops casual reuse.
#[cfg(windows)]
const DPAPI_ENTROPY: &[u8] = b"NetLaunch vault data key v1";

#[cfg(windows)]
impl KeyProtector for Dpapi {
    fn kind(&self) -> &'static str {
        "dpapi-user"
    }

    fn protect(&self, key: &[u8]) -> Result<Vec<u8>> {
        // SAFETY: `dpapi_call` only reads `key` and the entropy for the
        // duration of the call.
        unsafe { dpapi::call(key, true) }.map(|out| out.to_vec())
    }

    fn unprotect(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        // SAFETY: as above.
        unsafe { dpapi::call(wrapped, false) }
    }
}

#[cfg(windows)]
mod dpapi {
    use super::DPAPI_ENTROPY;
    use crate::{Result, VaultError};
    use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use zeroize::{Zeroize, Zeroizing};

    /// One DPAPI call. Windows allocates the output buffer; we copy it into
    /// a `Zeroizing` vector, wipe Windows' copy, and free it with
    /// `LocalFree` as the documentation requires.
    pub unsafe fn call(input: &[u8], protect: bool) -> Result<Zeroizing<Vec<u8>>> {
        let len = u32::try_from(input.len()).map_err(|_| VaultError::KeyProtection("input too large".into()))?;
        let in_blob = CRYPT_INTEGER_BLOB { cbData: len, pbData: input.as_ptr() as *mut u8 };
        let entropy = CRYPT_INTEGER_BLOB { cbData: DPAPI_ENTROPY.len() as u32, pbData: DPAPI_ENTROPY.as_ptr() as *mut u8 };
        let mut out_blob = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };

        // UI_FORBIDDEN: never show a Windows prompt; fail instead.
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &in_blob,
                    std::ptr::null(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out_blob,
                )
            } else {
                CryptUnprotectData(
                    &in_blob,
                    std::ptr::null_mut(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out_blob,
                )
            }
        };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            let what = if protect { "CryptProtectData" } else { "CryptUnprotectData" };
            return Err(VaultError::KeyProtection(format!("{what} failed with Windows error {code}")));
        }

        let out = unsafe { std::slice::from_raw_parts_mut(out_blob.pbData, out_blob.cbData as usize) };
        let copy = Zeroizing::new(out.to_vec());
        out.zeroize();
        unsafe { LocalFree(out_blob.pbData as _) };
        Ok(copy)
    }
}

/// **Not protection.** Stores the key with a fixed prefix, so the vault can
/// be tested on machines without DPAPI. Only available in this crate's
/// tests and with the `insecure-test-protector` feature, which the app
/// must never enable.
#[cfg(any(test, feature = "insecure-test-protector"))]
pub struct InsecureTestProtector;

#[cfg(any(test, feature = "insecure-test-protector"))]
const TEST_PREFIX: &[u8] = b"INSECURE-TEST-KEY:";

#[cfg(any(test, feature = "insecure-test-protector"))]
impl KeyProtector for InsecureTestProtector {
    fn kind(&self) -> &'static str {
        "insecure-test"
    }

    fn protect(&self, key: &[u8]) -> Result<Vec<u8>> {
        Ok([TEST_PREFIX, key].concat())
    }

    fn unprotect(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        wrapped
            .strip_prefix(TEST_PREFIX)
            .map(|key| Zeroizing::new(key.to_vec()))
            .ok_or_else(|| VaultError::KeyProtection("not a test-protected key".into()))
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn dpapi_round_trip() {
        let key = [7u8; 32];
        let wrapped = Dpapi.protect(&key).unwrap();
        assert_ne!(&wrapped[..], &key[..]);
        assert!(!wrapped.windows(key.len()).any(|w| w == key));
        assert_eq!(&Dpapi.unprotect(&wrapped).unwrap()[..], &key[..]);
    }

    #[test]
    fn dpapi_rejects_a_changed_blob() {
        let mut wrapped = Dpapi.protect(&[1u8; 32]).unwrap();
        let last = wrapped.len() - 1;
        wrapped[last] ^= 0xff;
        assert!(Dpapi.unprotect(&wrapped).is_err());
    }
}
