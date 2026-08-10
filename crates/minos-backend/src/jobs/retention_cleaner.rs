use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;
use crate::store::durable_event_log;

/// Cleans old entries from the durable event log and agent turn events.
///
/// Default retention: **90 days**.
/// Override via `MINOS_DURABLE_RETENTION_DAYS`. Multi-batch drain per tick.
const DEFAULT_RETENTION_DAYS: i64 = 90;
const BATCH_SIZE: u32 = 1000;
const MAX_BATCHES_PER_TICK: u32 = 10;

fn retention_window_ms() -> i64 {
    let days = std::env::var("MINOS_DURABLE_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS)
        .clamp(1, 3650);
    days * 24 * 60 * 60 * 1000
}

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
        let cutoff_ms = chrono::Utc::now().timestamp_millis() - retention_window_ms();
        let mut total_cleaned: u64 = 0;
        for _ in 0..MAX_BATCHES_PER_TICK {
            let batch =
                durable_event_log::delete_ready_for_retention(&ctx.store, cutoff_ms, BATCH_SIZE)
                    .await
                    .map_err(|e| JobError::Transient(e.to_string()))?;
            total_cleaned = total_cleaned.saturating_add(batch);
            if batch < u64::from(BATCH_SIZE) {
                break;
            }
        }
        let total_cleaned = u32::try_from(total_cleaned).map_err(|_| {
            JobError::Fatal(format!(
                "retention cleaner deleted more rows than u32 can report: {total_cleaned}"
            ))
        })?;

        if total_cleaned > 0 {
            tracing::info!(
                target: "minos_backend::jobs::retention_cleaner",
                cleaned = total_cleaned,
                cutoff_ms,
                "retention drain batch"
            );
            Ok(JobOutcome::DidWork(total_cleaned))
        } else {
            Ok(JobOutcome::Idle)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{self, StoreHandle};

    #[tokio::test]
    async fn tick_cleans_old_durable_events_on_sqlite() {
        let pool = store::test_support::memory_pool().await;
        let old_created_at_ms =
            chrono::Utc::now().timestamp_millis() - retention_window_ms() - 1_000;

        durable_event_log::append(
            &pool,
            "evt-old",
            "host:dev1",
            "host",
            1,
            "dev1",
            &serde_json::json!({ "kind": "old" }),
            old_created_at_ms,
        )
        .await
        .unwrap();

        let ctx = JobContext {
            store: StoreHandle::from(pool.clone()),
            outbox_wake: std::sync::Arc::new(tokio::sync::Notify::new()),
            agent_dispatch_wake: std::sync::Arc::new(tokio::sync::Notify::new()),
            instance_id: "test-instance".to_string(),
        };

        let outcome = RetentionCleanerJob.tick(&ctx).await.unwrap();

        assert_eq!(outcome, JobOutcome::DidWork(1));
        assert!(durable_event_log::get(&pool, "host", "evt-old")
            .await
            .unwrap()
            .is_none());
    }
}
