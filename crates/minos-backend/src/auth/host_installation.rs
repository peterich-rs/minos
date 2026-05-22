//! Steady-state host installation bearer credential extractor.
//!
//! Formal host rail requests authenticate with the opaque
//! `host_installation_token` issued by `/v1/host/pairing/redeem`. The legacy
//! `X-Device-*` bundle is intentionally not accepted here.

use axum::http::{HeaderMap, StatusCode};
use minos_domain::{DeviceId, DeviceRole};

use crate::http::BackendState;
use crate::store::{devices, host_installation_tokens};

#[derive(Debug, Clone)]
pub struct HostInstallationPrincipal {
    pub host_installation_id: DeviceId,
}

#[derive(Debug)]
pub enum HostInstallationAuthError {
    Missing,
    Invalid,
    Internal(String),
}

impl HostInstallationAuthError {
    pub fn into_response_tuple(self) -> (StatusCode, String) {
        match self {
            Self::Missing => (
                StatusCode::UNAUTHORIZED,
                "missing host installation token".into(),
            ),
            Self::Invalid => (
                StatusCode::UNAUTHORIZED,
                "invalid host installation token".into(),
            ),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        }
    }
}

pub async fn require(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<HostInstallationPrincipal, HostInstallationAuthError> {
    let token = bearer_token(headers)?;
    let token_hash = crate::pairing::sha256_hex(token);
    let row = host_installation_tokens::verify_active_token(
        &state.store,
        &token_hash,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|error| HostInstallationAuthError::Internal(error.to_string()))?
    .ok_or(HostInstallationAuthError::Invalid)?;

    let host = devices::get_device(&state.store, row.host_installation_id)
        .await
        .map_err(|error| HostInstallationAuthError::Internal(error.to_string()))?
        .ok_or(HostInstallationAuthError::Invalid)?;
    if host.role != DeviceRole::AgentHost {
        return Err(HostInstallationAuthError::Invalid);
    }

    Ok(HostInstallationPrincipal {
        host_installation_id: host.device_id,
    })
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, HostInstallationAuthError> {
    let raw = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or(HostInstallationAuthError::Missing)?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .ok_or(HostInstallationAuthError::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::test_support::backend_state;

    #[tokio::test]
    async fn missing_authorization_is_rejected() {
        let state = backend_state().await;
        let headers = HeaderMap::new();
        let err = require(&state, &headers).await.unwrap_err();
        assert!(matches!(err, HostInstallationAuthError::Missing));
    }
}
