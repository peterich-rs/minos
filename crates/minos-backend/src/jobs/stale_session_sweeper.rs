//! SessionLifecycle job: end dead host agent sessions + expire CompletionWatch TTL.
//!
//! Replaces the former COUNT-only stub. DidWork counts only real state changes.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use minos_domain::DeviceId;
use minos_protocol::DurableEvent;
use uuid::Uuid;

use super::job_trait::{Job, JobContext, JobError, JobOutcome};
use crate::app::tx::{DbTx, Storage as _};
use crate::config::RuntimeMode;
use crate::http::BackendState;
use crate::runtime::AppContext;
use crate::store::{
    agent_sessions, devices, durable_event_log, outbox_events, AsStorePool, StorePoolRef,
};

/// Host must be offline (no live WS) and last_seen older than this before
/// open agent sessions are forced to `failed`.
const STALE_HOST_THRESHOLD_MS: i64 = 5 * 60 * 1000;

/// Max open sessions claimed per tick.
const SESSION_BATCH: i64 = 64;

pub struct SessionLifecycleJob {
    app: Arc<AppContext>,
}

impl SessionLifecycleJob {
    #[must_use]
    pub fn new(app: Arc<AppContext>) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

/// Backward-compatible name used by `default_jobs` and docs greps.
pub type StaleSessionSweeperJob = SessionLifecycleJob;

#[async_trait]
impl Job for SessionLifecycleJob {
    fn name(&self) -> &'static str {
        "session_lifecycle"
    }

    fn applies_to(&self, mode: RuntimeMode) -> bool {
        mode.runs_supervised_workers()
    }

    fn idle_interval(&self) -> Duration {
        Duration::from_secs(15)
    }

    fn tick_deadline(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn tick(&self, _ctx: &JobContext) -> Result<JobOutcome, JobError> {
        let state = BackendState::from_app_context(Arc::clone(&self.app), None, "worker");
        let now_ms = chrono::Utc::now().timestamp_millis();

        let ended = end_stale_host_sessions(&state, now_ms)
            .await
            .map_err(|e| JobError::Transient(e.to_string()))?;

        let expired = crate::http::v1::social::expire_completion_watches(&state, now_ms)
            .await
            .map_err(|e| JobError::Transient(e.to_string()))?;

        let n = ended.saturating_add(expired);
        if n == 0 {
            Ok(JobOutcome::Idle)
        } else {
            Ok(JobOutcome::DidWork(n))
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct OpenHostSession {
    session_id: String,
    conversation_id: String,
    host_device_id: String,
    status: String,
}

/// End open formal sessions whose host is not live and last_seen is stale.
async fn end_stale_host_sessions(
    state: &BackendState,
    now_ms: i64,
) -> Result<u32, crate::error::BackendError> {
    let cutoff_ms = now_ms - STALE_HOST_THRESHOLD_MS;
    let rows = list_open_host_sessions(&state.store, SESSION_BATCH).await?;
    let mut ended = 0u32;

    for row in rows {
        let Ok(host_id) = Uuid::parse_str(&row.host_device_id).map(DeviceId) else {
            tracing::warn!(
                target: "minos_backend::session_lifecycle",
                session_id = %row.session_id,
                host = %row.host_device_id,
                "skip session with invalid host_device_id"
            );
            continue;
        };

        // Live WS ⇒ host is present; never force-end.
        if state.registry.get_host(host_id).is_some() {
            continue;
        }

        let last_seen = match devices::get_device(&state.store, host_id).await? {
            Some(dev) => dev.last_seen_at,
            None => 0,
        };
        if last_seen > cutoff_ms {
            continue;
        }

        if mark_session_failed_with_event(state, &row, now_ms).await? {
            ended = ended.saturating_add(1);
            tracing::info!(
                target: "minos_backend::session_lifecycle",
                session_id = %row.session_id,
                conversation_id = %row.conversation_id,
                host = %row.host_device_id,
                status_was = %row.status,
                last_seen_at_ms = last_seen,
                "ended stale host agent session"
            );
        }
    }

    Ok(ended)
}

async fn list_open_host_sessions(
    store: &impl AsStorePool,
    limit: i64,
) -> Result<Vec<OpenHostSession>, crate::error::BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, OpenHostSession>(
                "SELECT session_id, conversation_id, host_device_id, status
               FROM agent_sessions
              WHERE status IN ('pending', 'running', 'stopping')
                AND host_device_id IS NOT NULL
                AND ended_at_ms IS NULL
              ORDER BY started_at_ms ASC
              LIMIT ?",
            )
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, OpenHostSession>(
                "SELECT session_id, conversation_id, host_device_id, status
               FROM agent_sessions
              WHERE status IN ('pending', 'running', 'stopping')
                AND host_device_id IS NOT NULL
                AND ended_at_ms IS NULL
              ORDER BY started_at_ms ASC
              LIMIT $1",
            )
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| crate::error::BackendError::StoreQuery {
        operation: "session_lifecycle.list_open_host_sessions".into(),
        message: e.to_string(),
    })
}

/// CAS-style: only update if still open; record durable AgentSessionEnded.
async fn mark_session_failed_with_event(
    state: &BackendState,
    row: &OpenHostSession,
    now_ms: i64,
) -> Result<bool, crate::error::BackendError> {
    // Re-check status to avoid racing a clean stop/end.
    let Some(current) = agent_sessions::get(&state.store, &row.session_id).await? else {
        return Ok(false);
    };
    if !matches!(current.status.as_str(), "pending" | "running" | "stopping") {
        return Ok(false);
    }

    let mut tx = state.store.begin().await?;
    match &mut tx {
        DbTx::Sqlite(inner) => {
            let result = sqlx::query(
                "UPDATE agent_sessions
                    SET status = 'failed', ended_at_ms = ?
                  WHERE session_id = ?
                    AND status IN ('pending', 'running', 'stopping')
                    AND ended_at_ms IS NULL",
            )
            .bind(now_ms)
            .bind(&row.session_id)
            .execute(&mut **inner)
            .await
            .map_err(|e| crate::error::BackendError::StoreQuery {
                operation: "session_lifecycle.mark_failed".into(),
                message: e.to_string(),
            })?;
            if result.rows_affected() == 0 {
                return Ok(false);
            }
        }
        DbTx::Postgres(inner) => {
            let result = sqlx::query(
                "UPDATE agent_sessions
                    SET status = 'failed', ended_at_ms = $1
                  WHERE session_id = $2
                    AND status IN ('pending', 'running', 'stopping')
                    AND ended_at_ms IS NULL",
            )
            .bind(now_ms)
            .bind(&row.session_id)
            .execute(&mut **inner)
            .await
            .map_err(|e| crate::error::BackendError::StoreQuery {
                operation: "session_lifecycle.mark_failed".into(),
                message: e.to_string(),
            })?;
            if result.rows_affected() == 0 {
                return Ok(false);
            }
        }
    }

    let ended_event = DurableEvent::AgentSessionEnded {
        session_id: row.session_id.clone(),
        status: "failed".into(),
        at_ms: now_ms,
    };
    let cursor =
        durable_event_log::record_in_tx(&mut tx, &Uuid::new_v4().to_string(), &ended_event, now_ms)
            .await?;
    outbox_events::enqueue_in_tx(
        &mut tx,
        &Uuid::new_v4().to_string(),
        cursor.topic.kind().as_str(),
        &cursor.event_id,
        outbox_events::OutboxLane::SocialDurable,
        now_ms,
    )
    .await?;
    tx.commit().await?;

    // Best-effort immediate publish; outbox retries on failure.
    let _ = state
        .realtime
        .publish_durable_event_by_id(cursor.topic.kind().as_str(), &cursor.event_id)
        .await;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_link::HostLinkService;
    use crate::realtime::RealtimeConnectionRegistry;
    use crate::store::social;
    use crate::store::test_support::{insert_account, memory_pool, T0};
    use minos_domain::{DeviceId, DeviceRole};
    use std::sync::Arc;
    use std::time::Duration;

    fn state_for_pool(pool: sqlx::SqlitePool) -> BackendState {
        let registry = Arc::new(RealtimeConnectionRegistry::new());
        let host_link = Arc::new(HostLinkService::new(pool.clone()));
        BackendState::new(
            registry,
            host_link,
            pool,
            Duration::from_secs(300),
            "test-jwt-secret-32-bytes-padding!!".into(),
            None,
            "test-lifecycle".into(),
        )
    }

    async fn seed_session(
        pool: &sqlx::SqlitePool,
        account: &str,
        session_id: &str,
        host_id: &str,
        status: &str,
        ended_at: Option<i64>,
    ) {
        let conversation =
            social::create_group_conversation(pool, account, "LC", &[account.to_string()], T0)
                .await
                .unwrap();
        agent_sessions::create(
            pool,
            session_id,
            &conversation.conversation_id,
            None,
            Some(host_id),
            None,
            status,
            T0,
            ended_at,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ends_open_session_when_host_offline_and_stale() {
        let pool = memory_pool().await;
        let account = insert_account(&pool, "lifecycle@example.com").await;
        let host = DeviceId::new();
        crate::store::test_support::insert_test_host(&pool, host, "Host", T0).await;

        seed_session(
            &pool,
            &account,
            "sess_stale_1",
            &host.to_string(),
            "running",
            None,
        )
        .await;

        let state = state_for_pool(pool.clone());
        let now_ms = T0 + STALE_HOST_THRESHOLD_MS + 60_000;
        let n = end_stale_host_sessions(&state, now_ms).await.unwrap();
        assert_eq!(n, 1);

        let row = agent_sessions::get(&pool, "sess_stale_1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "failed");
        assert_eq!(row.ended_at_ms, Some(now_ms));
    }

    #[tokio::test]
    async fn skips_session_when_host_still_live_in_registry() {
        let pool = memory_pool().await;
        let account = insert_account(&pool, "lifecycle-live@example.com").await;
        let host = DeviceId::new();
        crate::store::test_support::insert_test_host(&pool, host, "Host", T0).await;

        seed_session(
            &pool,
            &account,
            "sess_live_1",
            &host.to_string(),
            "running",
            None,
        )
        .await;

        let state = state_for_pool(pool.clone());
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let conn = std::sync::Arc::new(crate::realtime::ConnectionState::new(
            minos_protocol::realtime::ConnectionPrincipal::Host {
                host_device_id: host.to_string(),
            },
            host,
            DeviceRole::AgentHost,
            tx,
            0,
        ));
        state.registry.insert(conn);

        let now_ms = T0 + STALE_HOST_THRESHOLD_MS + 60_000;
        let n = end_stale_host_sessions(&state, now_ms).await.unwrap();
        assert_eq!(n, 0);

        let row = agent_sessions::get(&pool, "sess_live_1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "running");
        assert!(row.ended_at_ms.is_none());
    }

    #[tokio::test]
    async fn does_not_end_recently_seen_offline_host() {
        let pool = memory_pool().await;
        let account = insert_account(&pool, "lifecycle-recent@example.com").await;
        let host = DeviceId::new();
        let now_ms = T0 + 10_000;
        crate::store::test_support::insert_test_host(&pool, host, "Host", now_ms).await;

        seed_session(
            &pool,
            &account,
            "sess_recent_1",
            &host.to_string(),
            "pending",
            None,
        )
        .await;

        let state = state_for_pool(pool.clone());
        let n = end_stale_host_sessions(&state, now_ms).await.unwrap();
        assert_eq!(n, 0);
        let row = agent_sessions::get(&pool, "sess_recent_1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "pending");
    }
}
