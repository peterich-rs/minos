//! Background drain of [`crate::store::bot_message_deliveries`].
//!
//! HTTP send only enqueues; this worker performs host RPC when a live host
//! is available and arms CompletionWatch on success.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;
use crate::http::BackendState;
use crate::runtime::AppContext;

pub struct AgentDispatchWorkerJob {
    app: Arc<AppContext>,
}

impl AgentDispatchWorkerJob {
    #[must_use]
    pub fn new(app: Arc<AppContext>) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

#[async_trait]
impl Job for AgentDispatchWorkerJob {
    fn name(&self) -> &'static str {
        "agent_dispatch_worker"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn tick_deadline(&self) -> Duration {
        // Forward may enqueue host_command; keep headroom without serial wait.
        Duration::from_secs(30)
    }

    async fn tick(&self, _ctx: &JobContext) -> Result<JobOutcome, JobError> {
        let state = BackendState::from_app_context(Arc::clone(&self.app), None, "worker");
        match crate::http::v1::social::process_agent_dispatch_batch(&state).await {
            Ok(0) => Ok(JobOutcome::Idle),
            Ok(n) => Ok(JobOutcome::DidWork(n)),
            Err(error) => Err(JobError::Transient(error.to_string())),
        }
    }
}
