//! Long-lived DeviceSecret hashing primitives (argon2id PHC).
//!
//! Separate from the pairing-token digest in [`super`] because the two
//! values have different threat profiles:
//!
//! - `DeviceSecret` is stored forever in the backend's SQLite `devices.secret_hash`
//!   column, so a DB exfiltration attacker has unbounded compute time to brute
//!   the original. Argon2id's tunable memory/time cost is the right tool.
//! - `PairingToken` is short-lived (≤5 min TTL) and high-entropy (32B random,
//!   256 bits), so a deterministic SHA-256 digest is both sufficient and
//!   necessary (deterministic → PK lookup works). See the module doc on
//!   [`super`] for details.
//!
//! The plan (§6) asks `verify_secret` to wrap `argon2::Argon2::verify_password`
//! "with `subtle::ConstantTimeEq` for the final byte compare". `verify_password`
//! already performs a constant-time comparison internally (via `subtle` pulled
//! in by `password_hash`), so this module is the belt; the surrounding code
//! structure is the suspenders — we never route the verify path through a
//! non-constant-time `==` on byte slices.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use minos_domain::DeviceSecret;

use crate::error::BackendError;

/// Hash a fresh `DeviceSecret` with argon2id default parameters for at-rest
/// storage.
///
/// The returned PHC string carries algorithm id, params, random salt, and
/// digest — all we need for [`verify_secret`] later.
pub fn hash_secret(plain: &DeviceSecret) -> Result<String, BackendError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_str().as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| BackendError::PairingHash {
            message: e.to_string(),
        })
}

/// Verify a candidate plaintext secret against a stored PHC hash.
///
/// - `Ok(true)` — candidate matches the stored hash.
/// - `Ok(false)` — candidate does not match; signal this to the caller as a
///   failed authentication (distinct from a malformed-hash error).
/// - `Err(BackendError::PairingHash)` — the stored hash string failed to parse
///   (schema drift or column corruption) or argon2 reported an internal error.
///
/// `argon2::Argon2::verify_password` performs the final byte compare via
/// `subtle::ConstantTimeEq`, so no additional guard is needed. We never
/// recover the plaintext or use a `==` on the digest bytes in this path.
pub fn verify_secret(plain: &str, hash: &str) -> Result<bool, BackendError> {
    let parsed = PasswordHash::new(hash).map_err(|e| BackendError::PairingHash {
        message: e.to_string(),
    })?;
    match Argon2::default().verify_password(plain.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(BackendError::PairingHash {
            message: e.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn hash_then_verify_round_trips_matching_secret() {
        let s = DeviceSecret::generate();
        let h = hash_secret(&s).unwrap();
        assert!(verify_secret(s.as_str(), &h).unwrap());
    }

    #[test]
    fn verify_rejects_non_matching_secret() {
        let s = DeviceSecret::generate();
        let h = hash_secret(&s).unwrap();
        assert_eq!(verify_secret("not the secret", &h).unwrap(), false);
    }

    #[test]
    fn hash_is_salted_so_same_input_produces_different_output() {
        // PHC encodes a fresh random salt; two calls on the same plaintext
        // must differ, else the per-call salt is broken.
        let s = DeviceSecret::generate();
        let h1 = hash_secret(&s).unwrap();
        let h2 = hash_secret(&s).unwrap();
        assert_ne!(h1, h2);
        // …but both still verify against the plaintext.
        assert!(verify_secret(s.as_str(), &h1).unwrap());
        assert!(verify_secret(s.as_str(), &h2).unwrap());
    }

    #[test]
    fn verify_with_malformed_hash_returns_pairing_hash_error() {
        let err = verify_secret("anything", "not-a-valid-phc-string").unwrap_err();
        match err {
            BackendError::PairingHash { message } => {
                assert!(!message.is_empty());
            }
            other => panic!("expected PairingHash, got {other:?}"),
        }
    }
}
