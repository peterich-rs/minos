use std::sync::{Arc, Weak};
use std::time::Duration;

use minos_domain::DeviceId;
use minos_protocol::{ApprovalDecisionRequest, Envelope, EventKind};
use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::error::BackendError;
use crate::forward_rpc;
use crate::session::SessionRegistry;
use crate::store::{account_host_pairings, pending_approvals};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const FORWARD_TIMEOUT: Duration = Duration::from_secs(5);
const EXPIRED_BATCH_SIZE: u32 = 32;

#[derive(Debug)]
pub struct ApprovalRelay {
    store: SqlitePool,
    registry: Arc<SessionRegistry>,
}

impl ApprovalRelay {
    pub fn new(store: SqlitePool, registry: Arc<SessionRegistry>) -> Arc<Self> {
        let relay = Arc::new(Self { store, registry });
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let weak = Arc::downgrade(&relay);
            runtime.spawn(async move {
                timeout_poller(weak).await;
            });
        }
        relay
    }

    pub async fn record_request(
        &self,
        host_device_id: DeviceId,
        request_id: &str,
        thread_id: &str,
        turn_id: &str,
        method: &str,
        params_json: &Value,
        created_at_ms: i64,
        timeout_ms: u64,
    ) -> Result<(), BackendError> {
        let timeout_at_ms =
            created_at_ms.saturating_add(i64::try_from(timeout_ms).unwrap_or(i64::MAX));
        pending_approvals::insert(
            &self.store,
            request_id,
            thread_id,
            turn_id,
            host_device_id,
            method,
            params_json,
            created_at_ms,
            timeout_at_ms,
        )
        .await
    }

    pub async fn handle_host_timeout(
        &self,
        _thread_id: &str,
        request_id: &str,
        reason: &str,
        resolved_at_ms: i64,
    ) -> Result<(), BackendError> {
        let _ = pending_approvals::resolve(&self.store, request_id, reason, resolved_at_ms).await?;
        Ok(())
    }

    pub async fn submit_decision(
        &self,
        account_id: &str,
        req: ApprovalDecisionRequest,
    ) -> Result<bool, BackendError> {
        let Some(row) = pending_approvals::get(&self.store, &req.request_id).await? else {
            return Ok(false);
        };
        if row.resolved_at_ms.is_some() {
            return Ok(false);
        }
        if !account_host_pairings::exists(&self.store, row.host_device_id, account_id).await? {
            return Ok(false);
        }

        forward_rpc::call_host::<_, ()>(
            &self.registry,
            row.host_device_id,
            "minos_approval_decision",
            &req,
            FORWARD_TIMEOUT,
        )
        .await?;

        pending_approvals::resolve(
            &self.store,
            &row.request_id,
            "user_decision",
            chrono::Utc::now().timestamp_millis(),
        )
        .await?;
        Ok(true)
    }

    pub async fn resolve_disconnected_for_account(
        &self,
        account_id: &str,
    ) -> Result<(), BackendError> {
        if self.registry.mobile_account_session_count(account_id) > 0 {
            return Ok(());
        }

        let hosts = account_host_pairings::list_hosts_for_account(&self.store, account_id)
            .await?
            .into_iter()
            .map(|row| row.host_device_id)
            .collect::<Vec<_>>();
        let rows = pending_approvals::list_unresolved_for_hosts(&self.store, &hosts).await?;
        for row in rows {
            self.resolve_automatically(row, "disconnected").await;
        }
        Ok(())
    }

    async fn poll_expired(&self) -> Result<(), BackendError> {
        let rows = pending_approvals::list_expired_unresolved(
            &self.store,
            chrono::Utc::now().timestamp_millis(),
            EXPIRED_BATCH_SIZE,
        )
        .await?;

        for row in rows {
            self.resolve_automatically(row, "timeout").await;
        }

        Ok(())
    }

    async fn resolve_automatically(
        &self,
        row: pending_approvals::PendingApprovalRow,
        resolution: &'static str,
    ) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        match pending_approvals::resolve(&self.store, &row.request_id, resolution, now_ms).await {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::approval_relay",
                    error = %error,
                    request_id = %row.request_id,
                    resolution,
                    "failed to mark pending approval resolved"
                );
                return;
            }
        }

        if let Some(decision) = auto_reject_decision(&row.method) {
            let request = ApprovalDecisionRequest {
                request_id: row.request_id.clone(),
                thread_id: row.thread_id.clone(),
                decision,
            };
            if let Err(error) = forward_rpc::call_host::<_, ()>(
                &self.registry,
                row.host_device_id,
                "minos_approval_decision",
                &request,
                FORWARD_TIMEOUT,
            )
            .await
            {
                tracing::warn!(
                    target: "minos_backend::approval_relay",
                    error = %error,
                    request_id = %row.request_id,
                    resolution,
                    "failed to forward automatic approval decision to host"
                );
            }
        }

        self.broadcast_timeout(
            row.host_device_id,
            &row.thread_id,
            &row.request_id,
            resolution,
        )
        .await;
    }

    async fn broadcast_timeout(
        &self,
        host_device_id: DeviceId,
        thread_id: &str,
        request_id: &str,
        reason: &str,
    ) {
        let frame = Envelope::Event {
            version: 1,
            event: EventKind::ApprovalTimeout {
                thread_id: thread_id.to_string(),
                request_id: request_id.to_string(),
                reason: reason.to_string(),
            },
        };

        match account_host_pairings::list_accounts_for_host(&self.store, host_device_id).await {
            Ok(accounts) => {
                for account in accounts {
                    let _ = self
                        .registry
                        .broadcast_mobile_account(&account.mobile_account_id, frame.clone());
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::approval_relay",
                    error = %error,
                    host_device_id = %host_device_id,
                    request_id,
                    "failed to broadcast approval timeout"
                );
            }
        }
    }
}

async fn timeout_poller(relay: Weak<ApprovalRelay>) {
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    interval.tick().await;

    loop {
        interval.tick().await;
        let Some(relay) = relay.upgrade() else {
            break;
        };
        if let Err(error) = relay.poll_expired().await {
            tracing::warn!(
                target: "minos_backend::approval_relay",
                error = %error,
                "approval timeout poller iteration failed"
            );
        }
    }
}

fn auto_reject_decision(method: &str) -> Option<Value> {
    match method {
        "applyPatchApproval" | "execCommandApproval" => Some(json!({ "decision": "denied" })),
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            Some(json!({ "decision": "decline" }))
        }
        "item/permissions/requestApproval" => Some(json!({
            "permissions": {},
            "scope": "turn"
        })),
        _ => None,
    }
}
