use std::sync::Arc;

use async_trait::async_trait;
use minos_domain::DeviceId;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::agent_sessions::dto::{
    AgentSessionSummary, ListAgentSessionsInput, ListAgentSessionsOutput, ReadTurnEvent,
    ReadTurnMetadata, ReadTurnsInput, ReadTurnsOutput, SendInputInput, SendInputOutput,
    StartAgentSessionInput, StartAgentSessionOutput, StopAgentSessionInput,
};
use crate::app::repositories::RepositorySet;
use crate::app::tx::{DbTx, Storage};
use crate::error::BackendError;
use crate::host_commands::{HostCommandService, NewHostCommand};
use crate::realtime::DurableEvent;
use crate::store::{durable_event_log, outbox_events, StoreHandle};

const START_COMMAND_METHOD: &str = "agent_session.start";
const SEND_INPUT_COMMAND_METHOD: &str = "agent_session.send_input";
const STOP_COMMAND_METHOD: &str = "agent_session.stop";
const DEFAULT_HOST_COMMAND_DEADLINE_MS: i64 = 5_000;

#[derive(Debug, thiserror::Error)]
pub enum AgentSessionError {
    #[error("agent_session_not_found")]
    NotFound,
    #[error("agent_turn_not_found")]
    TurnNotFound,
    #[error("conversation_forbidden")]
    ConversationForbidden,
    #[error("agent_session_host_unavailable")]
    HostUnavailable,
    #[error("agent_session_state_invalid")]
    StateInvalid,
    #[error("validation_missing_field: {0}")]
    ValidationMissing(&'static str),
    #[error("validation_format: {0}")]
    ValidationFormat(&'static str),
    #[error(transparent)]
    Internal(#[from] BackendError),
}

#[async_trait]
pub trait AgentSessionService: Send + Sync {
    async fn start(
        &self,
        input: StartAgentSessionInput,
    ) -> Result<StartAgentSessionOutput, AgentSessionError>;

    async fn send_input(&self, input: SendInputInput)
        -> Result<SendInputOutput, AgentSessionError>;

    async fn stop(&self, input: StopAgentSessionInput) -> Result<(), AgentSessionError>;

    async fn list(
        &self,
        input: ListAgentSessionsInput,
    ) -> Result<ListAgentSessionsOutput, AgentSessionError>;

    async fn read_turns(&self, input: ReadTurnsInput)
        -> Result<ReadTurnsOutput, AgentSessionError>;
}

pub struct DefaultAgentSessionService {
    repos: Arc<RepositorySet>,
    store: StoreHandle,
    host_commands: Arc<dyn HostCommandService>,
}

impl DefaultAgentSessionService {
    #[must_use]
    pub fn new(
        repos: Arc<RepositorySet>,
        store: StoreHandle,
        host_commands: Arc<dyn HostCommandService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            repos,
            store,
            host_commands,
        })
    }

    async fn select_host_for_account(
        &self,
        account_id: &str,
    ) -> Result<DeviceId, AgentSessionError> {
        crate::store::account_host_pairings::list_hosts_for_account(&self.store, account_id)
            .await?
            .into_iter()
            .map(|row| row.host_device_id)
            .next()
            .ok_or(AgentSessionError::HostUnavailable)
    }

    async fn validate_agent_id(&self, agent_id: &str) -> Result<(), AgentSessionError> {
        if crate::store::social::get_agent(&self.store, agent_id)
            .await?
            .is_some()
        {
            return Ok(());
        }
        match agent_id {
            "agent_codex" | "agent_claude" | "agent_gemini" => Ok(()),
            _ => Err(AgentSessionError::ValidationFormat("unknown agent_id")),
        }
    }

    fn parse_host_device_id(host_device_id: &str) -> Result<DeviceId, AgentSessionError> {
        Uuid::parse_str(host_device_id)
            .map(DeviceId)
            .map_err(|_| AgentSessionError::ValidationFormat("invalid host installation id"))
    }
}

#[async_trait]
impl AgentSessionService for DefaultAgentSessionService {
    async fn start(
        &self,
        input: StartAgentSessionInput,
    ) -> Result<StartAgentSessionOutput, AgentSessionError> {
        if input.conversation_id.trim().is_empty() {
            return Err(AgentSessionError::ValidationMissing("conversation_id"));
        }
        if input.agent_id.trim().is_empty() {
            return Err(AgentSessionError::ValidationMissing("agent_id"));
        }
        if input.client_request_id.trim().is_empty() {
            return Err(AgentSessionError::ValidationMissing("client_request_id"));
        }
        if !crate::store::social::is_conversation_member(
            &self.store,
            &input.conversation_id,
            &input.caller_account_id,
        )
        .await?
        {
            return Err(AgentSessionError::ConversationForbidden);
        }
        self.validate_agent_id(&input.agent_id).await?;

        let session_id = deterministic_uuid(
            "agent-session-start",
            &[&input.caller_account_id, &input.client_request_id],
        );
        let host_command_id = format!("cmd-agent-session-start-{session_id}");
        if let Some(existing) = crate::store::agent_sessions::get(&self.store, &session_id).await? {
            let initial_turn_id = input
                .initial_user_message
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .map(|_| deterministic_uuid("agent-session-initial-turn", &[&session_id]));
            return Ok(StartAgentSessionOutput {
                session_id: existing.session_id,
                conversation_id: existing.conversation_id,
                host_installation_id: existing
                    .host_device_id
                    .ok_or(AgentSessionError::HostUnavailable)?,
                started_at_ms: existing.started_at_ms,
                initial_turn_id,
                host_command_id,
            });
        }

        let host_device_id = match input.host_installation_id.as_deref() {
            Some(host_id) => {
                let host_device_id = Self::parse_host_device_id(host_id)?;
                if !self
                    .repos
                    .account_host_pairings
                    .exists(host_device_id, &input.caller_account_id)
                    .await?
                {
                    return Err(AgentSessionError::HostUnavailable);
                }
                host_device_id
            }
            None => {
                self.select_host_for_account(&input.caller_account_id)
                    .await?
            }
        };

        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let initial_turn_id = input
            .initial_user_message
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .map(|_| deterministic_uuid("agent-session-initial-turn", &[&session_id]));

        let mut tx = self.store.begin().await?;
        insert_agent_session_in_tx(
            &mut tx,
            &session_id,
            &input.conversation_id,
            input.project_id.as_deref(),
            Some(&host_device_id.to_string()),
            Some(&input.agent_id),
            "pending",
            started_at_ms,
            None,
            Some(&input.caller_account_id),
            Some(&input.client_request_id),
        )
        .await?;

        if let Some(turn_id) = initial_turn_id.as_deref() {
            insert_agent_turn_in_tx(
                &mut tx,
                turn_id,
                &session_id,
                1,
                "user",
                "completed",
                started_at_ms,
                Some(started_at_ms),
                input.initial_user_message.as_deref(),
                None,
            )
            .await?;
        }

        let started_event = DurableEvent::AgentSessionStarted {
            session_id: session_id.clone(),
            conversation_id: input.conversation_id.clone(),
            project_id: input.project_id.clone(),
            host_installation_id: host_device_id.to_string(),
            agent_id: input.agent_id.clone(),
            at_ms: started_at_ms,
        };
        let cursor = durable_event_log::record_in_tx(
            &mut tx,
            &Uuid::new_v4().to_string(),
            &started_event,
            started_at_ms,
        )
        .await?;
        outbox_events::enqueue_in_tx(
            &mut tx,
            &Uuid::new_v4().to_string(),
            cursor.topic.kind().as_str(),
            &cursor.event_id,
            started_at_ms,
        )
        .await?;

        self.host_commands
            .enqueue_in_tx(
                &mut tx,
                NewHostCommand {
                    command_id: host_command_id.clone(),
                    host_installation_id: host_device_id,
                    agent_session_id: Some(session_id.clone()),
                    method: START_COMMAND_METHOD.into(),
                    params_json: serde_json::json!({
                        "session_id": session_id.clone(),
                        "agent_id": input.agent_id.clone(),
                        "project_id": input.project_id.clone(),
                        "conversation_id": input.conversation_id.clone(),
                        "initial_user_message": input.initial_user_message.clone(),
                    }),
                    requested_by_account_id: Some(input.caller_account_id.clone()),
                    deadline_at_ms: started_at_ms.saturating_add(DEFAULT_HOST_COMMAND_DEADLINE_MS),
                    created_at_ms: started_at_ms,
                },
            )
            .await?;
        tx.commit().await?;

        Ok(StartAgentSessionOutput {
            session_id,
            conversation_id: input.conversation_id,
            host_installation_id: host_device_id.to_string(),
            started_at_ms,
            initial_turn_id,
            host_command_id,
        })
    }

    async fn send_input(
        &self,
        input: SendInputInput,
    ) -> Result<SendInputOutput, AgentSessionError> {
        if input.text.trim().is_empty() {
            return Err(AgentSessionError::ValidationMissing("text"));
        }
        if input.client_request_id.trim().is_empty() {
            return Err(AgentSessionError::ValidationMissing("client_request_id"));
        }

        let turn_id = deterministic_uuid(
            "agent-session-send-input",
            &[&input.session_id, &input.client_request_id],
        );
        if let Some(existing) = crate::store::agent_turns::get(&self.store, &turn_id).await? {
            return Ok(SendInputOutput {
                session_id: existing.agent_session_id,
                turn_id: existing.turn_id,
                turn_seq: existing.turn_seq,
            });
        }

        let session = self
            .repos
            .agent_sessions
            .get_for_account(&input.session_id, &input.caller_account_id)
            .await?
            .ok_or(AgentSessionError::NotFound)?;
        if matches!(
            session.status.as_str(),
            "stopping" | "stopped" | "ended" | "failed"
        ) {
            return Err(AgentSessionError::StateInvalid);
        }

        let host_device_id = session
            .host_device_id
            .as_deref()
            .ok_or(AgentSessionError::HostUnavailable)
            .and_then(Self::parse_host_device_id)?;

        let existing_turns = self
            .repos
            .agent_turns
            .list_for_session(&session.session_id, None, u32::MAX)
            .await?;
        let turn_seq = existing_turns.last().map_or(1, |turn| turn.turn_seq + 1);
        let now_ms = chrono::Utc::now().timestamp_millis();
        let host_command_id = format!("cmd-agent-session-send-{turn_id}");

        let mut tx = self.store.begin().await?;
        insert_agent_turn_in_tx(
            &mut tx,
            &turn_id,
            &session.session_id,
            turn_seq,
            "user",
            "completed",
            now_ms,
            Some(now_ms),
            Some(&input.text),
            None,
        )
        .await?;

        let appended_event = DurableEvent::AgentTurnAppended {
            session_id: session.session_id.clone(),
            turn_id: turn_id.clone(),
            turn_seq,
            role: "user".into(),
            status: "completed".into(),
            at_ms: now_ms,
        };
        let cursor = durable_event_log::record_in_tx(
            &mut tx,
            &Uuid::new_v4().to_string(),
            &appended_event,
            now_ms,
        )
        .await?;
        outbox_events::enqueue_in_tx(
            &mut tx,
            &Uuid::new_v4().to_string(),
            cursor.topic.kind().as_str(),
            &cursor.event_id,
            now_ms,
        )
        .await?;

        self.host_commands
            .enqueue_in_tx(
                &mut tx,
                NewHostCommand {
                    command_id: host_command_id,
                    host_installation_id: host_device_id,
                    agent_session_id: Some(session.session_id.clone()),
                    method: SEND_INPUT_COMMAND_METHOD.into(),
                    params_json: serde_json::json!({
                        "session_id": session.session_id.clone(),
                        "turn_id": turn_id.clone(),
                        "text": input.text.clone(),
                        "mentions": input.mentions.clone(),
                    }),
                    requested_by_account_id: Some(input.caller_account_id),
                    deadline_at_ms: now_ms.saturating_add(DEFAULT_HOST_COMMAND_DEADLINE_MS),
                    created_at_ms: now_ms,
                },
            )
            .await?;
        tx.commit().await?;

        Ok(SendInputOutput {
            session_id: session.session_id,
            turn_id,
            turn_seq,
        })
    }

    async fn stop(&self, input: StopAgentSessionInput) -> Result<(), AgentSessionError> {
        let session = self
            .repos
            .agent_sessions
            .get_for_account(&input.session_id, &input.caller_account_id)
            .await?
            .ok_or(AgentSessionError::NotFound)?;
        let host_device_id = session
            .host_device_id
            .as_deref()
            .ok_or(AgentSessionError::HostUnavailable)
            .and_then(Self::parse_host_device_id)?;
        if matches!(
            session.status.as_str(),
            "stopping" | "stopped" | "ended" | "failed"
        ) {
            return Err(AgentSessionError::StateInvalid);
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut tx = self.store.begin().await?;
        update_agent_session_status_in_tx(&mut tx, &session.session_id, "stopping", None).await?;
        self.host_commands
            .enqueue_in_tx(
                &mut tx,
                NewHostCommand {
                    command_id: format!("cmd-agent-session-stop-{}", session.session_id),
                    host_installation_id: host_device_id,
                    agent_session_id: Some(session.session_id.clone()),
                    method: STOP_COMMAND_METHOD.into(),
                    params_json: serde_json::json!({
                        "session_id": session.session_id,
                    }),
                    requested_by_account_id: Some(input.caller_account_id),
                    deadline_at_ms: now_ms.saturating_add(DEFAULT_HOST_COMMAND_DEADLINE_MS),
                    created_at_ms: now_ms,
                },
            )
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn list(
        &self,
        input: ListAgentSessionsInput,
    ) -> Result<ListAgentSessionsOutput, AgentSessionError> {
        if input.limit > 200 {
            return Err(AgentSessionError::ValidationFormat("limit must be <= 200"));
        }

        let rows = match (
            input.conversation_id.as_deref(),
            input.project_id.as_deref(),
        ) {
            (Some(_), Some(_)) => {
                return Err(AgentSessionError::ValidationFormat(
                    "conversation_id and project_id cannot be combined",
                ));
            }
            (Some(conversation_id), None) => {
                self.repos
                    .agent_sessions
                    .list_for_account_conversation(
                        conversation_id,
                        &input.caller_account_id,
                        input.before_started_at_ms,
                        input.limit,
                    )
                    .await?
            }
            (None, Some(project_id)) => {
                self.repos
                    .agent_sessions
                    .list_for_account_project(
                        project_id,
                        &input.caller_account_id,
                        input.before_started_at_ms,
                        input.limit,
                    )
                    .await?
            }
            (None, None) => {
                self.repos
                    .agent_sessions
                    .list_for_account(
                        &input.caller_account_id,
                        input.before_started_at_ms,
                        input.limit,
                    )
                    .await?
            }
        };

        let next_before_started_at_ms =
            if rows.len() == usize::try_from(input.limit).unwrap_or(usize::MAX) {
                rows.last().map(|row| row.started_at_ms)
            } else {
                None
            };

        Ok(ListAgentSessionsOutput {
            sessions: rows
                .into_iter()
                .map(|row| AgentSessionSummary {
                    session_id: row.session_id,
                    conversation_id: row.conversation_id,
                    project_id: row.project_id,
                    agent_id: row.agent_id,
                    status: row.status,
                    started_at_ms: row.started_at_ms,
                    ended_at_ms: row.ended_at_ms,
                })
                .collect(),
            next_before_started_at_ms,
        })
    }

    async fn read_turns(
        &self,
        input: ReadTurnsInput,
    ) -> Result<ReadTurnsOutput, AgentSessionError> {
        if input.limit > 200 {
            return Err(AgentSessionError::ValidationFormat("limit must be <= 200"));
        }

        if let Some(turn_id) = input.turn_id.as_deref() {
            let turn = self
                .repos
                .agent_turns
                .get_for_account(turn_id, &input.caller_account_id)
                .await?
                .ok_or(AgentSessionError::TurnNotFound)?;
            let events = self
                .repos
                .agent_turn_events
                .list_for_turn(&turn.turn_id, input.after_event_seq, input.limit)
                .await?;

            let mut decoded_events = Vec::with_capacity(events.len());
            for event in events {
                let payload = serde_json::from_str(&event.payload_json).map_err(|_| {
                    AgentSessionError::Internal(BackendError::StoreDecode {
                        column: "agent_turn_events.payload_json".into(),
                        message: "invalid stored turn event payload".into(),
                    })
                })?;
                decoded_events.push(ReadTurnEvent {
                    turn_id: event.turn_id,
                    event_seq: event.event_seq,
                    kind: event.kind,
                    payload,
                    created_at_ms: event.created_at_ms,
                });
            }
            let next_event_seq = decoded_events.last().map(|event| event.event_seq);

            return Ok(ReadTurnsOutput {
                session_id: Some(turn.agent_session_id),
                turn_id: Some(turn.turn_id),
                turns: Vec::new(),
                events: decoded_events,
                next_turn_seq: None,
                next_event_seq,
            });
        }

        let session_id = input
            .session_id
            .as_deref()
            .ok_or(AgentSessionError::ValidationMissing("session_id"))?;
        let session = self
            .repos
            .agent_sessions
            .get_for_account(session_id, &input.caller_account_id)
            .await?
            .ok_or(AgentSessionError::NotFound)?;
        let turns = self
            .repos
            .agent_turns
            .list_for_session(&session.session_id, input.after_turn_seq, input.limit)
            .await?;
        let next_turn_seq = turns.last().map(|turn| turn.turn_seq);

        Ok(ReadTurnsOutput {
            session_id: Some(session.session_id),
            turn_id: None,
            turns: turns
                .into_iter()
                .map(|turn| ReadTurnMetadata {
                    turn_id: turn.turn_id,
                    turn_seq: turn.turn_seq,
                    role: turn.role,
                    status: turn.status,
                    started_at_ms: turn.started_at_ms,
                    finished_at_ms: turn.finished_at_ms,
                    summary_text: turn.summary_text,
                })
                .collect(),
            events: Vec::new(),
            next_turn_seq,
            next_event_seq: None,
        })
    }
}

fn deterministic_uuid(namespace: &str, parts: &[&str]) -> String {
    let mut value = namespace.to_string();
    for part in parts {
        value.push(':');
        value.push_str(part);
    }
    let digest = Sha256::digest(value.as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn insert_agent_session_in_tx(
    tx: &mut DbTx<'_>,
    session_id: &str,
    conversation_id: &str,
    project_id: Option<&str>,
    host_device_id: Option<&str>,
    agent_id: Option<&str>,
    status: &str,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
    idempotency_account_id: Option<&str>,
    idempotency_key: Option<&str>,
) -> Result<(), BackendError> {
    match tx {
        DbTx::Sqlite(tx) => {
            sqlx::query(
                "INSERT INTO agent_sessions
                    (session_id, conversation_id, project_id, host_device_id, agent_id, status, started_at_ms, ended_at_ms, idempotency_account_id, idempotency_key)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(session_id)
            .bind(conversation_id)
            .bind(project_id)
            .bind(host_device_id)
            .bind(agent_id)
            .bind(status)
            .bind(started_at_ms)
            .bind(ended_at_ms)
            .bind(idempotency_account_id)
            .bind(idempotency_key)
            .execute(&mut **tx)
            .await
            .map(|_| ())
        }
        DbTx::Postgres(tx) => {
            sqlx::query(
                "INSERT INTO agent_sessions
                    (session_id, conversation_id, project_id, host_device_id, agent_id, status, started_at_ms, ended_at_ms, idempotency_account_id, idempotency_key)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(session_id)
            .bind(conversation_id)
            .bind(project_id)
            .bind(host_device_id)
            .bind(agent_id)
            .bind(status)
            .bind(started_at_ms)
            .bind(ended_at_ms)
            .bind(idempotency_account_id)
            .bind(idempotency_key)
            .execute(&mut **tx)
            .await
            .map(|_| ())
        }
    }
    .map_err(|error| BackendError::StoreQuery {
        operation: "agent_sessions.insert_in_tx".into(),
        message: error.to_string(),
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_agent_turn_in_tx(
    tx: &mut DbTx<'_>,
    turn_id: &str,
    session_id: &str,
    turn_seq: i64,
    role: &str,
    status: &str,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    summary_text: Option<&str>,
    usage_json: Option<&str>,
) -> Result<(), BackendError> {
    match tx {
        DbTx::Sqlite(tx) => {
            sqlx::query(
                "INSERT INTO agent_turns
                    (turn_id, agent_session_id, turn_seq, role, status, started_at_ms, finished_at_ms, summary_text, usage_json)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(turn_id)
            .bind(session_id)
            .bind(turn_seq)
            .bind(role)
            .bind(status)
            .bind(started_at_ms)
            .bind(finished_at_ms)
            .bind(summary_text)
            .bind(usage_json)
            .execute(&mut **tx)
            .await
            .map(|_| ())
        }
        DbTx::Postgres(tx) => {
            sqlx::query(
                "INSERT INTO agent_turns
                    (turn_id, agent_session_id, turn_seq, role, status, started_at_ms, finished_at_ms, summary_text, usage_json)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(turn_id)
            .bind(session_id)
            .bind(turn_seq)
            .bind(role)
            .bind(status)
            .bind(started_at_ms)
            .bind(finished_at_ms)
            .bind(summary_text)
            .bind(usage_json)
            .execute(&mut **tx)
            .await
            .map(|_| ())
        }
    }
    .map_err(|error| BackendError::StoreQuery {
        operation: "agent_turns.insert_in_tx".into(),
        message: error.to_string(),
    })?;
    Ok(())
}

async fn update_agent_session_status_in_tx(
    tx: &mut DbTx<'_>,
    session_id: &str,
    status: &str,
    ended_at_ms: Option<i64>,
) -> Result<(), BackendError> {
    match tx {
        DbTx::Sqlite(tx) => sqlx::query(
            "UPDATE agent_sessions
                    SET status = ?, ended_at_ms = ?
                  WHERE session_id = ?",
        )
        .bind(status)
        .bind(ended_at_ms)
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .map(|_| ()),
        DbTx::Postgres(tx) => sqlx::query(
            "UPDATE agent_sessions
                    SET status = $1, ended_at_ms = $2
                  WHERE session_id = $3",
        )
        .bind(status)
        .bind(ended_at_ms)
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .map(|_| ()),
    }
    .map_err(|error| BackendError::StoreQuery {
        operation: "agent_sessions.update_status_in_tx".into(),
        message: error.to_string(),
    })?;
    Ok(())
}
