use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use minos_domain::DeviceId;
use minos_protocol::ApprovalDecisionRequest;
use serde_json::Value;
use tokio::sync::Notify;

use crate::app::tx::{DbTx, Storage};
use crate::error::BackendError;
use crate::realtime::DurableEvent;
use crate::session::SessionRegistry;
use crate::store::{durable_event_log, host_commands, outbox_events, AsStorePool, StoreHandle};

const FORWARD_TIMEOUT: Duration = Duration::from_secs(5);
const APPROVAL_COMMAND_METHOD: &str = "minos_approval_decision";
const COMMAND_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const POLLER_RETRY_DELAY: Duration = Duration::from_secs(1);
const LATE_REPLY_GRACE_MS: i64 = 30_000;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

struct PendingHostCommandRpc {
    command_id: String,
    target_device_id: DeviceId,
    deadline_at_ms: i64,
}

fn pending_host_commands() -> &'static DashMap<u64, PendingHostCommandRpc> {
    static PENDING: OnceLock<DashMap<u64, PendingHostCommandRpc>> = OnceLock::new();
    PENDING.get_or_init(DashMap::new)
}

fn register_pending_host_command(
    request_id: u64,
    command_id: &str,
    target_device_id: DeviceId,
    deadline_at_ms: i64,
) {
    pending_host_commands().insert(
        request_id,
        PendingHostCommandRpc {
            command_id: command_id.to_string(),
            target_device_id,
            deadline_at_ms,
        },
    );
}

fn cancel_pending_host_command(request_id: u64) {
    let _ = pending_host_commands().remove(&request_id);
}

fn clear_expired_pending_host_commands(now_ms: i64) {
    let stale_ids = pending_host_commands()
        .iter()
        .filter_map(|entry| {
            let deadline = entry.value().deadline_at_ms;
            (deadline.saturating_add(LATE_REPLY_GRACE_MS) <= now_ms).then_some(*entry.key())
        })
        .collect::<Vec<_>>();
    for request_id in stale_ids {
        let _ = pending_host_commands().remove(&request_id);
    }
}

pub(crate) async fn resolve_pending_host_command(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
    request_id: u64,
    payload: Value,
) -> bool {
    let Some((_, pending)) = pending_host_commands().remove(&request_id) else {
        return false;
    };

    if pending.target_device_id != host_device_id {
        pending_host_commands().insert(request_id, pending);
        return false;
    }

    let finished_at_ms = chrono::Utc::now().timestamp_millis();
    if let Some(error) = payload.get("error") {
        let _ = host_commands::finish(
            store,
            &pending.command_id,
            host_commands::HostCommandTerminalStatus::Failed,
            None,
            Some(error),
            finished_at_ms,
        )
        .await;
    } else {
        let result = payload.get("result").cloned().unwrap_or(Value::Null);
        let _ = host_commands::finish(
            store,
            &pending.command_id,
            host_commands::HostCommandTerminalStatus::Succeeded,
            Some(&result),
            None,
            finished_at_ms,
        )
        .await;
    }

    true
}

#[derive(Debug, Clone)]
pub struct NewHostCommand {
    pub command_id: String,
    pub host_installation_id: DeviceId,
    pub agent_session_id: Option<String>,
    pub method: String,
    pub params_json: Value,
    pub requested_by_account_id: Option<String>,
    pub deadline_at_ms: i64,
    pub created_at_ms: i64,
}

#[async_trait]
pub trait HostCommandService: Send + Sync {
    async fn dispatch_json(
        &self,
        command_id: &str,
        host_device_id: DeviceId,
        agent_session_id: Option<&str>,
        method: &str,
        params: &Value,
        requested_by_account_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, BackendError>;

    async fn enqueue_in_tx(
        &self,
        tx: &mut DbTx<'_>,
        command: NewHostCommand,
    ) -> Result<(), BackendError>;
}

pub struct RuntimeHostCommandService {
    store: StoreHandle,
    registry: Option<Arc<SessionRegistry>>,
    notify: Notify,
}

impl RuntimeHostCommandService {
    #[must_use]
    pub fn new(store: StoreHandle) -> Arc<Self> {
        Self::new_with_timeout_worker(store, None, true)
    }

    pub fn new_with_timeout_worker(
        store: StoreHandle,
        registry: Option<Arc<SessionRegistry>>,
        enable_timeout_worker: bool,
    ) -> Arc<Self> {
        let service = Arc::new(Self {
            store,
            registry,
            notify: Notify::new(),
        });
        if enable_timeout_worker {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let weak = Arc::downgrade(&service);
                handle.spawn(async move {
                    timeout_poller(weak).await;
                });
            }
        }
        service
    }

    pub async fn dispatch_approval_decision(
        &self,
        host_device_id: DeviceId,
        requested_by_account_id: Option<&str>,
        request: &ApprovalDecisionRequest,
    ) -> Result<(), BackendError> {
        let params = serde_json::to_value(request).map_err(|error| BackendError::StoreQuery {
            operation: "host_commands.dispatch_approval_decision.serialize".into(),
            message: error.to_string(),
        })?;
        self.dispatch_json(
            &format!("cmd-approval-{}", request.request_id),
            host_device_id,
            None,
            APPROVAL_COMMAND_METHOD,
            &params,
            requested_by_account_id,
            FORWARD_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    pub async fn enqueue_if_missing(
        &self,
        command_id: &str,
        host_installation_id: DeviceId,
        agent_session_id: Option<&str>,
        method: &str,
        params_json: &Value,
        requested_by_account_id: Option<&str>,
        deadline_at_ms: i64,
        created_at_ms: i64,
    ) -> Result<(), BackendError> {
        if host_commands::get(&self.store, command_id).await?.is_none() {
            let mut tx = self.store.begin().await?;
            self.enqueue_new_command_in_tx(
                &mut tx,
                NewHostCommand {
                    command_id: command_id.to_string(),
                    host_installation_id,
                    agent_session_id: agent_session_id.map(str::to_string),
                    method: method.to_string(),
                    params_json: params_json.clone(),
                    requested_by_account_id: requested_by_account_id.map(str::to_string),
                    deadline_at_ms,
                    created_at_ms,
                },
            )
            .await?;
            tx.commit().await?;
            self.notify.notify_one();
        }
        Ok(())
    }

    pub async fn enqueue(
        &self,
        command_id: &str,
        host_installation_id: DeviceId,
        agent_session_id: Option<&str>,
        method: &str,
        params_json: &Value,
        requested_by_account_id: Option<&str>,
        deadline_at_ms: i64,
        created_at_ms: i64,
    ) -> Result<(), BackendError> {
        let mut tx = self.store.begin().await?;
        self.enqueue_new_command_in_tx(
            &mut tx,
            NewHostCommand {
                command_id: command_id.to_string(),
                host_installation_id,
                agent_session_id: agent_session_id.map(str::to_string),
                method: method.to_string(),
                params_json: params_json.clone(),
                requested_by_account_id: requested_by_account_id.map(str::to_string),
                deadline_at_ms,
                created_at_ms,
            },
        )
        .await?;
        tx.commit().await?;
        self.notify.notify_one();
        Ok(())
    }

    async fn enqueue_new_command_in_tx(
        &self,
        tx: &mut DbTx<'_>,
        command: NewHostCommand,
    ) -> Result<(), BackendError> {
        host_commands::enqueue_in_tx(
            tx,
            &command.command_id,
            command.host_installation_id,
            command.agent_session_id.as_deref(),
            &command.method,
            &command.params_json,
            command.requested_by_account_id.as_deref(),
            command.deadline_at_ms,
            command.created_at_ms,
        )
        .await?;

        let durable_event = DurableEvent::HostCommandIssued {
            command_id: command.command_id,
            host_installation_id: command.host_installation_id.to_string(),
            agent_session_id: command.agent_session_id,
            method: command.method,
            params: command.params_json,
            requested_by_account_id: command.requested_by_account_id,
            deadline_at_ms: command.deadline_at_ms,
            at_ms: command.created_at_ms,
        };
        let event_id = uuid::Uuid::new_v4().to_string();
        let cursor =
            durable_event_log::record_in_tx(tx, &event_id, &durable_event, command.created_at_ms)
                .await?;
        outbox_events::enqueue_in_tx(
            tx,
            &uuid::Uuid::new_v4().to_string(),
            cursor.topic.kind().as_str(),
            &cursor.event_id,
            command.created_at_ms,
        )
        .await?;

        Ok(())
    }

    async fn try_dispatch_via_legacy_session(
        &self,
        command_id: &str,
        host_device_id: DeviceId,
        method: &str,
        params: &Value,
        request_id: u64,
        created_at_ms: i64,
        deadline_at_ms: i64,
    ) -> Result<(), BackendError> {
        let Some(registry) = &self.registry else {
            return Ok(());
        };
        if registry.get(host_device_id).is_none() {
            return Ok(());
        }

        let Some(row) = host_commands::get(&self.store, command_id).await? else {
            return Ok(());
        };
        if row.created_at_ms != created_at_ms
            || row.host_installation_id != host_device_id
            || !matches!(
                row.status,
                host_commands::HostCommandStatus::Pending | host_commands::HostCommandStatus::Acked
            )
        {
            return Ok(());
        }

        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params.clone(),
        });
        register_pending_host_command(request_id, command_id, host_device_id, deadline_at_ms);
        if let Err(error) = registry
            .route(DeviceId::new(), host_device_id, payload)
            .await
        {
            cancel_pending_host_command(request_id);
            tracing::debug!(
                target: "minos_backend::host_commands",
                command_id = %command_id,
                host_device_id = %host_device_id,
                error = %error,
                "legacy live-session host command dispatch unavailable; relying on durable delivery"
            );
        }

        Ok(())
    }

    async fn wait_for_terminal_response(
        &self,
        command_id: &str,
        target_device_id: DeviceId,
        method: &str,
        timeout: Duration,
    ) -> Result<Value, BackendError> {
        let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(tokio::time::Instant::now);

        loop {
            let row = host_commands::get(&self.store, command_id)
                .await?
                .ok_or_else(|| BackendError::ForwardRpc {
                    method: method.to_string(),
                    message: format!("host command row missing: {command_id}"),
                })?;
            if row.host_installation_id != target_device_id {
                return Err(BackendError::ForwardRpc {
                    method: method.to_string(),
                    message: format!(
                        "host command {command_id} belongs to {} not {target_device_id}",
                        row.host_installation_id
                    ),
                });
            }
            if let Some(response) = terminal_response(method, timeout_ms, &row)? {
                return Ok(response);
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }

            tokio::time::sleep(
                COMMAND_STATUS_POLL_INTERVAL.min(deadline.saturating_duration_since(now)),
            )
            .await;
        }

        let finished_at_ms = chrono::Utc::now().timestamp_millis();
        let timeout_error = timeout_error_json(timeout_ms);
        if host_commands::mark_timed_out(&self.store, command_id, &timeout_error, finished_at_ms)
            .await?
        {
            return Err(BackendError::ForwardRpcTimeout {
                method: method.to_string(),
                timeout_ms,
            });
        }

        let row = host_commands::get(&self.store, command_id)
            .await?
            .ok_or_else(|| BackendError::ForwardRpc {
                method: method.to_string(),
                message: format!("host command row missing after timeout: {command_id}"),
            })?;
        if row.host_installation_id != target_device_id {
            return Err(BackendError::ForwardRpc {
                method: method.to_string(),
                message: format!(
                    "host command {command_id} belongs to {} not {target_device_id}",
                    row.host_installation_id
                ),
            });
        }
        if let Some(response) = terminal_response(method, timeout_ms, &row)? {
            return Ok(response);
        }

        Err(BackendError::ForwardRpcTimeout {
            method: method.to_string(),
            timeout_ms,
        })
    }

    async fn poll_timed_out_commands(&self) -> Result<(), BackendError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let rows = host_commands::list_timed_out_open(&self.store, now_ms, 64).await?;
        for row in rows {
            let _ = host_commands::mark_timed_out(
                &self.store,
                &row.command_id,
                &timeout_error_json(
                    u64::try_from(now_ms.saturating_sub(row.created_at_ms)).unwrap_or(u64::MAX),
                ),
                now_ms,
            )
            .await?;
        }
        clear_expired_pending_host_commands(now_ms);
        Ok(())
    }
}

#[async_trait]
impl HostCommandService for RuntimeHostCommandService {
    async fn dispatch_json(
        &self,
        command_id: &str,
        host_device_id: DeviceId,
        agent_session_id: Option<&str>,
        method: &str,
        params: &Value,
        requested_by_account_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, BackendError> {
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let created_at_ms = chrono::Utc::now().timestamp_millis();
        let deadline_at_ms =
            created_at_ms.saturating_add(i64::try_from(timeout.as_millis()).unwrap_or(i64::MAX));

        self.enqueue_if_missing(
            command_id,
            host_device_id,
            agent_session_id,
            method,
            params,
            requested_by_account_id,
            deadline_at_ms,
            created_at_ms,
        )
        .await?;

        self.try_dispatch_via_legacy_session(
            command_id,
            host_device_id,
            method,
            params,
            request_id,
            created_at_ms,
            deadline_at_ms,
        )
        .await?;

        let result = self
            .wait_for_terminal_response(command_id, host_device_id, method, timeout)
            .await;
        if !matches!(result, Err(BackendError::ForwardRpcTimeout { .. })) {
            cancel_pending_host_command(request_id);
        }
        result
    }

    async fn enqueue_in_tx(
        &self,
        tx: &mut DbTx<'_>,
        command: NewHostCommand,
    ) -> Result<(), BackendError> {
        self.enqueue_new_command_in_tx(tx, command).await
    }
}

async fn timeout_poller(service: Weak<RuntimeHostCommandService>) {
    loop {
        let Some(service) = service.upgrade() else {
            break;
        };
        if let Err(error) = service.poll_timed_out_commands().await {
            tracing::warn!(
                target: "minos_backend::host_commands",
                error = %error,
                "host command timeout poller iteration failed"
            );
        }
        tokio::select! {
            _ = tokio::time::sleep(POLLER_RETRY_DELAY) => {}
            _ = service.notify.notified() => {}
        }
    }
}

fn terminal_response(
    method: &str,
    fallback_timeout_ms: u64,
    row: &host_commands::HostCommandRow,
) -> Result<Option<Value>, BackendError> {
    match row.status {
        host_commands::HostCommandStatus::Pending | host_commands::HostCommandStatus::Acked => {
            Ok(None)
        }
        host_commands::HostCommandStatus::Succeeded => {
            Ok(Some(row.response_json.clone().unwrap_or(Value::Null)))
        }
        host_commands::HostCommandStatus::Failed => Err(host_command_failure(
            method,
            fallback_timeout_ms,
            row.error_json.as_ref(),
        )),
    }
}

fn host_command_failure(
    method: &str,
    fallback_timeout_ms: u64,
    error_json: Option<&Value>,
) -> BackendError {
    let Some(error_json) = error_json else {
        return BackendError::ForwardRpc {
            method: method.to_string(),
            message: "host command failed without error payload".into(),
        };
    };

    if error_json.get("kind").and_then(Value::as_str) == Some("timeout") {
        return BackendError::ForwardRpcTimeout {
            method: method.to_string(),
            timeout_ms: error_json
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(fallback_timeout_ms),
        };
    }

    if let Some(message) = error_json.get("message").and_then(Value::as_str) {
        if let Some(code) = error_json.get("code").and_then(Value::as_i64) {
            return BackendError::ForwardRpc {
                method: method.to_string(),
                message: format!("json-rpc error {code}: {message}"),
            };
        }
        return BackendError::ForwardRpc {
            method: method.to_string(),
            message: message.to_string(),
        };
    }

    if let Some(kind) = error_json.get("kind").and_then(Value::as_str) {
        return BackendError::ForwardRpc {
            method: method.to_string(),
            message: kind.to_string(),
        };
    }

    BackendError::ForwardRpc {
        method: method.to_string(),
        message: error_json.to_string(),
    }
}

fn timeout_error_json(timeout_ms: u64) -> Value {
    serde_json::json!({
        "kind": "timeout",
        "timeout_ms": timeout_ms,
    })
}
