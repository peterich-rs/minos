//! Push fanout background job.
//!
//! Polls the outbox for push-eligible events and dispatches notifications
//! via the NotificationService. Runs as a supervised background worker
//! when the runtime mode supports it.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::notifications::NotificationService;
use crate::realtime::event::DurableEventEnvelope;
use crate::store::{durable_event_log, outbox_events, StoreHandle};

/// Worker ID for the push fanout job.
const WORKER_ID: &str = "push-fanout-worker";

/// Batch size for each outbox poll.
const BATCH_SIZE: u32 = 32;

/// Delay between idle polls (no events to process).
const IDLE_DELAY: Duration = Duration::from_millis(500);

/// Delay after an error before retrying.
const ERROR_DELAY: Duration = Duration::from_secs(1);

/// Maximum attempts before dead-lettering an outbox row.
const MAX_ATTEMPTS: u32 = 5;

/// Spawn the push fanout background job. Returns a JoinHandle that can
/// be aborted on shutdown.
pub fn spawn(
    store: StoreHandle,
    notification_service: Arc<dyn NotificationService>,
    enable: bool,
) -> Option<JoinHandle<()>> {
    if !enable {
        return None;
    }

    Some(tokio::spawn(async move {
        tracing::info!(
            target: "minos_backend::jobs::push_fanout",
            "push fanout worker started"
        );

        loop {
            match tick(&store, &notification_service).await {
                Ok(0) => tokio::time::sleep(IDLE_DELAY).await,
                Ok(count) => {
                    tracing::debug!(
                        target: "minos_backend::jobs::push_fanout",
                        count,
                        "processed push fanout batch"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::jobs::push_fanout",
                        error = %error,
                        "push fanout tick failed"
                    );
                    tokio::time::sleep(ERROR_DELAY).await;
                }
            }
        }
    }))
}

/// Process a single batch of outbox events for push notification dispatch.
async fn tick(
    store: &StoreHandle,
    notification_service: &Arc<dyn NotificationService>,
) -> Result<usize, crate::error::BackendError> {
    let now_ms = chrono::Utc::now().timestamp_millis();

    let claimed = outbox_events::claim_available(store, WORKER_ID, now_ms, BATCH_SIZE).await?;
    let count = claimed.len();

    for row in claimed {
        // Fetch the durable event
        let durable = match durable_event_log::get(store, &row.topic_kind, &row.event_id).await {
            Ok(Some(durable)) => durable,
            Ok(None) => {
                dead_letter(store, &row.outbox_id, now_ms, "missing durable event").await;
                continue;
            }
            Err(error) => {
                requeue_or_dead(store, &row, &error.to_string(), now_ms).await;
                continue;
            }
        };

        // Parse the durable event payload into a DurableEvent
        let event = match serde_json::from_value::<crate::realtime::event::DurableEvent>(durable.payload_json.clone()) {
            Ok(event) => event,
            Err(error) => {
                requeue_or_dead(store, &row, &error.to_string(), now_ms).await;
                continue;
            }
        };
        let envelope = DurableEventEnvelope {
            topic: durable.topic.clone(),
            topic_seq: durable.topic_seq,
            event_id: durable.event_id.clone(),
            payload: event,
        };

        // Dispatch via the notification service
        match notification_service.dispatch_for_event(&envelope).await {
            Ok(outcome) => {
                tracing::debug!(
                    target: "minos_backend::jobs::push_fanout",
                    event_id = %row.event_id,
                    ?outcome,
                    "push fanout dispatch outcome"
                );
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::jobs::push_fanout",
                    event_id = %row.event_id,
                    error = %error,
                    "push fanout dispatch error"
                );
            }
        }

        // Ack the outbox row regardless of dispatch outcome — we don't want
        // to keep retrying push failures indefinitely.
        let _ = outbox_events::ack(store, &row.outbox_id, now_ms).await;
    }

    Ok(count)
}

async fn dead_letter(store: &StoreHandle, outbox_id: &str, now_ms: i64, message: &str) {
    let error_json = serde_json::json!({ "message": message });
    if let Err(e) = outbox_events::dead_letter(store, outbox_id, now_ms, &error_json).await {
        tracing::warn!(
            target: "minos_backend::jobs::push_fanout",
            error = %e,
            outbox_id,
            "failed to dead-letter outbox row"
        );
    }
}

async fn requeue_or_dead(
    store: &StoreHandle,
    row: &outbox_events::OutboxEventRow,
    message: &str,
    now_ms: i64,
) {
    if row.attempts >= MAX_ATTEMPTS {
        dead_letter(store, &row.outbox_id, now_ms, message).await;
        return;
    }

    let retry_at_ms = now_ms + 1_000; // Retry after 1 second
    let error_json = serde_json::json!({ "message": message });
    if let Err(e) = outbox_events::retry(store, &row.outbox_id, retry_at_ms, &error_json).await {
        tracing::warn!(
            target: "minos_backend::jobs::push_fanout",
            error = %e,
            outbox_id = %row.outbox_id,
            "failed to requeue outbox row"
        );
    }
}
