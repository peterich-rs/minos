//! Background jobs framework for the Minos backend.
//!
//! Provides a unified `Job` trait, a `JobSupervisor` that manages job lifecycles,
//! and individual job implementations for background tasks like token GC,
//! outbox dispatch, approval timeouts, and retention cleaning.

pub mod agent_dispatch_worker;
pub mod approval_timeout;
pub mod audit_indexer;
pub mod health;
pub mod host_command_timeout;
pub mod job_trait;
pub mod outbox_dispatcher;
pub mod push_dispatch_worker;
pub mod refresh_token_gc;
pub mod retention_cleaner;
pub mod stale_session_sweeper;
pub mod supervisor;

use std::sync::Arc;

pub use health::{JobHealth, JobHealthRegistry};
pub use job_trait::{Job, JobContext, JobError, JobOutcome};
pub use supervisor::JobSupervisor;

/// Build the default set of background jobs for the backend.
///
/// Each job declares which `RuntimeMode` it applies to via `Job::applies_to`.
pub fn default_jobs(
    realtime: Option<Arc<crate::realtime::RealtimeFanout>>,
    app: Option<Arc<crate::runtime::AppContext>>,
) -> Vec<Arc<dyn Job>> {
    let mut jobs: Vec<Arc<dyn Job>> = vec![
        Arc::new(refresh_token_gc::RefreshTokenGcJob),
        Arc::new(approval_timeout::ApprovalTimeoutJob),
        Arc::new(host_command_timeout::HostCommandTimeoutJob),
        Arc::new(retention_cleaner::RetentionCleanerJob),
        Arc::new(audit_indexer::AuditIndexerJob),
    ];

    if let Some(realtime) = realtime {
        jobs.push(outbox_dispatcher::OutboxDispatcherJob::new(Arc::clone(
            &realtime,
        )));
        jobs.push(outbox_dispatcher::HostCommandOutboxJob::new(realtime));
    }

    if let Some(app) = app {
        jobs.push(agent_dispatch_worker::AgentDispatchWorkerJob::new(
            Arc::clone(&app),
        ));
        jobs.push(push_dispatch_worker::PushDispatchWorkerJob::new(Arc::clone(
            &app,
        )));
        jobs.push(stale_session_sweeper::SessionLifecycleJob::new(app));
    }

    jobs
}
