use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;
use crate::host_commands::expire_open_timed_out_commands;

/// Sole owner of host-command deadline expiry.
///
/// For each open command past `deadline_at_ms`:
/// 1. dead-letter `host_command` outbox rows (metric `outbox_host_command_expired_total`)
/// 2. `mark_timed_out` on `host_commands`
///
/// Never success-acks outbox on timeout.
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
        match expire_open_timed_out_commands(&ctx.store, now_ms, 32).await {
            Ok(0) => Ok(JobOutcome::Idle),
            Ok(n) => Ok(JobOutcome::DidWork(n)),
            Err(e) => Err(JobError::Transient(e.to_string())),
        }
    }
}
