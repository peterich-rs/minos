use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use minos_domain::DeviceId;
use minos_protocol::ApprovalDecisionRequest;
use serde_json::Value;
use tokio::sync::Notify;

use crate::app::tx::{DbTx, Storage};
use crate::error::BackendError;
use crate::realtime::{DurableEvent, RealtimeFanout};
use crate::store::{durable_event_log, host_commands, outbox_events, AsStorePool, StoreHandle};

const FORWARD_TIMEOUT: Duration = Duration::from_secs(5);
const APPROVAL_COMMAND_METHOD: &str = "minos_approval_decision";
const COMMAND_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct NewHostCommand {
    pub command_id: String,
    pub host_device_id: DeviceId,
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
    realtime: Option<Arc<RealtimeFanout>>,
    notify: Notify,
}

impl RuntimeHostCommandService {
    #[must_use]
    pub fn new(store: StoreHandle) -> Arc<Self> {
        Self::new_with_timeout_worker_and_realtime(store, true, None)
    }

    /// Timeout lifecycle is owned solely by [`crate::jobs::host_command_timeout::HostCommandTimeoutJob`]
    /// via [`expire_open_timed_out_commands`]. The `_enable_timeout_worker` flag is retained for
    /// call-site stability but no longer spawns a private poller.
    pub fn new_with_timeout_worker_and_realtime(
        store: StoreHandle,
        _enable_timeout_worker: bool,
        realtime: Option<Arc<RealtimeFanout>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            realtime,
            notify: Notify::new(),
        })
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
        host_device_id: DeviceId,
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
                    host_device_id,
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
            self.dispatch_outbox_once();
        }
        Ok(())
    }

    pub async fn enqueue(
        &self,
        command_id: &str,
        host_device_id: DeviceId,
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
                host_device_id,
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
        self.dispatch_outbox_once();
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
            command.host_device_id,
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
            host_device_id: command.host_device_id.to_string(),
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
            outbox_events::OutboxLane::HostCommand,
            command.created_at_ms,
        )
        .await?;

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
            if row.host_device_id != target_device_id {
                return Err(BackendError::ForwardRpc {
                    method: method.to_string(),
                    message: format!(
                        "host command {command_id} belongs to {} not {target_device_id}",
                        row.host_device_id
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
        // Same order as job path: dead-letter outbox before mark_timed_out when deadline elapsed.
        if expire_command_if_deadline_passed(
            &self.store,
            command_id,
            &timeout_error,
            finished_at_ms,
        )
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
        if row.host_device_id != target_device_id {
            return Err(BackendError::ForwardRpc {
                method: method.to_string(),
                message: format!(
                    "host command {command_id} belongs to {} not {target_device_id}",
                    row.host_device_id
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

    fn dispatch_outbox_once(&self) {
        let Some(realtime) = self.realtime.clone() else {
            return;
        };
        tokio::spawn(async move {
            // Host commands live on the host_command lane; also drain social so a
            // single wake remains a general post-commit pipeline nudge.
            if let Err(error) = realtime.dispatch_host_command_outbox_batch().await {
                tracing::warn!(
                    target: "minos_backend::host_commands",
                    error = %error,
                    "host command outbox wake failed"
                );
            }
            if let Err(error) = realtime.dispatch_outbox_batch().await {
                tracing::warn!(
                    target: "minos_backend::host_commands",
                    error = %error,
                    "social outbox wake failed"
                );
            }
        });
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

        // Durable host-topic outbox is the sole delivery path; wait for host observation.
        self.wait_for_terminal_response(command_id, host_device_id, method, timeout)
            .await
    }

    async fn enqueue_in_tx(
        &self,
        tx: &mut DbTx<'_>,
        command: NewHostCommand,
    ) -> Result<(), BackendError> {
        self.enqueue_new_command_in_tx(tx, command).await
    }
}

/// Expire a single host command when its DB deadline has elapsed.
///
/// Outbox settlement rule (observation wins):
/// - **Host observed** (`ack_at_ms` / non-timeout host terminal) → success-ack outbox
///   (never dead-letter), then `mark_timed_out` if the command is still unfinished.
/// - **Not observed** → dead-letter host_command outbox first, then `mark_timed_out`.
///
/// Returns `true` when the command was newly marked timed out.
pub async fn expire_command_if_deadline_passed(
    store: &impl AsStorePool,
    command_id: &str,
    timeout_error: &Value,
    now_ms: i64,
) -> Result<bool, BackendError> {
    let Some(row) = host_commands::get(store, command_id).await? else {
        return Ok(false);
    };
    if row.finished_at_ms.is_some() {
        return Ok(false);
    }
    if row.deadline_at_ms > now_ms {
        return Ok(false);
    }

    if row.is_host_observed() {
        // Delivery was confirmed; settle outbox as acked even though we time out the RPC wait.
        match outbox_events::ack_pending_host_command_events(store, command_id, now_ms).await {
            Ok(0) => {}
            Ok(n) => {
                tracing::debug!(
                    target: "minos_backend::host_commands",
                    command_id = %command_id,
                    acked = n,
                    "acked host command outbox on deadline after host observation"
                );
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::host_commands",
                    command_id = %command_id,
                    error = %error,
                    "failed to ack observed host command outbox at deadline"
                );
            }
        }
    } else {
        // No host observation: dead-letter before mark so concurrent ack_pending cannot
        // success-ack after finished_at_ms is set with kind=timeout.
        match outbox_events::dead_letter_host_command_events(
            store,
            command_id,
            now_ms,
            &serde_json::json!({
                "kind": "host_command_expired",
                "command_id": command_id,
            }),
        )
        .await
        {
            Ok(0) => {}
            Ok(n) => {
                crate::telemetry::increment_host_command_outbox_expired();
                tracing::warn!(
                    target: "minos_backend::host_commands",
                    command_id = %command_id,
                    dead_lettered = n,
                    "dead-lettered expired host command outbox rows"
                );
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::host_commands",
                    command_id = %command_id,
                    error = %error,
                    "failed to dead-letter expired host command outbox"
                );
            }
        }
    }

    host_commands::mark_timed_out(store, command_id, timeout_error, now_ms).await
}

/// Sweep open host commands past deadline (single owner for job + wait path).
///
/// Returns the number of commands newly marked timed out.
pub async fn expire_open_timed_out_commands(
    store: &impl AsStorePool,
    now_ms: i64,
    limit: u32,
) -> Result<u32, BackendError> {
    let rows = host_commands::list_timed_out_open(store, now_ms, limit).await?;
    let mut expired = 0u32;
    for row in rows {
        let error_json = timeout_error_json(
            u64::try_from(now_ms.saturating_sub(row.created_at_ms)).unwrap_or(u64::MAX),
        );
        if expire_command_if_deadline_passed(store, &row.command_id, &error_json, now_ms).await? {
            expired = expired.saturating_add(1);
            crate::telemetry::increment_host_command_timeout();
        }
    }
    Ok(expired)
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
