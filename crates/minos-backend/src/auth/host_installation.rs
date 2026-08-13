//! Steady-state host installation bearer credential extractor.
//!
//! Formal host rail requests authenticate with the opaque
//! `host_installation_token` issued by `/v1/host/pairing/redeem`. The
//! `X-Device-*` bundle is intentionally not accepted here.

use axum::http::{HeaderMap, StatusCode};
use minos_domain::{DeviceId, DeviceRole};

use crate::http::BackendState;
use crate::store::{devices, host_tokens};

/// Authenticated Host rail: this Mac (`device_id`) bound to one account.
#[derive(Debug, Clone)]
pub struct HostPrincipal {
    pub device_id: DeviceId,
    pub account_id: String,
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
) -> Result<HostPrincipal, HostInstallationAuthError> {
    let token = bearer_token(headers)?;
    let token_hash = crate::host_link::sha256_hex(token);
    let row = host_tokens::verify_active_token(
        &state.store,
        &token_hash,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|error| HostInstallationAuthError::Internal(error.to_string()))?
    .ok_or(HostInstallationAuthError::Invalid)?;

    let host = devices::get_device(&state.store, row.host_device_id)
        .await
        .map_err(|error| HostInstallationAuthError::Internal(error.to_string()))?
        .ok_or(HostInstallationAuthError::Invalid)?;
    // Desktop login binds host_token to the same DeviceId as the account
    // installation (kind=desktop). Standalone daemons remain AgentHost.
    if host.role != DeviceRole::AgentHost && host.role != DeviceRole::DesktopConsole {
        return Err(HostInstallationAuthError::Invalid);
    }

    let account_id = match row.account_id.filter(|id| !id.is_empty()) {
        Some(id) => id,
        None => {
            let links =
                crate::store::host_links::list_accounts_for_host(&state.store, row.host_device_id)
                    .await
                    .map_err(|error| HostInstallationAuthError::Internal(error.to_string()))?;
            match links.as_slice() {
                [only] => only.mobile_account_id.clone(),
                _ => return Err(HostInstallationAuthError::Invalid),
            }
        }
    };

    Ok(HostPrincipal {
        device_id: host.device_id,
        account_id,
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
