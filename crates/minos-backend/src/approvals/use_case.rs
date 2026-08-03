use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use minos_domain::DeviceId;
use minos_protocol::{ApprovalDecisionRequest, Envelope, EventKind};
use serde_json::{json, Value};
use tokio::sync::Notify;

use crate::app::repositories::RepositorySet;
use crate::error::BackendError;
use crate::host_commands::HostCommandService;
use crate::session::SessionRegistry;
use crate::store::{self, approval_requests::ApprovalRequestState, StoreHandle};

const APPROVAL_COMMAND_METHOD: &str = "minos_approval_decision";
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(5);
const EXPIRED_BATCH_SIZE: u32 = 32;
const POLLER_RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct RecordApprovalRequestInput {
    pub request_id: String,
    pub agent_session_id: String,
    pub turn_id: Option<String>,
    pub method: String,
    pub params_json: Value,
    pub created_at_ms: i64,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct RespondApprovalInput {
    pub request_id: String,
    pub decision: Value,
    pub client_request_id: Option<String>,
    pub caller_account_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    #[error("approval_not_found")]
    NotFound,
    #[error("approval_already_resolved")]
    AlreadyResolved,
    #[error("conversation_forbidden")]
    Forbidden,
    #[error("validation_format: {0}")]
    ValidationFormat(String),
    #[error(transparent)]
    Internal(#[from] BackendError),
}

#[async_trait]
pub trait ApprovalService: Send + Sync {
    async fn record_request(&self, input: RecordApprovalRequestInput) -> Result<(), BackendError>;
    async fn handle_host_timeout(
        &self,
        request_id: &str,
        reason: &str,
        resolved_at_ms: i64,
    ) -> Result<(), BackendError>;
    async fn resolve_disconnected_for_account(&self, account_id: &str) -> Result<(), BackendError>;
    async fn respond(&self, input: RespondApprovalInput) -> Result<(), ApprovalError>;
}

pub struct DefaultApprovalService {
    repos: Arc<RepositorySet>,
    store: StoreHandle,
    registry: Arc<SessionRegistry>,
    host_commands: Arc<dyn HostCommandService>,
    notify: Notify,
}

impl DefaultApprovalService {
    #[must_use]
    pub fn new(
        repos: Arc<RepositorySet>,
        store: StoreHandle,
        registry: Arc<SessionRegistry>,
        host_commands: Arc<dyn HostCommandService>,
        enable_timeout_worker: bool,
    ) -> Arc<Self> {
        let service = Arc::new(Self {
            repos,
            store,
            registry,
            host_commands,
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

    async fn poll_expired(&self) -> Result<(), BackendError> {
        let rows = store::approval_requests::list_expired_pending(
            &self.store,
            chrono::Utc::now().timestamp_millis(),
            EXPIRED_BATCH_SIZE,
        )
        .await?;

        for row in rows {
            self.resolve_automatically(row, ApprovalRequestState::Timeout)
                .await;
        }

        Ok(())
    }

    async fn resolve_automatically(
        &self,
        row: store::approval_requests::ApprovalRequestRow,
        state: ApprovalRequestState,
    ) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let auto_decision = auto_reject_decision(&row.method);
        let resolution_json = match auto_decision.as_ref() {
            Some(decision) => json!({
                "reason": state.as_str(),
                "auto_decision": decision,
            }),
            None => json!({ "reason": state.as_str() }),
        };

        match self
            .repos
            .approval_requests
            .resolve(&row.request_id, state, now_ms, Some(&resolution_json))
            .await
        {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::approvals",
                    error = %error,
                    request_id = %row.request_id,
                    state = state.as_str(),
                    "failed to mark approval resolved"
                );
                return;
            }
        }

        if let Some(decision) = auto_decision {
            let request = ApprovalDecisionRequest {
                request_id: row.request_id.clone(),
                session_id: row.agent_session_id.clone(),
                decision,
            };
            if let Err(error) = self
                .dispatch_host_decision(
                    row.host_device_id,
                    Some(&row.agent_session_id),
                    None,
                    &request,
                )
                .await
            {
                tracing::warn!(
                    target: "minos_backend::approvals",
                    error = %error,
                    request_id = %row.request_id,
                    state = state.as_str(),
                    "failed to forward automatic approval decision to host"
                );
            }
        }

        self.broadcast_timeout(
            row.host_device_id,
            &row.agent_session_id,
            &row.request_id,
            state.as_str(),
        )
        .await;
    }

    async fn broadcast_timeout(
        &self,
        host_device_id: DeviceId,
        session_id: &str,
        request_id: &str,
        reason: &str,
    ) {
        let frame = Envelope::Event {
            version: 1,
            event: EventKind::ApprovalTimeout {
                session_id: session_id.to_string(),
                request_id: request_id.to_string(),
                reason: reason.to_string(),
            },
        };

        match store::host_links::list_accounts_for_host(&self.store, host_device_id).await {
            Ok(accounts) => {
                for account in accounts {
                    let _ = self
                        .registry
                        .broadcast_mobile_account(&account.mobile_account_id, frame.clone());
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::approvals",
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
        agent_session_id: Option<&str>,
        requested_by_account_id: Option<&str>,
        request: &ApprovalDecisionRequest,
    ) -> Result<(), BackendError> {
        let params = serde_json::to_value(request).map_err(|error| BackendError::StoreQuery {
            operation: "approvals.dispatch_host_decision.serialize".into(),
            message: error.to_string(),
        })?;
        self.host_commands
            .dispatch_json(
                &format!("cmd-approval-{}", request.request_id),
                host_device_id,
                agent_session_id,
                APPROVAL_COMMAND_METHOD,
                &params,
                requested_by_account_id,
                APPROVAL_TIMEOUT,
            )
            .await?;
        Ok(())
    }
}

#[async_trait]
impl ApprovalService for DefaultApprovalService {
    async fn record_request(&self, input: RecordApprovalRequestInput) -> Result<(), BackendError> {
        // timeout_ms == 0 means "no host/backend auto-timeout" (wait for user).
        let deadline_at_ms = if input.timeout_ms == 0 {
            i64::MAX
        } else {
            input
                .created_at_ms
                .saturating_add(i64::try_from(input.timeout_ms).unwrap_or(i64::MAX))
        };
        store::approval_requests::insert_pending(
            &self.store,
            &input.request_id,
            &input.agent_session_id,
            input.turn_id.as_deref(),
            &input.method,
            &input.params_json,
            input.created_at_ms,
            deadline_at_ms,
        )
        .await?;
        self.notify.notify_one();
        Ok(())
    }

    async fn handle_host_timeout(
        &self,
        request_id: &str,
        reason: &str,
        resolved_at_ms: i64,
    ) -> Result<(), BackendError> {
        let state = if reason == ApprovalRequestState::Disconnected.as_str() {
            ApprovalRequestState::Disconnected
        } else {
            ApprovalRequestState::Timeout
        };
        let resolution_json = json!({ "reason": reason });
        let _ = self
            .repos
            .approval_requests
            .resolve(request_id, state, resolved_at_ms, Some(&resolution_json))
            .await?;
        Ok(())
    }

    async fn resolve_disconnected_for_account(&self, account_id: &str) -> Result<(), BackendError> {
        if self.registry.mobile_account_session_count(account_id) > 0 {
            return Ok(());
        }

        let mut hosts = store::host_links::list_hosts_for_account(&self.store, account_id)
            .await?
            .into_iter()
            .map(|row| row.host_device_id)
            .collect::<Vec<_>>();
        hosts.sort_unstable_by_key(|host| host.to_string());
        hosts.dedup();

        let mut fully_disconnected_hosts = Vec::with_capacity(hosts.len());
        for host_device_id in hosts {
            let still_online =
                store::host_links::list_accounts_for_host(&self.store, host_device_id)
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

        let rows = store::approval_requests::list_pending_for_hosts(
            &self.store,
            &fully_disconnected_hosts,
        )
        .await?;
        for row in rows {
            self.resolve_automatically(row, ApprovalRequestState::Disconnected)
                .await;
        }
        Ok(())
    }

    async fn respond(&self, input: RespondApprovalInput) -> Result<(), ApprovalError> {
        let client_request_id = input
            .client_request_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // C5.3: same client_request_id retry → return prior success (no re-dispatch).
        if let Some(ref cid) = client_request_id {
            if let Some(existing) =
                store::approval_requests::get_by_client_request_id(&self.store, cid).await?
            {
                if existing.request_id == input.request_id {
                    return Ok(());
                }
                return Err(ApprovalError::ValidationFormat(
                    "client_request_id already used for a different approval".into(),
                ));
            }
        }

        let row = self
            .repos
            .approval_requests
            .get(&input.request_id)
            .await?
            .ok_or(ApprovalError::NotFound)?;

        if row.state != ApprovalRequestState::Pending {
            // Already resolved without this client_request_id → conflict.
            return Err(ApprovalError::AlreadyResolved);
        }
        if !self
            .repos
            .account_host_pairings
            .exists(row.host_device_id, &input.caller_account_id)
            .await?
        {
            return Err(ApprovalError::Forbidden);
        }

        let request = ApprovalDecisionRequest {
            request_id: row.request_id.clone(),
            session_id: row.agent_session_id.clone(),
            decision: input.decision,
        };

        if let Err(error) = self
            .dispatch_host_decision(
                row.host_device_id,
                Some(&row.agent_session_id),
                Some(&input.caller_account_id),
                &request,
            )
            .await
        {
            if let BackendError::ForwardRpc { message, .. } = &error {
                if message.contains("invalid decision") {
                    return Err(ApprovalError::ValidationFormat(message.clone()));
                }
            }
            return Err(ApprovalError::Internal(error));
        }

        let resolved = store::approval_requests::resolve_with_client_request_id(
            &self.store,
            &row.request_id,
            ApprovalRequestState::Decided,
            chrono::Utc::now().timestamp_millis(),
            Some(&request.decision),
            client_request_id.as_deref(),
        )
        .await?;
        if !resolved {
            // Lost race: if our client_request_id was stamped by concurrent twin, ok.
            if let Some(ref cid) = client_request_id {
                if let Some(existing) =
                    store::approval_requests::get_by_client_request_id(&self.store, cid).await?
                {
                    if existing.request_id == input.request_id {
                        return Ok(());
                    }
                }
            }
            return Err(ApprovalError::AlreadyResolved);
        }

        Ok(())
    }
}

async fn timeout_poller(service: Weak<DefaultApprovalService>) {
    loop {
        let Some(service) = service.upgrade() else {
            break;
        };
        let next_deadline =
            match store::approval_requests::next_pending_deadline_at_ms(&service.store).await {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::approvals",
                        error = %error,
                        "approval timeout poller failed to query next deadline"
                    );
                    tokio::time::sleep(POLLER_RETRY_DELAY).await;
                    continue;
                }
            };

        let Some(deadline_at_ms) = next_deadline else {
            service.notify.notified().await;
            continue;
        };

        let now_ms = chrono::Utc::now().timestamp_millis();
        if deadline_at_ms <= now_ms {
            if let Err(error) = service.poll_expired().await {
                tracing::warn!(
                    target: "minos_backend::approvals",
                    error = %error,
                    "approval timeout poller iteration failed"
                );
            }
            continue;
        }

        let sleep_for = Duration::from_millis(
            u64::try_from(deadline_at_ms.saturating_sub(now_ms)).unwrap_or(u64::MAX),
        );
        tokio::select! {
            _ = tokio::time::sleep(sleep_for) => {
                if let Err(error) = service.poll_expired().await {
                    tracing::warn!(
                        target: "minos_backend::approvals",
                        error = %error,
                        "approval timeout poller iteration failed"
                    );
                }
            }
            _ = service.notify.notified() => {}
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
