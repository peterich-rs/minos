//! Bearer-token extractor (spec §5.4). Pattern mirrors
//! `crate::http::auth::authenticate` — handler-level call, not axum
//! middleware, so handlers can opt in per route.

use axum::http::{HeaderMap, StatusCode};

use crate::auth::jwt::{self, Claims};
use crate::error::BackendError;
use crate::http::auth::extract_device_id;
use crate::http::BackendState;

#[derive(Debug, Clone)]
pub struct AccountAuthOutcome {
    pub account_id: String,
    pub device_id: String,
    pub claims: Claims,
}

#[derive(Debug)]
pub enum BearerError {
    Missing,
    Invalid(String),
    DeviceMismatch,
}

impl BearerError {
    pub fn into_response_tuple(self) -> (StatusCode, String) {
        match self {
            Self::Missing => (StatusCode::UNAUTHORIZED, "missing bearer".into()),
            Self::Invalid(m) => (StatusCode::UNAUTHORIZED, format!("invalid bearer: {m}")),
            Self::DeviceMismatch => (StatusCode::UNAUTHORIZED, "device mismatch".into()),
        }
    }
}

/// Verify the `Authorization: Bearer <jwt>` header and bind it to the
/// `X-Device-Id` header. Returns `Ok(AccountAuthOutcome)` only when:
///
/// - Header is present and starts with `Bearer ` (case-insensitive).
/// - JWT signature + exp validate against `state.jwt_secret`.
/// - JWT `did` claim equals the `X-Device-Id` header (replay defence).
pub fn require(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<AccountAuthOutcome, BearerError> {
    let raw = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(BearerError::Missing)?;
    let tok = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .ok_or(BearerError::Missing)?;
    let claims = jwt::verify(state.jwt_secret.as_bytes(), tok).map_err(|e| match e {
        BackendError::JwtVerify { message } => BearerError::Invalid(message),
        _ => BearerError::Invalid("verify failed".into()),
    })?;
    let device_id = extract_device_id(headers).map_err(|_| BearerError::DeviceMismatch)?;
    if claims.did != device_id.to_string() {
        return Err(BearerError::DeviceMismatch);
    }
    Ok(AccountAuthOutcome {
        account_id: claims.sub.clone(),
        device_id: device_id.to_string(),
        claims,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt;
    use crate::http::test_support::{backend_state, TEST_JWT_SECRET};
    use minos_domain::DeviceId;

    fn auth_header(token: &str) -> String {
        format!("Bearer {token}")
    }

    #[tokio::test]
    async fn require_missing_authorization_returns_missing() {
        let state = backend_state().await;
        let headers = HeaderMap::new();
        let err = require(&state, &headers).unwrap_err();
        assert!(matches!(err, BearerError::Missing));
    }

    #[tokio::test]
    async fn require_with_valid_token_and_matching_device_succeeds() {
        let state = backend_state().await;
        let device_id = DeviceId::new();
        let token = jwt::sign(
            TEST_JWT_SECRET.as_bytes(),
            "acct-1",
            &device_id.to_string(),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", auth_header(&token).parse().unwrap());
        headers.insert("x-device-id", device_id.to_string().parse().unwrap());
        let outcome = require(&state, &headers).unwrap();
        assert_eq!(outcome.account_id, "acct-1");
        assert_eq!(outcome.device_id, device_id.to_string());
    }

    #[tokio::test]
    async fn require_with_mismatched_device_returns_device_mismatch() {
        let state = backend_state().await;
        let token =
            jwt::sign(TEST_JWT_SECRET.as_bytes(), "acct-1", "some-other-device").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", auth_header(&token).parse().unwrap());
        headers.insert(
            "x-device-id",
            DeviceId::new().to_string().parse().unwrap(),
        );
        let err = require(&state, &headers).unwrap_err();
        assert!(matches!(err, BearerError::DeviceMismatch));
    }

    #[tokio::test]
    async fn require_with_garbage_token_returns_invalid() {
        let state = backend_state().await;
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer not.a.jwt".parse().unwrap());
        headers.insert(
            "x-device-id",
            DeviceId::new().to_string().parse().unwrap(),
        );
        let err = require(&state, &headers).unwrap_err();
        assert!(matches!(err, BearerError::Invalid(_)));
    }

    #[tokio::test]
    async fn require_with_lowercase_bearer_prefix_succeeds() {
        let state = backend_state().await;
        let device_id = DeviceId::new();
        let token = jwt::sign(
            TEST_JWT_SECRET.as_bytes(),
            "acct-1",
            &device_id.to_string(),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("bearer {token}").parse().unwrap(),
        );
        headers.insert("x-device-id", device_id.to_string().parse().unwrap());
        let outcome = require(&state, &headers).unwrap();
        assert_eq!(outcome.account_id, "acct-1");
    }
}
