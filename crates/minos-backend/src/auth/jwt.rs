//! HS256 JWT helpers. Spec §5.3.
//!
//! Claims: { sub: account_id, did: device_id, iat, exp, jti }. The
//! `did` claim binds the access token to a specific device — replay
//! from another device is rejected at verify time.

use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::BackendError;

pub const ACCESS_TTL_SECS: i64 = 15 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claims {
    pub sub: String,
    pub did: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

pub fn sign(secret: &[u8], account_id: &str, device_id: &str) -> Result<String, BackendError> {
    let now = Utc::now().timestamp();
    let claims = Claims {
        sub: account_id.into(),
        did: device_id.into(),
        iat: now,
        exp: now + ACCESS_TTL_SECS,
        jti: Uuid::new_v4().to_string(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .map_err(|e| BackendError::JwtSign {
        message: e.to_string(),
    })
}

/// Parse + verify (signature + exp). Caller is responsible for
/// `did == X-Device-Id` check (`bearer.rs` does it).
pub fn verify(secret: &[u8], token: &str) -> Result<Claims, BackendError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 5;
    let data =
        decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation).map_err(|e| {
            BackendError::JwtVerify {
                message: e.to_string(),
            }
        })?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip() {
        let secret = b"a".repeat(32);
        let tok = sign(&secret, "acct-1", "dev-1").unwrap();
        let claims = verify(&secret, &tok).unwrap();
        assert_eq!(claims.sub, "acct-1");
        assert_eq!(claims.did, "dev-1");
        // exp is iat + ACCESS_TTL_SECS
        assert_eq!(claims.exp - claims.iat, ACCESS_TTL_SECS);
        assert!(!claims.jti.is_empty());
    }

    #[test]
    fn verify_with_wrong_secret_fails() {
        let tok = sign(&b"a".repeat(32), "acct-1", "dev-1").unwrap();
        assert!(verify(&b"b".repeat(32), &tok).is_err());
    }

    #[test]
    fn verify_with_garbage_token_fails() {
        let secret = b"a".repeat(32);
        assert!(verify(&secret, "not.a.jwt").is_err());
    }

    #[test]
    fn each_call_to_sign_emits_a_unique_jti() {
        let secret = b"a".repeat(32);
        let t1 = sign(&secret, "acct-1", "dev-1").unwrap();
        let t2 = sign(&secret, "acct-1", "dev-1").unwrap();
        // Different `jti` ⇒ different bodies ⇒ different signatures, even
        // though sub/did/iat may collide. (iat is second-precision.)
        let c1 = verify(&secret, &t1).unwrap();
        let c2 = verify(&secret, &t2).unwrap();
        assert_ne!(c1.jti, c2.jti);
    }
}
