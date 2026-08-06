//! Firebase Cloud Messaging (FCM) HTTP v1 channel.
//!
//! Config hooks (env) for production wiring when ops secrets land:
//!
//! - `MINOS_PUSH_FCM_PROJECT_ID` — GCP project ID
//! - `MINOS_PUSH_FCM_SERVICE_ACCOUNT_JSON` — path to service account JSON
//!
//! **P5 status: BLOCKED on ops secrets + FCM HTTP v1 integration.**
//! `from_env` validates config presence; `send` returns
//! [`PushSendOutcome::NotWired`] and never pretends delivery succeeded.

use async_trait::async_trait;

use super::{PushAttempt, PushChannel, PushKind, PushSendError, PushSendOutcome};

/// FCM push channel. Reads configuration from environment on construction.
pub struct FcmChannel {
    project_id: String,
    _service_account_path: String,
}

impl FcmChannel {
    /// Create from environment variables. Returns `None` if required vars
    /// are missing.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let project_id = std::env::var("MINOS_PUSH_FCM_PROJECT_ID").ok()?;
        let service_account_path = std::env::var("MINOS_PUSH_FCM_SERVICE_ACCOUNT_JSON").ok()?;

        Some(Self {
            project_id,
            _service_account_path: service_account_path,
        })
    }
}

#[async_trait]
impl PushChannel for FcmChannel {
    fn kind(&self) -> PushKind {
        PushKind::Fcm
    }

    async fn send(&self, attempt: PushAttempt) -> Result<PushSendOutcome, PushSendError> {
        // Production path (when unblocked):
        // 1. Load service account JSON
        // 2. OAuth2 access token
        // 3. POST https://fcm.googleapis.com/v1/projects/{project_id}/messages:send
        // 4. Map response → Sent / TokenExpired / RateLimited / Provider error
        tracing::warn!(
            target: "minos_backend::notifications::fcm",
            account_id = %attempt.account_id,
            token_hash = %attempt.token_hash,
            project_id = %self.project_id,
            "FCM channel NotWired: config present but production send not implemented (P5 BLOCKED on ops secrets)"
        );
        Ok(PushSendOutcome::NotWired)
    }
}
