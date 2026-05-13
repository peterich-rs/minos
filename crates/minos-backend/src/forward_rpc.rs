use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::Envelope;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use crate::error::BackendError;
use crate::session::{SessionHandle, SessionRegistry};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

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
    let (requester, mut requester_rx) =
        SessionHandle::new(DeviceId::new(), DeviceRole::MobileClient);
    if let Some(account_id) = registry
        .get(target_device_id)
        .and_then(|handle| handle.account_id())
    {
        requester.set_account_id(account_id);
    }
    let requester_id = requester.device_id;
    registry.insert(requester.clone());

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

        registry
            .route(requester_id, target_device_id, payload)
            .await?;

        let response_payload = tokio::time::timeout(timeout, async {
            loop {
                match requester_rx.recv().await {
                    Some(Envelope::Forwarded { payload, .. })
                        if payload.get("id").and_then(Value::as_u64) == Some(request_id) =>
                    {
                        return Ok(payload);
                    }
                    Some(_) => {}
                    None => {
                        return Err(BackendError::ForwardRpc {
                            method: method.to_string(),
                            message: "temporary forwarded rpc channel closed".into(),
                        });
                    }
                }
            }
        })
        .await
        .map_err(|_| BackendError::ForwardRpcTimeout {
            method: method.to_string(),
            timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        })??;

        parse_response(method, response_payload)
    }
    .await;

    let _ = registry.remove_current(&requester);
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
