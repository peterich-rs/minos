use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;
use crate::realtime::RealtimeFanout;

/// Social durable outbox lane (`social_durable`): publish → ack, no host wait.
///
/// Host commands run on [`HostCommandOutboxJob`] so a backlog of host RPCs cannot
/// block chat fanout or trip this job's tick deadline.
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
        // Default 500ms floor; wake Notify short-circuits for low-latency publish.
        // Override via MINOS_OUTBOX_IDLE_MS (clamped 100..=5000).
        let ms = std::env::var("MINOS_OUTBOX_IDLE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(500)
            .clamp(100, 5_000);
        Duration::from_millis(ms)
    }

    fn tick_deadline(&self) -> Duration {
        // Social path has no serial host wait; 10s covers a full 64-row publish batch.
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

/// Host command outbox lane (`host_command`): publish without blocking wait_ack.
///
/// Settlement is async via gateway `HostCommandAck` / `HostCommandResult` →
/// `ack_pending_host_command_events`. Expired commands are dead-lettered (never
/// success-acked).
pub struct HostCommandOutboxJob {
    realtime: Arc<RealtimeFanout>,
}

impl HostCommandOutboxJob {
    #[must_use]
    pub fn new(realtime: Arc<RealtimeFanout>) -> Arc<Self> {
        Arc::new(Self { realtime })
    }
}

#[async_trait]
impl Job for HostCommandOutboxJob {
    fn name(&self) -> &'static str {
        "host_command_outbox"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        let ms = std::env::var("MINOS_OUTBOX_IDLE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(500)
            .clamp(100, 5_000);
        Duration::from_millis(ms)
    }

    fn tick_deadline(&self) -> Duration {
        // Publish-only (no serial wait); batch is small. Keep headroom for bus RTT.
        Duration::from_secs(15)
    }

    async fn tick(&self, _ctx: &JobContext) -> Result<JobOutcome, JobError> {
        match self.realtime.dispatch_host_command_outbox_batch().await {
            Ok(0) => Ok(JobOutcome::Idle),
            Ok(n) => Ok(JobOutcome::DidWork(n as u32)),
            Err(error) => Err(JobError::Transient(error.to_string())),
        }
    }
}
