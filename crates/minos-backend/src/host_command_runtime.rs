use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use dashmap::DashMap;
use minos_domain::DeviceId;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Notify};

use crate::error::BackendError;
use crate::session::SessionRegistry;
use crate::store::{host_commands, AsStorePool, StoreHandle};

const POLLER_RETRY_DELAY: Duration = Duration::from_secs(1);
const LATE_REPLY_GRACE_MS: i64 = 30_000;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

struct PendingHostCommandRpc {
    command_id: String,
    target_device_id: DeviceId,
    method: String,
    deadline_at_ms: i64,
    response_tx: oneshot::Sender<Value>,
}

fn pending_host_commands() -> &'static DashMap<u64, PendingHostCommandRpc> {
    static PENDING: OnceLock<DashMap<u64, PendingHostCommandRpc>> = OnceLock::new();
    PENDING.get_or_init(DashMap::new)
}

fn register_pending_host_command(
    request_id: u64,
    command_id: &str,
    target_device_id: DeviceId,
    method: &str,
    deadline_at_ms: i64,
) -> oneshot::Receiver<Value> {
    let (response_tx, response_rx) = oneshot::channel();
    pending_host_commands().insert(
        request_id,
        PendingHostCommandRpc {
            command_id: command_id.to_string(),
            target_device_id,
            method: method.to_string(),
            deadline_at_ms,
            response_tx,
        },
    );
    response_rx
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

#[derive(Debug)]
pub struct HostCommandRuntime {
    store: StoreHandle,
    registry: Arc<SessionRegistry>,
    notify: Notify,
}

impl HostCommandRuntime {
    pub fn new(store: impl Into<StoreHandle>, registry: Arc<SessionRegistry>) -> Arc<Self> {
        Self::new_with_timeout_worker(store, registry, true)
    }

    pub fn new_with_timeout_worker(
        store: impl Into<StoreHandle>,
        registry: Arc<SessionRegistry>,
        enable_timeout_worker: bool,
    ) -> Arc<Self> {
        let store = store.into();
        let runtime = Arc::new(Self {
            store,
            registry,
            notify: Notify::new(),
        });
        if enable_timeout_worker {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let weak = Arc::downgrade(&runtime);
                handle.spawn(async move {
                    timeout_poller(weak).await;
                });
            }
        }
        runtime
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
            host_commands::enqueue(
                &self.store,
                command_id,
                host_installation_id,
                agent_session_id,
                method,
                params_json,
                requested_by_account_id,
                deadline_at_ms,
                created_at_ms,
            )
            .await?;
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
        host_commands::enqueue(
            &self.store,
            command_id,
            host_installation_id,
            agent_session_id,
            method,
            params_json,
            requested_by_account_id,
            deadline_at_ms,
            created_at_ms,
        )
        .await?;
        self.notify.notify_one();
        Ok(())
    }

    pub async fn dispatch<P, R>(
        &self,
        command_id: &str,
        target_device_id: DeviceId,
        method: &str,
        params: &P,
        timeout: Duration,
    ) -> Result<R, BackendError>
    where
        P: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let requester_id = DeviceId::new();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let deadline_at_ms = now_ms.saturating_add(i64::try_from(timeout_ms).unwrap_or(i64::MAX));
        let params_value =
            serde_json::to_value(params).map_err(|error| BackendError::ForwardRpc {
                method: method.to_string(),
                message: format!("failed to serialize params: {error}"),
            })?;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params_value,
        });
        let response_rx = register_pending_host_command(
            request_id,
            command_id,
            target_device_id,
            method,
            deadline_at_ms,
        );

        if let Err(error) = self
            .registry
            .route(requester_id, target_device_id, payload)
            .await
        {
            cancel_pending_host_command(request_id);
            let _ = host_commands::finish(
                &self.store,
                command_id,
                host_commands::HostCommandTerminalStatus::Failed,
                None,
                Some(&transport_error_json(&error)),
                chrono::Utc::now().timestamp_millis(),
            )
            .await?;
            return Err(error);
        }

        let response_payload = match tokio::time::timeout(timeout, response_rx).await {
            Ok(Ok(payload)) => payload,
            Ok(Err(_)) => {
                let error = BackendError::ForwardRpc {
                    method: method.to_string(),
                    message: "pending host command channel closed".into(),
                };
                let _ = host_commands::finish(
                    &self.store,
                    command_id,
                    host_commands::HostCommandTerminalStatus::Failed,
                    None,
                    Some(&transport_error_json(&error)),
                    chrono::Utc::now().timestamp_millis(),
                )
                .await?;
                return Err(error);
            }
            Err(_) => {
                let error = timeout_error_json(timeout_ms);
                let _ = host_commands::mark_timed_out(
                    &self.store,
                    command_id,
                    &error,
                    chrono::Utc::now().timestamp_millis(),
                )
                .await?;
                return Err(BackendError::ForwardRpcTimeout {
                    method: method.to_string(),
                    timeout_ms,
                });
            }
        };

        parse_response(method, response_payload)
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

    pending.response_tx.send(payload).is_ok()
}

async fn timeout_poller(runtime: Weak<HostCommandRuntime>) {
    loop {
        let Some(runtime) = runtime.upgrade() else {
            break;
        };
        if let Err(error) = runtime.poll_timed_out_commands().await {
            tracing::warn!(
                target: "minos_backend::host_command_runtime",
                error = %error,
                "host command timeout poller iteration failed"
            );
        }
        tokio::select! {
            _ = tokio::time::sleep(POLLER_RETRY_DELAY) => {}
            _ = runtime.notify.notified() => {}
        }
    }
}

fn parse_response<R>(method: &str, response_payload: Value) -> Result<R, BackendError>
where
    R: DeserializeOwned,
{
    if let Some(error) = response_payload.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown forwarded rpc error");
        return Err(BackendError::ForwardRpc {
            method: method.to_string(),
            message: format!("json-rpc error {code}: {message}"),
        });
    }

    let result = response_payload
        .get("result")
        .cloned()
        .unwrap_or(Value::Null);
    serde_json::from_value(result).map_err(|error| BackendError::ForwardRpc {
        method: method.to_string(),
        message: format!("invalid forwarded rpc response: {error}"),
    })
}

fn timeout_error_json(timeout_ms: u64) -> Value {
    json!({
        "kind": "timeout",
        "timeout_ms": timeout_ms,
    })
}

fn transport_error_json(error: &BackendError) -> Value {
    let kind = match error {
        BackendError::PeerOffline { .. } => "peer_offline",
        BackendError::PeerBackpressure { .. } => "peer_backpressure",
        BackendError::ForwardRpcTimeout { .. } => "timeout",
        _ => "dispatch_error",
    };
    json!({
        "kind": kind,
        "message": error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope;
    use crate::session::SessionHandle;
    use crate::store::devices::insert_device;
    use crate::store::test_support::{memory_pool, T0};
    use minos_domain::DeviceRole;
    use minos_protocol::Envelope;
    use serde_json::json;

    #[tokio::test]
    async fn dispatch_finishes_host_command_from_host_reply() {
        let pool = memory_pool().await;
        let registry = Arc::new(SessionRegistry::new());
        let runtime = HostCommandRuntime::new(pool.clone(), Arc::clone(&registry));
        let host_id = DeviceId::new();
        insert_device(&pool, host_id, "host", DeviceRole::AgentHost, T0)
            .await
            .unwrap();

        runtime
            .enqueue(
                "cmd-host-runtime-1",
                host_id,
                None,
                "minos.test.host_command",
                &json!({ "ok": true }),
                None,
                T0 + 5_000,
                T0,
            )
            .await
            .unwrap();

        let (host, mut host_rx) = SessionHandle::new(host_id, DeviceRole::AgentHost);
        registry.insert(host.clone());

        let pool_for_reply = pool.clone();
        let registry_for_reply = Arc::clone(&registry);
        let host_for_reply = host.clone();
        let responder = tokio::spawn(async move {
            let frame = host_rx.recv().await.expect("host should receive command");
            let Envelope::Forwarded { from, payload, .. } = frame else {
                panic!("expected forwarded command");
            };
            let request_id = payload
                .get("id")
                .and_then(Value::as_u64)
                .expect("request should carry json-rpc id");
            let handled = envelope::handle_forward(
                &host_for_reply,
                &registry_for_reply,
                &pool_for_reply,
                from,
                json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": { "session_id": "sess-1" }
                }),
            )
            .await;
            assert!(handled.is_none());
        });

        let result = runtime
            .dispatch::<_, Value>(
                "cmd-host-runtime-1",
                host_id,
                "minos.test.host_command",
                &json!({ "ok": true }),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(result, json!({ "session_id": "sess-1" }));
        responder.await.unwrap();

        let row = host_commands::get(&pool, "cmd-host-runtime-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, host_commands::HostCommandStatus::Succeeded);
        assert_eq!(row.response_json, Some(json!({ "session_id": "sess-1" })));
    }
}
