//! Shared header extraction + auth classification for HTTP handlers.
//!
//! Both `GET /devices` (WS upgrade) and the `/v1/*` REST routes call
//! [`authenticate`] to resolve `(device_id, role)` from the
//! `X-Device-*` header bundle. First-connect devices are inserted into
//! the `devices` table with `secret_hash = NULL`; existing rows are
//! verified against the supplied secret if one is stored.

use axum::http::{HeaderMap, StatusCode};
use minos_domain::{DeviceId, DeviceRole};
use sqlx::SqlitePool;
use std::str::FromStr;
use uuid::Uuid;

use crate::pairing::secret::verify_secret;
use crate::store::{
    self,
    devices::{insert_device, DeviceRow},
};

pub const HDR_DEVICE_ID: &str = "x-device-id";
pub const HDR_DEVICE_ROLE: &str = "x-device-role";
pub const HDR_DEVICE_SECRET: &str = "x-device-secret";
pub const HDR_DEVICE_NAME: &str = "x-device-name";
pub const HDR_CF_ACCESS_ID: &str = "cf-access-client-id";
pub const HDR_CF_ACCESS_SECRET: &str = "cf-access-client-secret";

const DEFAULT_DISPLAY_NAME: &str = "unnamed";

/// Result of a successful classification.
#[derive(Debug, Clone)]
pub struct AuthOutcome {
    pub device_id: DeviceId,
    pub role: DeviceRole,
    /// `Some(secret)` if the request supplied `X-Device-Secret` AND the
    /// stored row had a hash that verified. `None` for first-connect or
    /// existing-but-no-hash rows. Used by handlers that need to decide
    /// whether to allow secret-less calls (e.g. `/v1/pairing/consume`).
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

/// Parse headers, look up the device row, classify, insert on first
/// connect, and return the resolved `(device_id, role)`. Side-effecting:
/// may insert into `devices`.
pub async fn authenticate(
    pool: &SqlitePool,
    headers: &HeaderMap,
) -> Result<AuthOutcome, AuthError> {
    let device_id = extract_device_id(headers)?;
    let requested_role = extract_device_role(headers)?;
    let device_secret = extract_device_secret(headers);
    let display_name = extract_device_name(headers).unwrap_or_else(|| DEFAULT_DISPLAY_NAME.into());
    log_cf_access_presence(headers);

    let existing = store::devices::get_device(pool, device_id)
        .await
        .map_err(|e| AuthError::Internal(e.to_string()))?;
    let role = resolve_device_role(existing.as_ref(), requested_role)?;

    let classification = classify(existing, device_secret.as_deref(), role)?;
    let authenticated_with_secret = matches!(classification, Classification::Authenticated);

    if matches!(classification, Classification::FirstConnect) {
        let now = chrono::Utc::now().timestamp_millis();
        if let Err(e) = insert_device(pool, device_id, &display_name, role, now).await {
            tracing::warn!(
                target: "minos_backend::http::auth",
                error = %e,
                device_id = %device_id,
                "first-connect insert_device failed (race?)",
            );
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
    pool: &SqlitePool,
    headers: &HeaderMap,
    expected: DeviceRole,
) -> Result<AuthOutcome, AuthError> {
    let outcome = authenticate(pool, headers).await?;
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
    let _ = role; // documents that this fn is now aware of role; iOS rail
                  // (secret_hash NULL) collapses to UnpairedExisting per ADR-0020.
    match row {
        None => Ok(Classification::FirstConnect),
        Some(r) => match r.secret_hash {
            None => {
                // iOS rail: bearer-only after ADR-0020. A NULL secret_hash
                // is the steady state for iOS rows. Mac rows would only
                // be NULL pre-pair (FirstConnect-like).
                Ok(Classification::UnpairedExisting)
            }
            Some(hash) => {
                let Some(secret) = provided_secret else {
                    return Err(AuthError::Unauthorized(
                        "X-Device-Secret required for authenticated device".into(),
                    ));
                };
                match verify_secret(secret, &hash) {
                    Ok(true) => Ok(Classification::Authenticated),
                    Ok(false) => Err(AuthError::Unauthorized(
                        "X-Device-Secret does not match stored hash".into(),
                    )),
                    Err(e) => Err(AuthError::Internal(e.to_string())),
                }
            }
        },
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
        None => Ok(requested_role.unwrap_or(DeviceRole::IosClient)),
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

pub fn log_cf_access_presence(headers: &HeaderMap) {
    let cf_id = headers.contains_key(HDR_CF_ACCESS_ID);
    let cf_sec = headers.contains_key(HDR_CF_ACCESS_SECRET);
    if cf_id || cf_sec {
        tracing::debug!(
            target: "minos_backend::http::auth",
            cf_access_client_id_present = cf_id,
            cf_access_client_secret_present = cf_sec,
            "CF-Access headers observed (edge-validated; backend does not re-check)",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{devices::insert_device, test_support::memory_pool};
    use pretty_assertions::assert_eq;

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
        assert_eq!(role, DeviceRole::IosClient);
    }

    #[test]
    fn resolve_device_role_existing_row_rejects_mismatched_header() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "mac".to_string(),
            role: DeviceRole::AgentHost,
            secret_hash: None,
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };

        let err = resolve_device_role(Some(&row), Some(DeviceRole::IosClient)).unwrap_err();
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
        let out = classify(None, None, DeviceRole::IosClient).unwrap();
        assert!(matches!(out, Classification::FirstConnect));
    }

    #[test]
    fn classify_row_without_hash_is_unpaired_existing() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::IosClient,
            secret_hash: None,
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };
        let out = classify(Some(row), None, DeviceRole::IosClient).unwrap();
        assert!(matches!(out, Classification::UnpairedExisting));
    }

    #[test]
    fn classify_row_with_hash_missing_secret_is_401() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::IosClient,
            secret_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$abc$def".to_string()),
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };
        let err = classify(Some(row), None, DeviceRole::IosClient).unwrap_err();
        assert!(
            matches!(err, AuthError::Unauthorized(ref m) if m.contains("X-Device-Secret required"))
        );
    }

    #[tokio::test]
    async fn classify_row_with_hash_matching_secret_is_authenticated() {
        // Produce a real argon2id hash so verify_secret returns Ok(true).
        let plain = minos_domain::DeviceSecret::generate();
        let hash = crate::pairing::secret::hash_secret(&plain).unwrap();
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::IosClient,
            secret_hash: Some(hash),
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };
        let out = classify(Some(row), Some(plain.as_str()), DeviceRole::IosClient).unwrap();
        assert!(matches!(out, Classification::Authenticated));
    }

    #[tokio::test]
    async fn classify_row_with_hash_wrong_secret_is_401() {
        let plain = minos_domain::DeviceSecret::generate();
        let hash = crate::pairing::secret::hash_secret(&plain).unwrap();
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::IosClient,
            secret_hash: Some(hash),
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };
        let err = classify(Some(row), Some("wrong-secret"), DeviceRole::IosClient).unwrap_err();
        assert!(matches!(err, AuthError::Unauthorized(ref m) if m.contains("does not match")));
    }

    #[tokio::test]
    async fn classify_smoke_via_real_store_row() {
        // End-to-end smoke with a real pool: insert a row, fetch it, feed
        // through `classify` — exercises the DeviceRow shape from sqlx.
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "smoke", DeviceRole::AgentHost, 0)
            .await
            .unwrap();
        let row = store::devices::get_device(&pool, id)
            .await
            .unwrap()
            .unwrap();
        let out = classify(Some(row), None, DeviceRole::AgentHost).unwrap();
        assert!(matches!(out, Classification::UnpairedExisting));
    }

    #[test]
    fn classify_ios_with_null_secret_hash_passes_without_secret() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::IosClient,
            secret_hash: None,
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };
        let res = classify(Some(row), None, DeviceRole::IosClient).unwrap();
        assert!(matches!(res, Classification::UnpairedExisting));
    }

    #[test]
    fn classify_mac_with_secret_hash_but_no_secret_provided_returns_401() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "mac".to_string(),
            role: DeviceRole::AgentHost,
            secret_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$abc$def".to_string()),
            created_at: 0,
            last_seen_at: 0,
            account_id: None,
        };
        let err = classify(Some(row), None, DeviceRole::AgentHost).unwrap_err();
        assert!(matches!(err, AuthError::Unauthorized(_)));
    }
}
