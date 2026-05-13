use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use dashmap::DashMap;
use minos_domain::DeviceId;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::error::BackendError;
use crate::session::SessionRegistry;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

struct PendingForwardRpc {
    target_device_id: DeviceId,
    response_tx: oneshot::Sender<Value>,
}

fn pending_forward_rpcs() -> &'static DashMap<u64, PendingForwardRpc> {
    static PENDING: OnceLock<DashMap<u64, PendingForwardRpc>> = OnceLock::new();
    PENDING.get_or_init(DashMap::new)
}

fn register_pending_forward_rpc(
    request_id: u64,
    target_device_id: DeviceId,
) -> oneshot::Receiver<Value> {
    let (response_tx, response_rx) = oneshot::channel();
    pending_forward_rpcs().insert(
        request_id,
        PendingForwardRpc {
            target_device_id,
            response_tx,
        },
    );
    response_rx
}

fn cancel_pending_forward_rpc(request_id: u64) {
    let _ = pending_forward_rpcs().remove(&request_id);
}

pub(crate) fn resolve_pending_forward_rpc(
    host_device_id: DeviceId,
    request_id: u64,
    payload: Value,
) -> bool {
    let Some((_, pending)) = pending_forward_rpcs().remove(&request_id) else {
        return false;
    };

    if pending.target_device_id != host_device_id {
        pending_forward_rpcs().insert(request_id, pending);
        return false;
    }

    pending.response_tx.send(payload).is_ok()
}

pub async fn call_host<P, R>(
    registry: &SessionRegistry,
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
    let started_at = std::time::Instant::now();

    let result = async {
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
        let response_rx = register_pending_forward_rpc(request_id, target_device_id);

        registry
            .route(requester_id, target_device_id, payload)
            .await?;

        let response_payload = tokio::time::timeout(timeout, response_rx)
            .await
            .map_err(|_| BackendError::ForwardRpcTimeout {
                method: method.to_string(),
                timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            })?
            .map_err(|_| BackendError::ForwardRpc {
                method: method.to_string(),
                message: "pending forwarded rpc channel closed".into(),
            })?;

        parse_response(method, response_payload)
    }
    .await;

    if result.is_err() {
        cancel_pending_forward_rpc(request_id);
    }
    let outcome = match &result {
        Ok(_) => "ok",
        Err(BackendError::ForwardRpcTimeout { .. }) => "timeout",
        Err(BackendError::PeerBackpressure { .. }) => "peer_backpressure",
        Err(BackendError::PeerOffline { .. }) => "peer_offline",
        Err(_) => "error",
    };
    crate::telemetry::record_forward_rpc(method, outcome, started_at.elapsed().as_secs_f64());
    result
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope;
    use crate::session::SessionHandle;
    use crate::store::test_support::memory_pool;
    use minos_domain::DeviceRole;
    use minos_protocol::Envelope;
    use serde_json::json;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn call_host_does_not_add_temporary_session_to_registry() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let host_id = DeviceId::new();
        let (host, mut host_rx) = SessionHandle::new(host_id, DeviceRole::AgentHost);
        registry.insert(host.clone());

        let (request_seen_tx, request_seen_rx) = oneshot::channel();
        let (reply_release_tx, reply_release_rx) = oneshot::channel();
        let responder_registry = registry.clone();
        let responder_host = host.clone();
        let responder = tokio::spawn(async move {
            let frame = host_rx
                .recv()
                .await
                .expect("host should receive forwarded rpc");
            let Envelope::Forwarded { from, payload, .. } = frame else {
                panic!("expected forwarded frame");
            };
            let request_id = payload
                .get("id")
                .and_then(Value::as_u64)
                .expect("request should carry json-rpc id");
            request_seen_tx.send((from, request_id)).unwrap();
            reply_release_rx.await.unwrap();

            let reply = json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "result": { "ok": true }
            });
            let handled =
                envelope::handle_forward(&responder_host, &responder_registry, &pool, from, reply)
                    .await;
            assert!(
                handled.is_none(),
                "server-side rpc reply should not synthesize a frame"
            );
        });

        let caller_registry = registry.clone();
        let call = tokio::spawn(async move {
            call_host::<_, Value>(
                &caller_registry,
                host_id,
                "minos.test.forward_rpc",
                &json!({ "ping": true }),
                Duration::from_secs(1),
            )
            .await
            .expect("call_host should receive host response")
        });

        let (_from, _request_id) = request_seen_rx.await.unwrap();
        assert_eq!(
            registry.len(),
            1,
            "server-side forwarded rpc must not inflate live session count"
        );

        reply_release_tx.send(()).unwrap();
        let result = call.await.unwrap();
        assert_eq!(result, json!({ "ok": true }));
        responder.await.unwrap();
    }
}
