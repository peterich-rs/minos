//! Push notification channels: APNs, FCM, SMTP, and a composite dispatcher.

pub mod apns;
pub mod composite;
pub mod fcm;
pub mod smtp;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The push notification platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushKind {
    Apns,
    Fcm,
}

impl PushKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Apns => "apns",
            Self::Fcm => "fcm",
        }
    }
}

impl std::fmt::Display for PushKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for PushKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "apns" => Ok(Self::Apns),
            "fcm" => Ok(Self::Fcm),
            _ => Err(format!("unknown push kind: {s}")),
        }
    }
}

/// The payload for a push notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushPayload {
    pub title: String,
    pub body: String,
    pub category: String,
    pub data: serde_json::Value,
}

/// An attempt to send a push notification to a specific device token.
#[derive(Debug, Clone)]
pub struct PushAttempt {
    pub token_hash: String,
    pub account_id: String,
    pub payload: PushPayload,
}

/// Outcome of sending a push to a single device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushSendOutcome {
    /// Successfully enqueued / delivered.
    Sent,
    /// The device token is no longer valid (APNs BadDeviceToken, FCM Unregistered).
    /// The caller should revoke the token.
    TokenExpired,
    /// The push provider is rate-limiting us.
    RateLimited,
}

/// Error from a push channel send operation.
#[derive(Debug, thiserror::Error)]
pub enum PushSendError {
    #[error("push provider returned an error: {0}")]
    Provider(String),
    #[error("push channel not configured: {0}")]
    NotConfigured(String),
    #[error(transparent)]
    Internal(#[from] crate::error::BackendError),
}

/// Trait for a push notification channel (APNs, FCM, SMTP, etc.).
#[async_trait]
pub trait PushChannel: Send + Sync {
    /// The platform this channel handles.
    fn kind(&self) -> PushKind;

    /// Send a push notification attempt. Returns the outcome or an error.
    async fn send(&self, attempt: PushAttempt) -> Result<PushSendOutcome, PushSendError>;
}
