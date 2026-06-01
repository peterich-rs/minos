use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use super::health::JobHealthRegistry;
use super::job_trait::{Job, JobContext, JobError, JobOutcome};

/// Maximum consecutive failures before a job is stopped.
const MAX_CONSECUTIVE_FATALS: u32 = 5;

/// Base backoff duration for transient errors.
const BASE_BACKOFF: Duration = Duration::from_secs(1);

/// Maximum backoff duration.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Manages a set of background jobs, spawning a tokio task for each.
pub struct JobSupervisor {
    handles: Vec<JoinHandle<()>>,
    health: JobHealthRegistry,
}

impl JobSupervisor {
    /// Spawn all jobs that apply to the given runtime mode.
    pub fn spawn_all(
        jobs: Vec<Arc<dyn Job>>,
        ctx: Arc<JobContext>,
        mode: crate::config::RuntimeMode,
    ) -> Self {
        let health = JobHealthRegistry::new();
        let mut handles = Vec::new();

        for job in jobs {
            if !job.applies_to(mode) {
                continue;
            }
            health.register(job.name());
            let ctx = Arc::clone(&ctx);
            let health_clone = health.clone();
            let job_name = job.name();
            let idle_interval = job.idle_interval();
            let tick_deadline = job.tick_deadline();

            let handle = tokio::spawn(async move {
                run_job_loop(job, ctx, health_clone, idle_interval, tick_deadline).await;
            });
            handles.push(handle);

            tracing::info!(
                target: "minos_backend::jobs",
                job = job_name,
                "spawned background job"
            );
        }

        Self { handles, health }
    }

    /// Get the health registry for reporting.
    pub fn health(&self) -> &JobHealthRegistry {
        &self.health
    }

    /// Abort all running jobs.
    pub fn abort_all(&self) {
        for handle in &self.handles {
            handle.abort();
        }
    }
}

async fn run_job_loop(
    job: Arc<dyn Job>,
    ctx: Arc<JobContext>,
    health: JobHealthRegistry,
    idle_interval: Duration,
    tick_deadline: Duration,
) {
    let name = job.name();
    let mut consecutive_backoff = 0u32;

    loop {
        let tick_start = tokio::time::Instant::now();

        let result = tokio::time::timeout(tick_deadline, job.tick(&ctx)).await;

        let now_ms = chrono::Utc::now().timestamp_millis();

        match result {
            Err(_timeout) => {
                tracing::warn!(
                    target: "minos_backend::jobs",
                    job = name,
                    timeout_secs = tick_deadline.as_secs(),
                    "job tick timed out"
                );
                health.record_failure(name, "tick timed out");
                consecutive_backoff += 1;
            }
            Ok(Err(error)) => {
                match &error {
                    JobError::Transient(msg) => {
                        tracing::warn!(
                            target: "minos_backend::jobs",
                            job = name,
                            error = %msg,
                            "job tick transient error"
                        );
                        consecutive_backoff += 1;
                    }
                    JobError::Fatal(msg) => {
                        tracing::error!(
                            target: "minos_backend::jobs",
                            job = name,
                            error = %msg,
                            "job tick fatal error"
                        );
                        consecutive_backoff += 1;
                    }
                }
                health.record_failure(name, &error.to_string());

                if health.consecutive_failures(name) >= MAX_CONSECUTIVE_FATALS {
                    tracing::error!(
                        target: "minos_backend::jobs",
                        job = name,
                        failures = MAX_CONSECUTIVE_FATALS,
                        "job stopped after too many consecutive failures"
                    );
                    return;
                }
            }
            Ok(Ok(outcome)) => {
                consecutive_backoff = 0;
                health.record_success(name, now_ms);

                match outcome {
                    JobOutcome::Idle => {
                        tokio::time::sleep(idle_interval).await;
                        continue;
                    }
                    JobOutcome::DidWork(n) => {
                        tracing::debug!(
                            target: "minos_backend::jobs",
                            job = name,
                            items = n,
                            "job tick completed"
                        );
                        // Don't sleep after productive work; tick immediately.
                        continue;
                    }
                }
            }
        }

        // Apply exponential backoff on errors.
        if consecutive_backoff > 0 {
            let backoff = BASE_BACKOFF
                .saturating_mul(2u32.saturating_pow(consecutive_backoff.saturating_sub(1)))
                .min(MAX_BACKOFF);
            tokio::time::sleep(backoff).await;
        }
    }
}
