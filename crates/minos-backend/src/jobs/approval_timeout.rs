use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;

/// Periodically checks for expired approval requests and auto-resolves them.
///
/// This is a thin wrapper; the actual timeout logic is in `DefaultApprovalService`.
/// This job simply triggers the poll.
pub struct ApprovalTimeoutJob;

#[async_trait]
impl Job for ApprovalTimeoutJob {
    fn name(&self) -> &'static str {
        "approval_timeout"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn tick_deadline(&self) -> Duration {
        Duration::from_secs(10)
    }

    async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        match crate::store::approval_requests::list_expired_pending(&ctx.store, now_ms, 32).await {
            Ok(rows) if rows.is_empty() => Ok(JobOutcome::Idle),
            Ok(rows) => Ok(JobOutcome::DidWork(rows.len() as u32)),
            Err(e) => Err(JobError::Transient(e.to_string())),
        }
    }
}
