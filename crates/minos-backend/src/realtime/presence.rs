//! Installation presence fanout for the formal IM gateway.
//!
//! Online is live-connection truth (`RealtimeConnectionRegistry` / active WS).
//! `last_seen_at_ms` is durable on `device_installations` and returned on
//! HTTP list endpoints. Presence pushes are **ephemeral** `StreamEvent`s
//! (`kind = presence`) — not DurableEvent log entries.

use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::realtime::{
    ConnectionPrincipal, PresencePayload, PresencePrincipalKind, RealtimeTopic,
    PRESENCE_STREAM_KIND,
};
use serde_json::Value;

use crate::http::BackendState;
use crate::store::host_links;

/// Persist last_seen and push presence StreamEvents to interested topics.
pub async fn publish_connection_presence(
    state: &BackendState,
    device_id: DeviceId,
    role: DeviceRole,
    principal: &ConnectionPrincipal,
    online: bool,
) {
    let at_ms = chrono::Utc::now().timestamp_millis();
    if let Err(error) =
        crate::store::device_installations::touch_last_seen(&state.store, &device_id, at_ms).await
    {
        tracing::debug!(
            target: "minos_backend::realtime::presence",
            error = %error,
            device_id = %device_id,
            online,
            "failed to touch last_seen while publishing presence"
        );
    }

    let payload = PresencePayload {
        installation_id: device_id.to_string(),
        principal_kind: presence_principal_kind(role),
        online,
        last_seen_at_ms: at_ms,
        at_ms,
        account_id: principal.account_id().map(str::to_string),
    };
    let Ok(value) = serde_json::to_value(&payload) else {
        return;
    };

    match fanout_presence_targets(state, device_id, principal).await {
        Ok(topics) => {
            for topic in topics {
                state.realtime.fanout_stream_event(
                    &topic,
                    PRESENCE_STREAM_KIND,
                    None,
                    value.clone(),
                );
            }
            tracing::debug!(
                target: "minos_backend::realtime::presence",
                device_id = %device_id,
                online,
                "published presence stream events"
            );
        }
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::realtime::presence",
                error = %error,
                device_id = %device_id,
                online,
                "failed to resolve presence fanout targets"
            );
        }
    }
}

fn presence_principal_kind(role: DeviceRole) -> PresencePrincipalKind {
    if role == DeviceRole::AgentHost {
        PresencePrincipalKind::Host
    } else {
        PresencePrincipalKind::AccountClient
    }
}

async fn fanout_presence_targets(
    state: &BackendState,
    device_id: DeviceId,
    principal: &ConnectionPrincipal,
) -> Result<Vec<RealtimeTopic>, crate::error::BackendError> {
    match principal {
        ConnectionPrincipal::Host { .. } => {
            // Host online/offline → every linked account's default topic.
            let accounts = host_links::list_accounts_for_host(&state.store, device_id).await?;
            Ok(accounts
                .into_iter()
                .map(|pair| RealtimeTopic::Account(pair.mobile_account_id))
                .collect())
        }
        ConnectionPrincipal::Account { account_id } => {
            // Account-client online/offline → every linked host topic.
            let hosts = host_links::list_hosts_for_account(&state.store, account_id).await?;
            Ok(hosts
                .into_iter()
                .map(|pair| RealtimeTopic::Host(pair.host_device_id.to_string()))
                .collect())
        }
    }
}

/// Build a presence stream payload value (tests / call sites that already
/// have timestamps).
#[must_use]
pub fn presence_payload_value(payload: &PresencePayload) -> Value {
    serde_json::to_value(payload).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_protocol::realtime::PRESENCE_STREAM_KIND;

    #[test]
    fn presence_payload_round_trips() {
        let payload = PresencePayload {
            installation_id: "11111111-1111-1111-1111-111111111111".into(),
            principal_kind: PresencePrincipalKind::Host,
            online: true,
            last_seen_at_ms: 100,
            at_ms: 100,
            account_id: None,
        };
        let value = presence_payload_value(&payload);
        let back: PresencePayload = serde_json::from_value(value).unwrap();
        assert_eq!(back, payload);
        assert_eq!(PRESENCE_STREAM_KIND, "presence");
    }
}
