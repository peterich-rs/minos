//! Outbound JSON-RPC dispatch over `Envelope::Forward`. Spec §6.2.
//!
//! The relay envelope is opaque payload-wise — JSON-RPC `{id, method,
//! params}` lives INSIDE `Envelope::Forward { payload }`. Reply
//! correlation reads the inner JSON-RPC `id` from the payload of the
//! returning `Envelope::Forwarded { payload, .. }` frame; matching is the
//! responsibility of [`MobileClient::handle_text_frame`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use minos_domain::{DeviceId, MinosError};
use minos_protocol::Envelope;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use crate::request_trace::{self, RequestTransport};

pub struct RpcTraceContext {
    pub thread_id: Option<String>,
    pub request_summary: Option<String>,
}

/// Inner-payload-level reply matched against a pending entry by JSON-RPC id.
///
/// `Ok` carries the raw `result` value verbatim; `Err` carries a JSON-RPC
/// 2.0 error pair. The outbound caller decodes `Ok` into its declared
/// response type and maps `Err` codes to typed [`MinosError`].
#[derive(Debug)]
pub enum RpcReply {
    Ok(Value),
    Err { code: i32, message: String },
}

/// Cap on the number of in-flight forward-RPCs. Hitting the cap returns
/// `MinosError::NotConnected` rather than queueing — the connection is
/// either healthy enough to drain or it's dropped, in which case the
/// pending map is purged on the disconnect transition.
pub const PENDING_CAP: usize = 1024;

/// Outbound JSON-RPC over the relay's forward channel.
///
/// 1. Allocate a fresh id via the shared `AtomicU64`.
/// 2. Insert a oneshot sender into `pending` so the inbound `Forwarded`
///    arm can fire it.
/// 3. Build the JSON-RPC payload `{ jsonrpc: "2.0", id, method, params }`,
///    wrap in `Envelope::Forward { v: 1, payload }`, send via `outbox`.
/// 4. Await the oneshot with the per-call `timeout`. On timeout, remove
///    the pending entry and surface `MinosError::Timeout`. On disconnect,
///    the recv-loop drains pending with `RpcReply::Err { code: -32099 }`,
///    which `map_rpc_err` translates to `MinosError::RequestDropped`.
#[allow(clippy::too_many_arguments)] // Single-site RPC fanout: each arg covers one orthogonal concern (channel handles, target/method/params, timeout/trace).
pub(crate) async fn forward_rpc<P: Serialize, R: DeserializeOwned + 'static>(
    pending: &DashMap<u64, oneshot::Sender<RpcReply>>,
    next_id: &AtomicU64,
    outbox: &mpsc::Sender<Envelope>,
    target_device_id: DeviceId,
    method: &str,
    params: P,
    timeout: Duration,
    trace: Option<RpcTraceContext>,
) -> Result<R, MinosError> {
    if pending.len() >= PENDING_CAP {
        return Err(MinosError::NotConnected);
    }
    let trace_id = trace.as_ref().map(|ctx| {
        request_trace::start(
            RequestTransport::Rpc,
            method,
            format!("rpc:{method}"),
            ctx.thread_id.clone(),
            ctx.request_summary.clone(),
        )
    });
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    pending.insert(id, tx);

    let params_value = serde_json::to_value(&params).map_err(|e| MinosError::BackendInternal {
        message: format!("serialize {method} params: {e}"),
    })?;
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params_value,
    });
    let env = Envelope::Forward {
        version: 1,
        target_device_id,
        payload,
    };

    if outbox.send(env).await.is_err() {
        pending.remove(&id);
        if let Some(trace_id) = trace_id {
            request_trace::finish_failure(trace_id, None, "outbox send failed");
        }
        return Err(MinosError::NotConnected);
    }

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(RpcReply::Ok(v))) => {
            // The daemon's `()`-returning RPCs reply with `null` (or
            // sometimes an empty object); both paths must coerce to `()`
            // gracefully so callers wired to `Result<(), _>` don't blow
            // up. Try the strict decode first; fall back to "treat as
            // unit if the type tag matches".
            let result = serde_json::from_value::<R>(v.clone()).or_else(|e| {
                if std::any::TypeId::of::<R>() == std::any::TypeId::of::<()>() {
                    // Safe: we just confirmed R == () so the empty value
                    // is the correct decode.
                    serde_json::from_value::<R>(serde_json::Value::Null).map_err(|e2| {
                        MinosError::BackendInternal {
                            message: format!(
                                "decode {method} reply (unit fallback): {e2}; original: {e}"
                            ),
                        }
                    })
                } else {
                    Err(MinosError::BackendInternal {
                        message: format!("decode {method} reply: {e}; payload: {v}"),
                    })
                }
            });
            match result {
                Ok(value) => {
                    if let Some(trace_id) = trace_id {
                        let response_summary = summarize_value(&v);
                        request_trace::finish_success(trace_id, None, Some(response_summary), None);
                    }
                    Ok(value)
                }
                Err(error) => {
                    if let Some(trace_id) = trace_id {
                        request_trace::finish_failure(trace_id, None, error.to_string());
                    }
                    Err(error)
                }
            }
        }
        Ok(Ok(RpcReply::Err { code, message })) => {
            let error = map_rpc_err(method, code, message);
            if let Some(trace_id) = trace_id {
                request_trace::finish_failure(trace_id, None, error.to_string());
            }
            Err(error)
        }
        Ok(Err(_)) => {
            // The oneshot sender was dropped without a value. This means
            // the recv loop closed pending without sending — which is the
            // disconnect path that already mapped to RequestDropped via
            // the drain-helper-in-progress. But we can also land here if
            // the pending entry was removed externally. Either way the
            // request is no longer in flight.
            if let Some(trace_id) = trace_id {
                request_trace::finish_failure(trace_id, None, "pending reply dropped");
            }
            Err(MinosError::RequestDropped)
        }
        Err(_) => {
            pending.remove(&id);
            if let Some(trace_id) = trace_id {
                request_trace::finish_failure(trace_id, None, "request timed out");
            }
            Err(MinosError::Timeout)
        }
    }
}

/// Drain every pending entry with `RpcReply::Err { code: -32099 }`. Called
/// from every disconnect transition so callers awaiting a forward_rpc see
/// `MinosError::RequestDropped` rather than hanging until their per-call
/// timeout fires. The exact error code is private to this module — see
/// [`map_rpc_err`].
pub(crate) fn drain_pending(pending: &DashMap<u64, oneshot::Sender<RpcReply>>) {
    let keys: Vec<u64> = pending.iter().map(|e| *e.key()).collect();
    for k in keys {
        if let Some((_, tx)) = pending.remove(&k) {
            let _ = tx.send(RpcReply::Err {
                code: REQUEST_DROPPED_CODE,
                message: "request dropped (connection closed)".into(),
            });
        }
    }
}

/// Synthetic JSON-RPC error code for "the connection went away while
/// this request was in flight". Sits in the JSON-RPC implementation-
/// defined range (-32000 to -32099). Spec §6.2.
pub(crate) const REQUEST_DROPPED_CODE: i32 = -32099;

/// Map a JSON-RPC error pair (code + message) to a typed `MinosError`.
/// Codes line up with the daemon's `rpc_server.rs` mappings; unknown
/// codes fall through to `RpcCallFailed` carrying the original method.
fn map_rpc_err(method: &str, code: i32, message: String) -> MinosError {
    match code {
        REQUEST_DROPPED_CODE => MinosError::RequestDropped,
        -32001 => MinosError::DeviceNotTrusted { device_id: message },
        -32002 => MinosError::AgentAlreadyRunning,
        -32003 => MinosError::AgentNotRunning,
        -32004 => MinosError::AgentSessionIdMismatch,
        _ => MinosError::RpcCallFailed {
            method: method.into(),
            message,
        },
    }
}

fn summarize_value(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Array(items) => format!("items={}", items.len()),
        Value::Object(map) => {
            if let Some(session_id) = map.get("session_id").and_then(Value::as_str) {
                format!("session_id={session_id}")
            } else if let Some(cwd) = map.get("cwd").and_then(Value::as_str) {
                format!("cwd={cwd}")
            } else {
                format!("keys={}", map.len())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn forward_rpc_timeout_removes_pending() {
        let pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>> = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let (tx, _rx) = mpsc::channel::<Envelope>(8);
        let target = DeviceId::new();
        let res: Result<Value, _> = forward_rpc(
            &pending,
            &next_id,
            &tx,
            target,
            "minos_health",
            serde_json::Value::Null,
            Duration::from_millis(50),
            None,
        )
        .await;
        assert!(matches!(res, Err(MinosError::Timeout)));
        assert!(pending.is_empty(), "timeout must remove the pending entry");
    }

    #[tokio::test]
    async fn forward_rpc_emits_envelope_forward_with_jsonrpc_payload() {
        let pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>> = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(7));
        let (tx, mut rx) = mpsc::channel::<Envelope>(8);
        let target = DeviceId::new();
        let _join = tokio::spawn({
            let pending = pending.clone();
            let next_id = next_id.clone();
            async move {
                let _: Result<Value, _> = forward_rpc(
                    &pending,
                    &next_id,
                    &tx,
                    target,
                    "minos_test_method",
                    serde_json::json!({"foo": "bar"}),
                    Duration::from_millis(200),
                    None,
                )
                .await;
            }
        });
        let env = rx.recv().await.expect("envelope must be sent");
        match env {
            Envelope::Forward {
                version,
                target_device_id,
                payload,
            } => {
                assert_eq!(version, 1);
                assert_eq!(target_device_id, target);
                assert_eq!(payload["jsonrpc"], "2.0");
                assert_eq!(payload["method"], "minos_test_method");
                assert_eq!(payload["id"], 7);
                assert_eq!(payload["params"]["foo"], "bar");
            }
            other => panic!("expected Envelope::Forward, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forward_rpc_returns_not_connected_when_outbox_is_dropped() {
        let pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>> = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let (tx, rx) = mpsc::channel::<Envelope>(8);
        drop(rx);
        let res: Result<Value, _> = forward_rpc(
            &pending,
            &next_id,
            &tx,
            DeviceId::new(),
            "minos_health",
            serde_json::Value::Null,
            Duration::from_secs(5),
            None,
        )
        .await;
        assert!(matches!(res, Err(MinosError::NotConnected)));
        assert!(
            pending.is_empty(),
            "send-failure must remove the pending entry"
        );
    }

    #[tokio::test]
    async fn drain_pending_fires_request_dropped_on_every_entry() {
        let pending: DashMap<u64, oneshot::Sender<RpcReply>> = DashMap::new();
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        pending.insert(10, tx1);
        pending.insert(11, tx2);

        drain_pending(&pending);

        assert!(pending.is_empty());
        let r1 = rx1.await.unwrap();
        let r2 = rx2.await.unwrap();
        match r1 {
            RpcReply::Err { code, .. } => assert_eq!(code, REQUEST_DROPPED_CODE),
            other => panic!("expected Err, got {other:?}"),
        }
        match r2 {
            RpcReply::Err { code, .. } => assert_eq!(code, REQUEST_DROPPED_CODE),
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forward_rpc_decodes_unit_reply_from_null() {
        let pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>> = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let (tx, mut rx) = mpsc::channel::<Envelope>(8);

        let pending_clone = pending.clone();
        let next_id_clone = next_id.clone();
        let join = tokio::spawn(async move {
            let res: Result<(), _> = forward_rpc(
                &pending_clone,
                &next_id_clone,
                &tx,
                DeviceId::new(),
                "minos_stop_agent",
                serde_json::Value::Null,
                Duration::from_secs(1),
                None,
            )
            .await;
            res
        });

        // Pull the outbound frame so we can extract the id used.
        let env = rx.recv().await.expect("envelope must be sent");
        let id = match env {
            Envelope::Forward { payload, .. } => payload["id"].as_u64().unwrap(),
            other => panic!("expected Forward, got {other:?}"),
        };

        // Fire a Null result — the unit-fallback path must accept it.
        if let Some((_, tx)) = pending.remove(&id) {
            let _ = tx.send(RpcReply::Ok(serde_json::Value::Null));
        }
        let result = join.await.unwrap();
        assert!(result.is_ok(), "unit decode must succeed: {result:?}");
    }

    #[test]
    fn map_rpc_err_codes_translate_to_typed_variants() {
        match map_rpc_err("foo", REQUEST_DROPPED_CODE, "x".into()) {
            MinosError::RequestDropped => {}
            other => panic!("expected RequestDropped, got {other:?}"),
        }
        match map_rpc_err("foo", -32002, "x".into()) {
            MinosError::AgentAlreadyRunning => {}
            other => panic!("expected AgentAlreadyRunning, got {other:?}"),
        }
        match map_rpc_err("foo", -32999, "boom".into()) {
            MinosError::RpcCallFailed { method, message } => {
                assert_eq!(method, "foo");
                assert_eq!(message, "boom");
            }
            other => panic!("expected RpcCallFailed, got {other:?}"),
        }
    }

    /// Regression: hitting the in-flight-RPC cap must surface
    /// `MinosError::NotConnected` rather than allocating a new pending
    /// entry. The cap is `PENDING_CAP` (1024); if we let it grow further
    /// the recv-loop drain on disconnect could trigger a long lock-storm
    /// over DashMap's shards.
    #[tokio::test]
    async fn forward_rpc_returns_not_connected_when_pending_is_full() {
        let pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>> = Arc::new(DashMap::new());
        for k in 0..(PENDING_CAP as u64) {
            let (tx, _rx) = oneshot::channel::<RpcReply>();
            pending.insert(k, tx);
        }
        assert_eq!(pending.len(), PENDING_CAP);

        let next_id = Arc::new(AtomicU64::new(PENDING_CAP as u64 + 1));
        let (tx, _rx) = mpsc::channel::<Envelope>(8);
        let res: Result<Value, _> = forward_rpc(
            &pending,
            &next_id,
            &tx,
            DeviceId::new(),
            "minos_health",
            serde_json::Value::Null,
            Duration::from_secs(5),
            None,
        )
        .await;
        assert!(matches!(res, Err(MinosError::NotConnected)));
        assert_eq!(
            pending.len(),
            PENDING_CAP,
            "cap-rejected request must not allocate a pending entry"
        );
    }

    /// Regression: when `drain_pending` runs while a `forward_rpc` is
    /// still awaiting its oneshot, the future must resolve with
    /// `MinosError::RequestDropped` (the drain path's mapped error), not
    /// fall through to `Timeout`. The per-call timeout in this test is
    /// well above the time `drain_pending` takes so a Timeout result
    /// would indicate a regression in the drain wiring.
    #[tokio::test]
    async fn forward_rpc_returns_request_dropped_when_pending_is_drained_externally() {
        let pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>> = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let (tx, _rx) = mpsc::channel::<Envelope>(8);

        let pending_for_call = pending.clone();
        let next_id_for_call = next_id.clone();
        let join = tokio::spawn(async move {
            // 5s is plenty above the time drain_pending takes; if the
            // future resolves with Timeout it indicates a regression in
            // the drain wiring, not a too-short timeout.
            let res: Result<Value, _> = forward_rpc(
                &pending_for_call,
                &next_id_for_call,
                &tx,
                DeviceId::new(),
                "minos_health",
                serde_json::Value::Null,
                Duration::from_secs(5),
                None,
            )
            .await;
            res
        });

        // Yield until forward_rpc has parked itself in the pending map.
        for _ in 0..100 {
            if !pending.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!pending.is_empty(), "forward_rpc must park before drain");

        drain_pending(&pending);

        let res = join.await.unwrap();
        assert!(matches!(res, Err(MinosError::RequestDropped)));
    }
}
