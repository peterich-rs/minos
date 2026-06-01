use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;
use crate::store::AsStorePool;

/// Cleans old entries from the durable event log and agent turn events.
///
/// Retention window: 30 days. This job deletes batches of old rows to avoid
/// long-running transactions.
const RETENTION_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const BATCH_SIZE: u32 = 1000;

pub struct RetentionCleanerJob;

#[async_trait]
impl Job for RetentionCleanerJob {
    fn name(&self) -> &'static str {
        "retention_cleaner"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_secs(3600) // Run once per hour
    }

    fn tick_deadline(&self) -> Duration {
        Duration::from_secs(60)
    }

    async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError> {
        let cutoff_ms = chrono::Utc::now().timestamp_millis() - RETENTION_WINDOW_MS;
        let mut total_cleaned = 0u32;

        match ctx.store.as_store_pool() {
            crate::store::StorePoolRef::Sqlite(pool) => {
                let result = sqlx::query(
                    "DELETE FROM durable_event_log WHERE created_at_ms < ? LIMIT ?",
                )
                .bind(cutoff_ms)
                .bind(i64::from(BATCH_SIZE))
                .execute(pool)
                .await
                .map_err(|e| JobError::Transient(e.to_string()))?;
                total_cleaned += result.rows_affected() as u32;
            }
            crate::store::StorePoolRef::Postgres(pool) => {
                let result = sqlx::query(
                    "DELETE FROM durable_event_log WHERE created_at_ms < $1 LIMIT $2",
                )
                .bind(cutoff_ms)
                .bind(i64::from(BATCH_SIZE))
                .execute(pool)
                .await
                .map_err(|e| JobError::Transient(e.to_string()))?;
                total_cleaned += result.rows_affected() as u32;
            }
        }

        if total_cleaned > 0 {
            Ok(JobOutcome::DidWork(total_cleaned))
        } else {
            Ok(JobOutcome::Idle)
        }
    }
}
