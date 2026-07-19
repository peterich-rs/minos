//! Apple Push Notification service (APNs) channel.
//!
//! Uses token-based authentication with a .p8 key. Configuration is
//! read from environment variables:
//!
//! - `MINOS_PUSH_APNS_KEY_PATH` — path to the .p8 key file
//! - `MINOS_PUSH_APNS_KEY_ID` — Apple key ID (10 chars)
//! - `MINOS_PUSH_APNS_TEAM_ID` — Apple team ID (10 chars)
//! - `MINOS_PUSH_APNS_TOPIC` — APNs topic (bundle ID)
//! - `MINOS_PUSH_APNS_SANDBOX` — "true" for sandbox, "false" for production
//!
//! Currently a stub implementation that logs instead of sending. Replace
//! the body of `send()` with real `a2` crate calls when the dependency
//! is available.

use async_trait::async_trait;

use super::{PushAttempt, PushChannel, PushKind, PushSendError, PushSendOutcome};

/// APNs push channel. Reads configuration from environment on construction.
pub struct ApnsChannel {
    #[allow(dead_code)]
    key_path: String,
    #[allow(dead_code)]
    key_id: String,
    #[allow(dead_code)]
    team_id: String,
    topic: String,
    sandbox: bool,
}

impl ApnsChannel {
    /// Create from environment variables. Returns `None` if required vars
    /// are missing (caller should skip this channel gracefully).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let key_path = std::env::var("MINOS_PUSH_APNS_KEY_PATH").ok()?;
        let key_id = std::env::var("MINOS_PUSH_APNS_KEY_ID").ok()?;
        let team_id = std::env::var("MINOS_PUSH_APNS_TEAM_ID").ok()?;
        let topic =
            std::env::var("MINOS_PUSH_APNS_TOPIC").unwrap_or_else(|_| "com.minos.app".to_string());
        let sandbox = std::env::var("MINOS_PUSH_APNS_SANDBOX")
            .map(|v| v == "true")
            .unwrap_or(true);

        Some(Self {
            key_path,
            key_id,
            team_id,
            topic,
            sandbox,
        })
    }
}

#[async_trait]
impl PushChannel for ApnsChannel {
    fn kind(&self) -> PushKind {
        PushKind::Apns
    }

    async fn send(&self, attempt: PushAttempt) -> Result<PushSendOutcome, PushSendError> {
        // TODO: Implement real APNs send using the `a2` crate.
        //
        // Steps:
        // 1. Load .p8 key from self.key_path
        // 2. Create JWT token for APNs auth
        // 3. Build APNs request with attempt.payload
        // 4. Send to either api.push.apple.com or api.development.push.apple.com
        // 5. Map response:
        //    - 200 => Ok(PushSendOutcome::Sent)
        //    - 400 with BadDeviceToken/Unregistered => Ok(PushSendOutcome::TokenExpired)
        //    - 429 => Ok(PushSendOutcome::RateLimited)
        //    - Other => Err(PushSendError::Provider(...))
        //
        // For now, log and return Sent.
        tracing::debug!(
            target: "minos_backend::notifications::apns",
            account_id = %attempt.account_id,
            token_hash = %attempt.token_hash,
            topic = %self.topic,
            sandbox = self.sandbox,
            "APNs push stub: would send notification"
        );
        Ok(PushSendOutcome::Sent)
    }
}
