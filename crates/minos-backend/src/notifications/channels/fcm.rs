//! Firebase Cloud Messaging (FCM) HTTP v1 channel.
//!
//! Uses a service account JWT for authentication. Configuration is
//! read from environment variables:
//!
//! - `MINOS_PUSH_FCM_PROJECT_ID` — GCP project ID
//! - `MINOS_PUSH_FCM_SERVICE_ACCOUNT_JSON` — path to service account JSON
//!
//! Currently a stub implementation that logs instead of sending.

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
        let service_account_path =
            std::env::var("MINOS_PUSH_FCM_SERVICE_ACCOUNT_JSON").ok()?;

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
        // TODO: Implement real FCM send using the FCM HTTP v1 API.
        //
        // Steps:
        // 1. Load service account JSON from self.service_account_path
        // 2. Create a signed JWT for Google OAuth2
        // 3. Exchange JWT for an access token
        // 4. Build FCM v1 request:
        //    POST https://fcm.googleapis.com/v1/projects/{project_id}/messages:send
        //    with the device token and notification payload
        // 5. Map response:
        //    - 200 => Ok(PushSendOutcome::Sent)
        //    - 404/UNREGISTERED => Ok(PushSendOutcome::TokenExpired)
        //    - 429 => Ok(PushSendOutcome::RateLimited)
        //    - Other => Err(PushSendError::Provider(...))
        //
        // For now, log and return Sent.
        tracing::debug!(
            target: "minos_backend::notifications::fcm",
            account_id = %attempt.account_id,
            token_hash = %attempt.token_hash,
            project_id = %self.project_id,
            "FCM push stub: would send notification"
        );
        Ok(PushSendOutcome::Sent)
    }
}
