//! Shared header extraction + auth classification for HTTP handlers.
//!
//! The remaining header-auth flows call [`authenticate`] to resolve
//! `(device_id, role)` from the `X-Device-*` header bundle.
//!
//! Installation rows are **not** created here: Postgres CHECK requires
//! `account_id` for clients and `public_key` for hosts. Clients are
//! inserted at login/register/exchange via `insert_client_for_account`;
//! hosts at bootstrap via `insert_host_with_public_key`. Device-secret
//! verification has been removed (hosts use `host_installation_tokens`;
//! clients use bearer access tokens).

use axum::http::{HeaderMap, StatusCode};
use minos_domain::{DeviceId, DeviceRole};
use std::str::FromStr;
use uuid::Uuid;

use crate::store::{self, device_installations::DeviceRow};

pub const HDR_DEVICE_ID: &str = "x-device-id";
pub const HDR_DEVICE_ROLE: &str = "x-device-role";
pub const HDR_DEVICE_SECRET: &str = "x-device-secret";
pub const HDR_DEVICE_NAME: &str = "x-device-name";

const DEFAULT_DISPLAY_NAME: &str = "unnamed";

/// Result of a successful classification.
#[derive(Debug, Clone)]
pub struct AuthOutcome {
    pub device_id: DeviceId,
    pub role: DeviceRole,
    /// `Some(secret)` if the request supplied `X-Device-Secret` AND the
    /// stored row had a hash that verified. `None` for first-connect or
    /// existing-but-no-hash rows. Used by handlers that need to distinguish
    /// steady-state host auth from first-connect bootstrap traffic.
    pub authenticated_with_secret: bool,
}

/// Auth-layer error kinds. Both variants carry an operator-facing
/// message; `Unauthorized` round-trips to HTTP 401 / WS pre-upgrade 401,
/// `Internal` round-trips to 500 / activation close 1011.
#[derive(Debug)]
pub enum AuthError {
    Unauthorized(String),
    Internal(String),
}

impl AuthError {
    pub fn into_response_tuple(self) -> (StatusCode, String) {
        match self {
            Self::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        }
    }
}

/// Parse headers, look up the installation row, classify, and return the
/// resolved `(device_id, role)`.
///
/// Does **not** insert missing rows (Postgres CHECK forbids null-account
/// clients and null-public_key hosts). Callers that create sessions must
/// insert via CHECK-compliant helpers once account / host key is known.
pub async fn authenticate(
    store: &impl store::AsStorePool,
    headers: &HeaderMap,
) -> Result<AuthOutcome, AuthError> {
    let device_id = extract_device_id(headers)?;
    let requested_role = extract_device_role(headers)?;
    let device_secret = extract_device_secret(headers);
    let provided_display_name = extract_device_name(headers)
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());

    let existing = store::device_installations::get_device(store, device_id)
        .await
        .map_err(|e| AuthError::Internal(e.to_string()))?;
    let should_backfill_display_name = existing.as_ref().is_some_and(|row| {
        let trimmed = row.display_name.trim();
        matches!(provided_display_name.as_deref(), Some(new_name) if (trimmed.is_empty() || trimmed == DEFAULT_DISPLAY_NAME) && trimmed != new_name)
    });
    let role = resolve_device_role(existing.as_ref(), requested_role)?;

    let classification = classify(existing, device_secret.as_deref(), role)?;
    let authenticated_with_secret = matches!(classification, Classification::Authenticated);

    if should_backfill_display_name {
        if let Some(new_display_name) = provided_display_name.as_deref() {
            if let Err(e) =
                store::device_installations::set_display_name(store, &device_id, new_display_name)
                    .await
            {
                tracing::warn!(
                    target: "minos_backend::http::auth",
                    error = %e,
                    device_id = %device_id,
                    "backfill device display_name failed",
                );
            }
        }
    }

    Ok(AuthOutcome {
        device_id,
        role,
        authenticated_with_secret,
    })
}

/// Same as [`authenticate`] but also asserts the resolved role equals
/// `expected`. Used by handlers that are role-gated.
pub async fn authenticate_role(
    store: &impl store::AsStorePool,
    headers: &HeaderMap,
    expected: DeviceRole,
) -> Result<AuthOutcome, AuthError> {
    let outcome = authenticate(store, headers).await?;
    if outcome.role != expected {
        return Err(AuthError::Unauthorized(format!(
            "role required: {expected}, got {}",
            outcome.role
        )));
    }
    Ok(outcome)
}

#[derive(Debug)]
pub enum Classification {
    FirstConnect,
    UnpairedExisting,
    Authenticated,
}

pub fn classify(
    row: Option<DeviceRow>,
    provided_secret: Option<&str>,
    role: DeviceRole,
) -> Result<Classification, AuthError> {
    // Device-secret rail removed with `secret_hash` column. Provided secrets
    // are ignored; host steady-state auth uses host_installation_tokens.
    let _ = (provided_secret, role);
    match row {
        None => Ok(Classification::FirstConnect),
        Some(_) => Ok(Classification::UnpairedExisting),
    }
}

pub fn extract_device_id(headers: &HeaderMap) -> Result<DeviceId, AuthError> {
    let raw = headers
        .get(HDR_DEVICE_ID)
        .ok_or_else(|| AuthError::Unauthorized("X-Device-Id required".into()))?;
    let s = raw
        .to_str()
        .map_err(|_| AuthError::Unauthorized("X-Device-Id not UTF-8".into()))?;
    Uuid::parse_str(s)
        .map(DeviceId)
        .map_err(|e| AuthError::Unauthorized(format!("X-Device-Id not a valid UUID: {e}")))
}

pub fn extract_device_role(headers: &HeaderMap) -> Result<Option<DeviceRole>, AuthError> {
    let Some(raw) = headers.get(HDR_DEVICE_ROLE) else {
        return Ok(None);
    };
    let s = raw
        .to_str()
        .map_err(|_| AuthError::Unauthorized("X-Device-Role not UTF-8".into()))?;
    DeviceRole::from_str(s)
        .map(Some)
        .map_err(|e| AuthError::Unauthorized(format!("X-Device-Role invalid: {e}")))
}

pub fn resolve_device_role(
    existing: Option<&DeviceRow>,
    requested_role: Option<DeviceRole>,
) -> Result<DeviceRole, AuthError> {
    match existing {
        Some(row) => {
            if let Some(role) = requested_role {
                if role != row.role {
                    return Err(AuthError::Unauthorized(format!(
                        "X-Device-Role mismatch for existing device: expected {}, got {}",
                        row.role, role
                    )));
                }
            }
            Ok(row.role)
        }
        None => Ok(requested_role.unwrap_or(DeviceRole::MobileClient)),
    }
}

pub fn extract_device_secret(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HDR_DEVICE_SECRET)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

pub fn extract_device_name(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HDR_DEVICE_NAME)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── header extraction ─────────────────────────────────────────────

    #[test]
    fn extract_device_id_missing_returns_401() {
        let headers = HeaderMap::new();
        let err = extract_device_id(&headers).unwrap_err();
        assert!(matches!(err, AuthError::Unauthorized(ref m) if m.contains("X-Device-Id")));
    }

    #[test]
    fn extract_device_id_non_uuid_returns_401() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ID, "not-a-uuid".parse().unwrap());
        let err = extract_device_id(&headers).unwrap_err();
        assert!(matches!(err, AuthError::Unauthorized(ref m) if m.contains("valid UUID")));
    }

    #[test]
    fn extract_device_id_valid_round_trips() {
        let id = DeviceId::new();
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ID, id.to_string().parse().unwrap());
        let got = extract_device_id(&headers).unwrap();
        assert_eq!(got, id);
    }

    #[test]
    fn extract_device_role_absent_defaults_to_ios_client() {
        let headers = HeaderMap::new();
        assert_eq!(extract_device_role(&headers).unwrap(), None);
    }

    #[test]
    fn extract_device_role_kebab_case_parses() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ROLE, "agent-host".parse().unwrap());
        assert_eq!(
            extract_device_role(&headers).unwrap(),
            Some(DeviceRole::AgentHost)
        );
    }

    #[test]
    fn extract_device_role_unknown_value_returns_401() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ROLE, "gizmo".parse().unwrap());
        let err = extract_device_role(&headers).unwrap_err();
        assert!(matches!(err, AuthError::Unauthorized(_)));
    }

    #[test]
    fn resolve_device_role_first_connect_defaults_to_ios_client() {
        let role = resolve_device_role(None, None).unwrap();
        assert_eq!(role, DeviceRole::MobileClient);
    }

    #[test]
    fn resolve_device_role_existing_row_rejects_mismatched_header() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "mac".to_string(),
            role: DeviceRole::AgentHost,
            public_key: None,
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };

        let err = resolve_device_role(Some(&row), Some(DeviceRole::MobileClient)).unwrap_err();
        assert!(matches!(err, AuthError::Unauthorized(ref m) if m.contains("mismatch")));
    }

    #[test]
    fn extract_device_secret_absent_is_none() {
        assert_eq!(extract_device_secret(&HeaderMap::new()), None);
    }

    #[test]
    fn extract_device_secret_present_returns_string() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_SECRET, "sek".parse().unwrap());
        assert_eq!(extract_device_secret(&headers), Some("sek".to_string()));
    }

    // ── classify: pure decision function ──────────────────────────────

    #[test]
    fn classify_no_row_is_first_connect() {
        let out = classify(None, None, DeviceRole::MobileClient).unwrap();
        assert!(matches!(out, Classification::FirstConnect));
    }

    #[test]
    fn classify_row_without_hash_is_unpaired_existing() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::MobileClient,
            public_key: None,
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };
        let out = classify(Some(row), None, DeviceRole::MobileClient).unwrap();
        assert!(matches!(out, Classification::UnpairedExisting));
    }

    #[test]
    fn classify_existing_row_ignores_provided_secret() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::AgentHost,
            public_key: None,
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };
        let out = classify(Some(row), Some("any-secret"), DeviceRole::AgentHost).unwrap();
        assert!(matches!(out, Classification::UnpairedExisting));
    }
}
