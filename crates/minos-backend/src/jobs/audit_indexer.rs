use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;

/// Archives old audit events. Currently a no-op stub.
///
/// In the future, this will move old audit_events to an S3-backed archive
/// or cold storage. For now, it simply reports idle on every tick.
pub struct AuditIndexerJob;

#[async_trait]
impl Job for AuditIndexerJob {
    fn name(&self) -> &'static str {
        "audit_indexer"
    }

    fn applies_to(&self, _mode: RuntimeMode) -> bool {
        // Not yet implemented; disabled for all modes.
        false
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_secs(3600)
    }

    async fn tick(&self, _ctx: &JobContext) -> Result<JobOutcome, JobError> {
        // No-op until S3 sink is implemented.
        Ok(JobOutcome::Idle)
    }
}
