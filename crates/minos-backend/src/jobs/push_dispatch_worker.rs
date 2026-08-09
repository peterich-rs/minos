//! Background drain of [`crate::store::push_dispatch_queue`].
//!
//! Durable publish only enqueues; this worker claims, dispatches, and records
//! success in `push_dispatch_log` with retry/backoff on transient failures.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::config::RuntimeMode;
use crate::realtime::event::{DurableEvent, DurableEventEnvelope};
use crate::runtime::AppContext;
use crate::store::push_dispatch_queue;

pub struct PushDispatchWorkerJob {
    app: Arc<AppContext>,
}

impl PushDispatchWorkerJob {
    #[must_use]
    pub fn new(app: Arc<AppContext>) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

#[async_trait]
impl Job for PushDispatchWorkerJob {
    fn name(&self) -> &'static str {
        "push_dispatch_worker"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn tick_deadline(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let worker_id = format!("push-{}", ctx.instance_id);
        let claimed = push_dispatch_queue::claim_due(&self.app.store, now_ms, 32, &worker_id)
            .await
            .map_err(|e| JobError::Transient(e.to_string()))?;
        if claimed.is_empty() {
            return Ok(JobOutcome::Idle);
        }

        let mut processed = 0u32;
        for row in claimed {
            if process_row(&self.app, &row, now_ms).await {
                processed = processed.saturating_add(1);
            }
        }
        if processed == 0 {
            Ok(JobOutcome::Idle)
        } else {
            Ok(JobOutcome::DidWork(processed))
        }
    }
}

/// Returns true when the row reached a terminal or requeued outcome without
/// supervisor-level failure.
async fn process_row(
    app: &AppContext,
    row: &push_dispatch_queue::PushDispatchRow,
    now_ms: i64,
) -> bool {
    let payload: DurableEvent = match serde_json::from_str(&row.payload_json) {
        Ok(p) => p,
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::notifications",
                event_id = %row.event_id,
                queue_id = %row.queue_id,
                error = %error,
                "push queue row has undecodable payload; marking dead"
            );
            let _ = push_dispatch_queue::mark_dead(
                &app.store,
                &row.queue_id,
                &format!("undecodable payload: {error}"),
                now_ms,
            )
            .await;
            return true;
        }
    };

    let envelope = DurableEventEnvelope {
        topic: row.topic.clone(),
        topic_seq: row.topic_seq,
        event_id: row.event_id.clone(),
        payload,
    };

    match app
        .notifications
        .dispatch_for_account(&envelope, &row.account_id)
        .await
    {
        Ok(crate::notifications::AccountDispatchOutcome::Sent) => {
            let _ = push_dispatch_queue::mark_sent(&app.store, &row.queue_id, None, now_ms).await;
            true
        }
        Ok(crate::notifications::AccountDispatchOutcome::Skipped { reason }) => {
            let _ =
                push_dispatch_queue::mark_skipped(&app.store, &row.queue_id, &reason, now_ms).await;
            true
        }
        Ok(crate::notifications::AccountDispatchOutcome::Transient { reason }) => {
            if row.attempts >= push_dispatch_queue::MAX_ATTEMPTS {
                let _ = push_dispatch_queue::mark_dead(
                    &app.store,
                    &row.queue_id,
                    &format!("max attempts: {reason}"),
                    now_ms,
                )
                .await;
            } else {
                let next = now_ms + push_dispatch_queue::backoff_delay_ms(row.attempts);
                let _ = push_dispatch_queue::requeue_pending(
                    &app.store,
                    &row.queue_id,
                    row.attempts,
                    next,
                    &reason,
                )
                .await;
            }
            true
        }
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::notifications",
                event_id = %row.event_id,
                account_id = %row.account_id,
                error = %error,
                "push dispatch error; requeue"
            );
            if row.attempts >= push_dispatch_queue::MAX_ATTEMPTS {
                let _ = push_dispatch_queue::mark_dead(
                    &app.store,
                    &row.queue_id,
                    &error.to_string(),
                    now_ms,
                )
                .await;
            } else {
                let next = now_ms + push_dispatch_queue::backoff_delay_ms(row.attempts);
                let _ = push_dispatch_queue::requeue_pending(
                    &app.store,
                    &row.queue_id,
                    row.attempts,
                    next,
                    &error.to_string(),
                )
                .await;
            }
            true
        }
    }
}
