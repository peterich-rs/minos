use std::sync::{Arc, Weak};
use std::time::Duration;

use minos_domain::DeviceId;
use minos_protocol::{ApprovalDecisionRequest, Envelope, EventKind};
use serde_json::{json, Value};
use tokio::sync::Notify;

use crate::error::BackendError;
use crate::host_command_runtime::HostCommandRuntime;
use crate::session::SessionRegistry;
use crate::store::{account_host_pairings, pending_approvals, StoreHandle};

const FORWARD_TIMEOUT: Duration = Duration::from_secs(5);
const EXPIRED_BATCH_SIZE: u32 = 32;
const POLLER_RETRY_DELAY: Duration = Duration::from_secs(1);
const APPROVAL_COMMAND_METHOD: &str = "minos_approval_decision";

#[derive(Debug, Clone)]
pub(crate) struct ApprovalDecisionInput {
    request_id: String,
    decision: Value,
    client_request_id: Option<String>,
}

impl ApprovalDecisionInput {
    pub(crate) fn new(
        request_id: String,
        decision: Value,
        client_request_id: Option<String>,
    ) -> Self {
        Self {
            request_id,
            decision,
            client_request_id,
        }
    }

    fn into_host_request(self, thread_id: String) -> ApprovalDecisionRequest {
        ApprovalDecisionRequest {
            request_id: self.request_id,
            thread_id,
            decision: self.decision,
        }
    }
}

#[derive(Debug)]
pub struct ApprovalRelay {
    store: StoreHandle,
    registry: Arc<SessionRegistry>,
    host_command_runtime: Arc<HostCommandRuntime>,
    notify: Notify,
}

impl ApprovalRelay {
    pub fn new(
        store: impl Into<StoreHandle>,
        registry: Arc<SessionRegistry>,
        host_command_runtime: Arc<HostCommandRuntime>,
    ) -> Arc<Self> {
        Self::new_with_timeout_worker(store, registry, host_command_runtime, true)
    }

    pub fn new_with_timeout_worker(
        store: impl Into<StoreHandle>,
        registry: Arc<SessionRegistry>,
        host_command_runtime: Arc<HostCommandRuntime>,
        enable_timeout_worker: bool,
    ) -> Arc<Self> {
        let store = store.into();
        let relay = Arc::new(Self {
            store,
            registry,
            host_command_runtime,
            notify: Notify::new(),
        });
        if enable_timeout_worker {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let weak = Arc::downgrade(&relay);
                runtime.spawn(async move {
                    timeout_poller(weak).await;
                });
            }
        }
        relay
    }

    #[allow(clippy::too_many_arguments)]
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
        .await?;
        self.notify.notify_one();
        Ok(())
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

    pub(crate) async fn submit_decision(
        &self,
        account_id: &str,
        req: ApprovalDecisionInput,
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

        let _client_request_id = req.client_request_id.as_deref();
        let request = req.into_host_request(row.thread_id.clone());
        self.dispatch_host_decision(row.host_device_id, Some(account_id), &request)
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

        let mut hosts = account_host_pairings::list_hosts_for_account(&self.store, account_id)
            .await?
            .into_iter()
            .map(|row| row.host_device_id)
            .collect::<Vec<_>>();
        hosts.sort_unstable_by_key(|host| host.to_string());
        hosts.dedup();

        let mut fully_disconnected_hosts = Vec::with_capacity(hosts.len());
        for host_device_id in hosts {
            let still_online =
                account_host_pairings::list_accounts_for_host(&self.store, host_device_id)
                    .await?
                    .into_iter()
                    .any(|row| {
                        self.registry
                            .mobile_account_session_count(&row.mobile_account_id)
                            > 0
                    });
            if !still_online {
                fully_disconnected_hosts.push(host_device_id);
            }
        }

        if fully_disconnected_hosts.is_empty() {
            return Ok(());
        }

        let rows =
            pending_approvals::list_unresolved_for_hosts(&self.store, &fully_disconnected_hosts)
                .await?;
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
            if let Err(error) = self
                .dispatch_host_decision(row.host_device_id, None, &request)
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

    async fn dispatch_host_decision(
        &self,
        host_device_id: DeviceId,
        requested_by_account_id: Option<&str>,
        request: &ApprovalDecisionRequest,
    ) -> Result<(), BackendError> {
        let created_at_ms = chrono::Utc::now().timestamp_millis();
        let command_id = approval_command_id(&request.request_id);
        let command_params =
            serde_json::to_value(request).map_err(|error| BackendError::StoreQuery {
                operation: "approval_relay::dispatch_host_decision.serialize".into(),
                message: error.to_string(),
            })?;
        let deadline_at_ms = created_at_ms
            .saturating_add(i64::try_from(FORWARD_TIMEOUT.as_millis()).unwrap_or(i64::MAX));
        self.host_command_runtime
            .enqueue_if_missing(
                &command_id,
                host_device_id,
                None,
                APPROVAL_COMMAND_METHOD,
                &command_params,
                requested_by_account_id,
                deadline_at_ms,
                created_at_ms,
            )
            .await?;

        self.host_command_runtime
            .dispatch::<_, ()>(
                &command_id,
                host_device_id,
                APPROVAL_COMMAND_METHOD,
                request,
                FORWARD_TIMEOUT,
            )
            .await?;

        Ok(())
    }
}

async fn timeout_poller(relay: Weak<ApprovalRelay>) {
    loop {
        let Some(relay) = relay.upgrade() else {
            break;
        };
        let next_timeout_at_ms =
            match pending_approvals::next_unresolved_timeout_at_ms(&relay.store).await {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::approval_relay",
                        error = %error,
                        "approval timeout poller failed to query next deadline"
                    );
                    tokio::time::sleep(POLLER_RETRY_DELAY).await;
                    continue;
                }
            };

        let Some(timeout_at_ms) = next_timeout_at_ms else {
            relay.notify.notified().await;
            continue;
        };

        let now_ms = chrono::Utc::now().timestamp_millis();
        if timeout_at_ms <= now_ms {
            if let Err(error) = relay.poll_expired().await {
                tracing::warn!(
                    target: "minos_backend::approval_relay",
                    error = %error,
                    "approval timeout poller iteration failed"
                );
            }
            continue;
        }

        let sleep_for = Duration::from_millis(
            u64::try_from(timeout_at_ms.saturating_sub(now_ms)).unwrap_or(u64::MAX),
        );
        tokio::select! {
            _ = tokio::time::sleep(sleep_for) => {
                if let Err(error) = relay.poll_expired().await {
                    tracing::warn!(
                        target: "minos_backend::approval_relay",
                        error = %error,
                        "approval timeout poller iteration failed"
                    );
                }
            }
            _ = relay.notify.notified() => {}
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

fn approval_command_id(request_id: &str) -> String {
    format!("cmd-approval-{request_id}")
}
