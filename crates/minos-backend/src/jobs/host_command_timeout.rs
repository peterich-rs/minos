use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;

/// Periodically checks for expired host commands and marks them as timed out.
///
/// This is a thin wrapper; the actual timeout logic is in `RuntimeHostCommandService`.
pub struct HostCommandTimeoutJob;

#[async_trait]
impl Job for HostCommandTimeoutJob {
    fn name(&self) -> &'static str {
        "host_command_timeout"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_millis(250)
    }

    fn tick_deadline(&self) -> Duration {
        Duration::from_secs(10)
    }

    async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        match crate::store::host_commands::list_timed_out_open(&ctx.store, now_ms, 32).await {
            Ok(rows) if rows.is_empty() => Ok(JobOutcome::Idle),
            Ok(rows) => Ok(JobOutcome::DidWork(rows.len() as u32)),
            Err(e) => Err(JobError::Transient(e.to_string())),
        }
    }
}
