use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;
use crate::realtime::RealtimeFanout;

/// Wraps the existing `RealtimeFanout` outbox dispatch loop as a Job.
///
/// The actual dispatch logic remains in `RealtimeFanout::dispatch_outbox_batch`.
/// This job simply calls it on each tick.
pub struct OutboxDispatcherJob {
    realtime: Arc<RealtimeFanout>,
}

impl OutboxDispatcherJob {
    #[must_use]
    pub fn new(realtime: Arc<RealtimeFanout>) -> Arc<Self> {
        Arc::new(Self { realtime })
    }
}

#[async_trait]
impl Job for OutboxDispatcherJob {
    fn name(&self) -> &'static str {
        "outbox_dispatcher"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_millis(100)
    }

    fn tick_deadline(&self) -> Duration {
        Duration::from_secs(10)
    }

    async fn tick(&self, _ctx: &JobContext) -> Result<JobOutcome, JobError> {
        match self.realtime.dispatch_outbox_batch().await {
            Ok(0) => Ok(JobOutcome::Idle),
            Ok(n) => Ok(JobOutcome::DidWork(n as u32)),
            Err(error) => Err(JobError::Transient(error.to_string())),
        }
    }
}
