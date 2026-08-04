//! Apple Push Notification service (APNs) channel.
//!
//! Config hooks (env) for production wiring when ops secrets land:
//!
//! - `MINOS_PUSH_APNS_KEY_PATH` — path to the .p8 key file
//! - `MINOS_PUSH_APNS_KEY_ID` — Apple key ID (10 chars)
//! - `MINOS_PUSH_APNS_TEAM_ID` — Apple team ID (10 chars)
//! - `MINOS_PUSH_APNS_TOPIC` — APNs topic (bundle ID)
//! - `MINOS_PUSH_APNS_SANDBOX` — "true" for sandbox, "false" for production
//!
//! **P5 status: BLOCKED on ops secrets + provider integration.**
//! `from_env` validates config presence; `send` returns
//! [`PushSendOutcome::NotWired`] and never pretends delivery succeeded.
//! Wire real `a2` (or HTTP/2) send when Apple credentials are available.

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
        // Production path (when unblocked):
        // 1. Load .p8 key from self.key_path
        // 2. Create JWT token for APNs auth
        // 3. Build APNs request with attempt.payload
        // 4. Send to api.push.apple.com or api.development.push.apple.com
        // 5. Map response → Sent / TokenExpired / RateLimited / Provider error
        tracing::warn!(
            target: "minos_backend::notifications::apns",
            account_id = %attempt.account_id,
            token_hash = %attempt.token_hash,
            topic = %self.topic,
            sandbox = self.sandbox,
            "APNs channel NotWired: config present but production send not implemented (P5 BLOCKED on ops secrets)"
        );
        Ok(PushSendOutcome::NotWired)
    }
}
