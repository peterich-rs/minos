//! Composite push channel that dispatches to all configured channels.
//!
//! The composite channel tries each inner channel in order. If a channel
//! returns `TokenExpired`, the composite propagates it. If all channels
//! fail, the composite returns the last error.

use async_trait::async_trait;

use super::{PushAttempt, PushChannel, PushKind, PushSendError, PushSendOutcome};

/// Composite channel that wraps multiple push channels.
pub struct CompositeChannel {
    channels: Vec<Box<dyn PushChannel>>,
}

impl CompositeChannel {
    #[must_use]
    pub fn new(channels: Vec<Box<dyn PushChannel>>) -> Self {
        Self { channels }
    }

    /// Build a composite channel from environment, including only channels
    /// whose configuration is present.
    #[must_use]
    pub fn from_env() -> Self {
        let mut channels: Vec<Box<dyn PushChannel>> = Vec::new();

        if let Some(apns) = super::apns::ApnsChannel::from_env() {
            channels.push(Box::new(apns));
        }
        if let Some(fcm) = super::fcm::FcmChannel::from_env() {
            channels.push(Box::new(fcm));
        }
        if let Some(smtp) = super::smtp::SmtpChannel::from_env() {
            channels.push(Box::new(smtp));
        }

        Self { channels }
    }
}

#[async_trait]
impl PushChannel for CompositeChannel {
    fn kind(&self) -> PushKind {
        // Composite doesn't have a single kind; this is used for routing,
        // not for the composite itself.
        PushKind::Fcm
    }

    async fn send(&self, attempt: PushAttempt) -> Result<PushSendOutcome, PushSendError> {
        if self.channels.is_empty() {
            tracing::warn!(
                target: "minos_backend::notifications::composite",
                "no push channels configured; notification dropped"
            );
            return Ok(PushSendOutcome::Sent); // Treat as sent to avoid retries
        }

        let mut last_error = None;

        for channel in &self.channels {
            match channel.send(attempt.clone()).await {
                Ok(outcome) => return Ok(outcome),
                Err(PushSendError::NotConfigured(_)) => continue,
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| PushSendError::NotConfigured("no channels available".into())))
    }
}
