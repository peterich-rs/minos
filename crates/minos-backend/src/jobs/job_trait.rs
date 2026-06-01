use std::time::Duration;

use async_trait::async_trait;

use crate::config::RuntimeMode;
use crate::store::StoreHandle;

/// Context passed to each job tick.
pub struct JobContext {
    pub store: StoreHandle,
    pub instance_id: String,
}

/// Outcome of a single job tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// The job had nothing to do.
    Idle,
    /// The job processed `n` items.
    DidWork(u32),
}

/// Error returned by a job tick.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// Transient failure; the supervisor will retry with backoff.
    #[error("transient: {0}")]
    Transient(String),
    /// Fatal failure; the supervisor will stop the job after repeated failures.
    #[error("fatal: {0}")]
    Fatal(String),
}

/// Trait for a background job that can be managed by the `JobSupervisor`.
#[async_trait]
pub trait Job: Send + Sync + 'static {
    /// Human-readable name for metrics and logging.
    fn name(&self) -> &'static str;

    /// Whether this job should run under the given runtime mode.
    fn applies_to(&self, mode: RuntimeMode) -> bool;

    /// Execute one tick of the job.
    async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError>;

    /// How long to wait between ticks when the previous tick was idle.
    fn idle_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    /// Maximum time a single tick may take before the supervisor considers it hung.
    fn tick_deadline(&self) -> Duration {
        Duration::from_secs(30)
    }

    /// Whether this job should run as a singleton (only one instance per backend).
    fn singleton_tick(&self) -> bool {
        true
    }
}
