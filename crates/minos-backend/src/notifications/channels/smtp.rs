//! SMTP email notification channel.
//!
//! Uses `lettre` with rustls for TLS. Configuration is read from
//! environment variables:
//!
//! - `MINOS_PUSH_SMTP_URL` — SMTP server URL (e.g. smtps://smtp.gmail.com:465)
//! - `MINOS_PUSH_SMTP_FROM` — sender email address
//!
//! Currently a stub implementation that logs instead of sending.

use async_trait::async_trait;

use super::{PushAttempt, PushChannel, PushKind, PushSendError, PushSendOutcome};

/// SMTP notification channel. This is an email-based channel and uses
/// `PushKind::Fcm` as a placeholder kind since SMTP isn't a push kind
/// per se — it delivers via email rather than device push.
pub struct SmtpChannel {
    _smtp_url: String,
    _from_address: String,
}

impl SmtpChannel {
    /// Create from environment variables. Returns `None` if required vars
    /// are missing.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let smtp_url = std::env::var("MINOS_PUSH_SMTP_URL").ok()?;
        let from_address = std::env::var("MINOS_PUSH_SMTP_FROM").ok()?;

        Some(Self {
            _smtp_url: smtp_url,
            _from_address: from_address,
        })
    }
}

#[async_trait]
impl PushChannel for SmtpChannel {
    fn kind(&self) -> PushKind {
        // SMTP isn't a device push kind; we use Fcm as a placeholder.
        // In practice, SMTP is used for email fallback, not device push.
        PushKind::Fcm
    }

    async fn send(&self, attempt: PushAttempt) -> Result<PushSendOutcome, PushSendError> {
        // TODO: Implement real SMTP send using lettre.
        //
        // Steps:
        // 1. Build lettre Message with HTML + plain text bodies
        // 2. Create SmtpTransport with rustls
        // 3. Send the message
        // 4. Map response:
        //    - Success => Ok(PushSendOutcome::Sent)
        //    - Permanent error => Ok(PushSendOutcome::TokenExpired)
        //    - Rate limit => Ok(PushSendOutcome::RateLimited)
        //
        // For now, log and return Sent.
        tracing::debug!(
            target: "minos_backend::notifications::smtp",
            account_id = %attempt.account_id,
            "SMTP push stub: would send email notification"
        );
        Ok(PushSendOutcome::Sent)
    }
}
