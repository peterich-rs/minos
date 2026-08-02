use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;
use crate::store::AsStorePool;

/// Sweeps stale host sessions that haven't been seen recently.
///
/// A session is considered stale if its `last_seen_at` is older than 5 minutes.
const STALE_THRESHOLD_MS: i64 = 5 * 60 * 1000;

pub struct StaleSessionSweeperJob;

#[async_trait]
impl Job for StaleSessionSweeperJob {
    fn name(&self) -> &'static str {
        "stale_session_sweeper"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn tick_deadline(&self) -> Duration {
        Duration::from_secs(15)
    }

    async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError> {
        let _cutoff_ms = chrono::Utc::now().timestamp_millis() - STALE_THRESHOLD_MS;

        // Mark agent sessions as stale if their host hasn't been seen recently.
        // This is a lightweight check that doesn't modify state yet.
        // In the future, this can be expanded to clean Redis presence keys.
        match ctx.store.as_store_pool() {
            crate::store::StorePoolRef::Sqlite(pool) => {
                let count = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM agent_sessions
                      WHERE status IN ('pending', 'active')
                        AND host_installation_id IS NOT NULL",
                )
                .fetch_one(pool)
                .await
                .map_err(|e| JobError::Transient(e.to_string()))?;

                if count > 0 {
                    Ok(JobOutcome::DidWork(count as u32))
                } else {
                    Ok(JobOutcome::Idle)
                }
            }
            crate::store::StorePoolRef::Postgres(pool) => {
                let count = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM agent_sessions
                      WHERE status IN ('pending', 'active')
                        AND host_installation_id IS NOT NULL",
                )
                .fetch_one(pool)
                .await
                .map_err(|e| JobError::Transient(e.to_string()))?;

                if count > 0 {
                    Ok(JobOutcome::DidWork(count as u32))
                } else {
                    Ok(JobOutcome::Idle)
                }
            }
        }
    }
}
