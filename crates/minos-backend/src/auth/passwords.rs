//! Argon2id password hashing. Reuses the workspace's existing default
//! parameters (`m=19456, t=2, p=1`) — see `pairing/secret.rs`.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

use crate::error::BackendError;

pub fn hash(password: &str) -> Result<String, BackendError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| BackendError::PasswordHash {
            message: e.to_string(),
        })
}

pub fn verify(password: &str, encoded: &str) -> Result<bool, BackendError> {
    let parsed = PasswordHash::new(encoded).map_err(|e| BackendError::PasswordHash {
        message: e.to_string(),
    })?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrip() {
        let h = hash("hunter22").unwrap();
        assert!(verify("hunter22", &h).unwrap());
        assert!(!verify("wrong", &h).unwrap());
    }

    #[test]
    fn hash_is_salted_so_repeated_input_differs() {
        let a = hash("hunter22").unwrap();
        let b = hash("hunter22").unwrap();
        assert_ne!(a, b);
        // Both still verify against the plaintext.
        assert!(verify("hunter22", &a).unwrap());
        assert!(verify("hunter22", &b).unwrap());
    }

    #[test]
    fn verify_with_malformed_encoded_returns_password_hash_error() {
        let err = verify("anything", "not-a-valid-phc").unwrap_err();
        assert!(matches!(err, BackendError::PasswordHash { .. }));
    }
}
