//! Stable DeviceId for this Mac + durable account/host credentials.

use minos_domain::DeviceId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::secure_store::{self, SecureStoreError};

const KEY_DEVICE_ID: &str = "device_id";
const KEY_REFRESH: &str = "refresh_token";
const KEY_HOST_TOKEN: &str = "host_token";
const KEY_ACCOUNT_ID: &str = "account_id";
const KEY_EMAIL: &str = "email";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopAccount {
    pub device_id: String,
    pub account_id: String,
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_token: Option<String>,
}

pub fn device_id() -> Result<DeviceId, SecureStoreError> {
    if let Some(raw) = secure_store::get(KEY_DEVICE_ID)? {
        if let Ok(uuid) = Uuid::parse_str(raw.trim()) {
            return Ok(DeviceId(uuid));
        }
    }
    let id = DeviceId::new();
    secure_store::set(KEY_DEVICE_ID, &id.to_string())?;
    Ok(id)
}

pub fn load_refresh_token() -> Result<Option<String>, SecureStoreError> {
    secure_store::get(KEY_REFRESH)
}

pub fn load_host_token() -> Result<Option<String>, SecureStoreError> {
    secure_store::get(KEY_HOST_TOKEN)
}

pub fn load_account_id() -> Result<Option<String>, SecureStoreError> {
    secure_store::get(KEY_ACCOUNT_ID)
}

pub fn load_email() -> Result<Option<String>, SecureStoreError> {
    secure_store::get(KEY_EMAIL)
}

pub fn persist_session(session: &DesktopAccount) -> Result<(), SecureStoreError> {
    secure_store::set(KEY_ACCOUNT_ID, &session.account_id)?;
    secure_store::set(KEY_EMAIL, &session.email)?;
    secure_store::set(KEY_REFRESH, &session.refresh_token)?;
    if let Some(host) = session.host_token.as_deref().filter(|s| !s.is_empty()) {
        secure_store::set(KEY_HOST_TOKEN, host)?;
        let _ =
            minos_daemon::device_secret_store::write(&minos_domain::DeviceSecret(host.to_string()));
    }
    Ok(())
}

pub fn clear_session() -> Result<(), SecureStoreError> {
    secure_store::delete(KEY_REFRESH)?;
    secure_store::delete(KEY_HOST_TOKEN)?;
    secure_store::delete(KEY_ACCOUNT_ID)?;
    secure_store::delete(KEY_EMAIL)?;
    let _ = minos_daemon::device_secret_store::delete();
    Ok(())
}
