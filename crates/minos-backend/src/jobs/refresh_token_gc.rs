use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;

/// Garbage-collects expired pairing tokens.
pub struct RefreshTokenGcJob;

#[async_trait]
impl Job for RefreshTokenGcJob {
    fn name(&self) -> &'static str {
        "refresh_token_gc"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_secs(60)
    }

    async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError> {
        let now = chrono::Utc::now().timestamp_millis();
        let rows = crate::store::tokens::gc_expired(&ctx.store, now)
            .await
            .map_err(|e| JobError::Transient(e.to_string()))?;

        if rows > 0 {
            tracing::info!(
                target: "minos_backend::jobs",
                job = self.name(),
                rows,
                "token GC removed expired rows"
            );
            Ok(JobOutcome::DidWork(rows as u32))
        } else {
            Ok(JobOutcome::Idle)
        }
    }
}
