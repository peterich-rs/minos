//! Daemon-owned agent result writeback and teamwork delegation completion.
//!
//! When a top-level conversation agent finishes a **turn** (not each intermediate
//! assistant message), this module:
//! 1. Upserts a durable conversation message (`agent-result:…`)
//! 2. Completes any running teamwork delegation for that thread
//! 3. Delivers the result to the source thread (or queues if busy)
//!
//! ## Turn-boundary latch
//!
//! Runtime `ThreadState::Idle`/`Closed` and ingest `MessageCompleted` race across
//! independent tasks. Neither alone is a safe write trigger for every agent:
//!
//! - Idle may arrive **before** the final `MessageCompleted` has been projected.
//! - `MessageCompleted` may fire mid-session for some agents / paths and must not
//!   durable-write conversation or complete a delegation early.
//!
//! Unified model:
//! - `Idle`/`Closed` → set `pending_boundary` and `try_record` if text is ready.
//! - Ingest terminal events → accumulate; `try_record` only when
//!   `pending_boundary`, `ThreadClosed`, or Opencode terminal complete.
//! - `Running` / user `MessageStarted` → reset turn-scoped projection fields.

use std::collections::HashMap;
use std::sync::Arc;

use minos_agent_runtime::{AgentManager, ThreadState};
use minos_chat_store::{
    TeamworkSourceDeliveryStatus, TeamworkStore,
};
use minos_domain::AgentName;
use minos_protocol::{ConversationMention, LocalConversationEvent};
use minos_ui_protocol::{MessageRole, UiEventMessage};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, info, warn};

use crate::store::LocalStore;

#[derive(Default)]
struct ThreadProjection {
    agent: Option<AgentName>,
    /// Accumulated assistant text by message_id (within the current turn).
    assistant_text: HashMap<String, String>,
    assistant_roles: HashMap<String, MessageRole>,
    /// Ordered completed assistant results (message_key, text) for this turn.
    completed: Vec<(String, String)>,
    last_error: Option<(String, String)>,
    /// message_key already written for this turn (within-turn dedupe).
    recorded_key: Option<String>,
    /// True after a successful conversation write for this turn.
    turn_recorded: bool,
    /// Claim held while `write_result` is in flight (prevents concurrent double write).
    write_in_flight: bool,
    /// Idle/Closed arrived; wait for last_completed if ingest is still lagging.
    pending_boundary: bool,
    /// Stable durable id suffix for this turn so races upsert one row.
    turn_write_id: Option<String>,
}

impl ThreadProjection {
    /// Clear turn-scoped fields so a new Running/user turn cannot resurrect
    /// the previous turn's `last_completed` after reset of write flags.
    fn begin_turn(&mut self) {
        self.assistant_text.clear();
        self.assistant_roles.clear();
        self.completed.clear();
        self.last_error = None;
        self.recorded_key = None;
        self.turn_recorded = false;
        self.write_in_flight = false;
        self.pending_boundary = false;
        self.turn_write_id = None;
    }

    /// Allocate a turn write id lazily on first successful claim.
    fn ensure_turn_write_id(&mut self, fallback_key: &str) -> String {
        if let Some(id) = self.turn_write_id.clone() {
            return id;
        }
        // Prefer first completed message key; fall back to a timestamped id.
        let id = if fallback_key.is_empty() {
            format!("t{}", current_unix_ms())
        } else {
            fallback_key.to_owned()
        };
        self.turn_write_id = Some(id.clone());
        id
    }

    /// Atomically claim the right to write this turn's conversation result.
    /// Returns `(source_message_key, text, durable_write_id)`.
    fn claim_write(&mut self) -> Option<(String, String, String)> {
        if self.turn_recorded || self.write_in_flight {
            return None;
        }
        let (key, text) = self.last_completed()?;
        let durable = self.ensure_turn_write_id(&key);
        self.write_in_flight = true;
        self.recorded_key = Some(key.clone());
        Some((key, text, durable))
    }

    fn finish_write_ok(&mut self) {
        self.turn_recorded = true;
        self.write_in_flight = false;
        self.pending_boundary = false;
    }

    fn finish_write_err(&mut self) {
        // Allow a later Idle/MessageCompleted retry; keep recorded_key so the
        // same durable id is reused via turn_write_id.
        self.write_in_flight = false;
    }

    fn apply_events(&mut self, agent: AgentName, events: &[UiEventMessage]) {
        self.agent = Some(agent);
        for event in events {
            match event {
                UiEventMessage::MessageStarted {
                    message_id, role, ..
                } => {
                    // A new user message starts a turn even if ThreadState::Running
                    // was missed or reordered relative to ingest.
                    if matches!(role, MessageRole::User) {
                        self.begin_turn();
                        self.agent = Some(agent);
                    }
                    self.assistant_roles.insert(message_id.clone(), *role);
                    if matches!(role, MessageRole::Assistant) {
                        self.assistant_text.entry(message_id.clone()).or_default();
                    }
                }
                UiEventMessage::TextDelta { message_id, text }
                | UiEventMessage::TextReplace { message_id, text } => {
                    if self
                        .assistant_roles
                        .get(message_id)
                        .is_some_and(|role| matches!(role, MessageRole::Assistant))
                        || !self.assistant_roles.contains_key(message_id)
                    {
                        let rendered = text.render_preview();
                        match event {
                            UiEventMessage::TextReplace { .. } => {
                                self.assistant_text
                                    .insert(message_id.clone(), rendered);
                            }
                            _ => {
                                self.assistant_text
                                    .entry(message_id.clone())
                                    .or_default()
                                    .push_str(&rendered);
                            }
                        }
                    }
                }
                UiEventMessage::MessageCompleted { message_id, .. } => {
                    if let Some(text) = self.assistant_text.get(message_id) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            let key = if message_id.is_empty() {
                                format!("text:{trimmed}")
                            } else {
                                message_id.clone()
                            };
                            self.completed.retain(|(existing, _)| existing != &key);
                            self.completed.push((key, trimmed.to_owned()));
                        }
                    }
                }
                UiEventMessage::ThreadClosed { .. } => {}
                UiEventMessage::Error {
                    message,
                    message_id,
                    ..
                } => {
                    let text = message.trim();
                    if !text.is_empty() {
                        let key = message_id
                            .as_deref()
                            .filter(|id| !id.is_empty())
                            .map(|id| format!("error:{id}"))
                            .unwrap_or_else(|| format!("error:{text}"));
                        self.last_error = Some((key, text.to_owned()));
                    }
                }
                _ => {}
            }
        }
    }

    fn last_completed(&self) -> Option<(String, String)> {
        self.completed
            .last()
            .cloned()
            .or_else(|| self.last_error.clone())
    }

    /// Whether an ingest terminal frame may call `try_record` right now.
    fn should_try_record_on_ingest(
        &self,
        has_message_completed: bool,
        has_thread_closed: bool,
    ) -> bool {
        if !(has_message_completed || has_thread_closed) {
            return false;
        }
        // ThreadClosed is always a boundary (thread gone).
        if has_thread_closed {
            return true;
        }
        // Idle already latched — MessageCompleted may be catching up.
        if self.pending_boundary {
            return true;
        }
        // Opencode emits terminal MessageCompleted only (finish:stop / idle);
        // it must not depend solely on Idle which can race ahead of text.
        matches!(self.agent, Some(AgentName::Opencode))
    }
}

#[derive(Clone)]
pub(crate) struct ConversationCompletion {
    store: Arc<LocalStore>,
    manager: Arc<AgentManager>,
    default_workspace: std::path::PathBuf,
    local_conversation_event_tx: broadcast::Sender<LocalConversationEvent>,
    projections: Arc<Mutex<HashMap<String, ThreadProjection>>>,
}

impl ConversationCompletion {
    pub(crate) fn new(
        store: Arc<LocalStore>,
        manager: Arc<AgentManager>,
        default_workspace: std::path::PathBuf,
        local_conversation_event_tx: broadcast::Sender<LocalConversationEvent>,
    ) -> Self {
        Self {
            store,
            manager,
            default_workspace,
            local_conversation_event_tx,
            projections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn on_ingest_frame(
        &self,
        thread_id: &str,
        agent: AgentName,
        ui_events: &[UiEventMessage],
    ) {
        let has_message_completed = ui_events
            .iter()
            .any(|event| matches!(event, UiEventMessage::MessageCompleted { .. }));
        let has_thread_closed = ui_events
            .iter()
            .any(|event| matches!(event, UiEventMessage::ThreadClosed { .. }));

        let should_try = {
            let mut projections = self.projections.lock().await;
            let projection = projections.entry(thread_id.to_owned()).or_default();
            projection.apply_events(agent, ui_events);
            projection.should_try_record_on_ingest(has_message_completed, has_thread_closed)
        };
        if should_try {
            self.try_record_result(thread_id).await;
        }
    }

    pub(crate) async fn on_thread_state(&self, thread_id: &str, state: &ThreadState) {
        match state {
            ThreadState::Starting
            | ThreadState::Resuming
            | ThreadState::Running { .. } => {
                let mut projections = self.projections.lock().await;
                projections
                    .entry(thread_id.to_owned())
                    .or_default()
                    .begin_turn();
            }
            ThreadState::Idle | ThreadState::Closed { .. } => {
                {
                    let mut projections = self.projections.lock().await;
                    let projection = projections.entry(thread_id.to_owned()).or_default();
                    projection.pending_boundary = true;
                }
                self.try_record_result(thread_id).await;
                self.flush_pending_source_deliveries(thread_id).await;
            }
            ThreadState::Suspended { .. } => {
                // Crash/suspend does not complete a turn into the conversation
                // timeline (product: no partial write on instance death).
            }
        }
    }

    async fn try_record_result(&self, thread_id: &str) {
        let thread_row = match self.store.get_thread(thread_id).await {
            Ok(Some(row)) => row,
            Ok(None) => return,
            Err(error) => {
                warn!(
                    target: "minos_daemon::conversation_completion",
                    error = %error,
                    thread_id,
                    "failed to load thread for completion"
                );
                return;
            }
        };
        if thread_row.parent_thread_id.is_some() {
            return;
        }
        if thread_row.conversation_id.trim().is_empty() {
            return;
        }
        let conversation_id = thread_row.conversation_id.clone();

        let (message_key, text, durable_id, agent) = {
            let mut projections = self.projections.lock().await;
            let Some(projection) = projections.get_mut(thread_id) else {
                return;
            };
            let Some((key, text, durable)) = projection.claim_write() else {
                // Already recorded, write in flight, or no completed text yet.
                return;
            };
            let agent = projection
                .agent
                .or_else(|| parse_agent_label(&thread_row.agent));
            (key, text, durable, agent)
        };

        if text.trim().is_empty() {
            let mut projections = self.projections.lock().await;
            if let Some(projection) = projections.get_mut(thread_id) {
                projection.finish_write_err();
            }
            return;
        }

        if let Err(error) = self
            .write_result(
                &conversation_id,
                thread_id,
                &durable_id,
                &text,
                agent,
            )
            .await
        {
            warn!(
                target: "minos_daemon::conversation_completion",
                error = %error,
                conversation_id = %conversation_id,
                thread_id,
                message_key = %message_key,
                "failed to write agent conversation result"
            );
            let mut projections = self.projections.lock().await;
            if let Some(projection) = projections.get_mut(thread_id) {
                projection.finish_write_err();
            }
            return;
        }

        let mut projections = self.projections.lock().await;
        if let Some(projection) = projections.get_mut(thread_id) {
            projection.finish_write_ok();
        }
    }

    async fn write_result(
        &self,
        conversation_id: &str,
        thread_id: &str,
        durable_id: &str,
        text: &str,
        agent: Option<AgentName>,
    ) -> anyhow::Result<()> {
        let teamwork = open_teamwork_store(&self.store, conversation_id, &self.default_workspace).await?;
        let delegation = teamwork
            .running_delegation_for_thread(conversation_id, thread_id)
            .await?;

        // durable_id is turn-scoped (claimed once) so concurrent try_record paths
        // upsert the same chat_messages row instead of inserting siblings.
        let message_id = format!("agent-result:{conversation_id}:{thread_id}:{durable_id}");
        let (body, reply_to, mentions, delegation_id) = if let Some(delegation) = delegation.as_ref()
        {
            let source_agent = delegation.source_agent;
            let source_thread = delegation.source_thread_id.clone();
            let short = source_thread
                .as_deref()
                .map(short_thread_id)
                .unwrap_or_else(|| "unknown".into());
            let body = match source_agent {
                Some(source) => format!("@{}#{} {}", source.bin_name(), short, text.trim()),
                None => text.trim().to_owned(),
            };
            let mentions = source_agent
                .map(|source| {
                    vec![ConversationMention {
                        agent: source,
                        thread_id: source_thread.clone(),
                        thread_short_id: Some(short),
                    }]
                })
                .unwrap_or_default();
            (
                body,
                delegation.request_message_id.clone(),
                mentions,
                Some(delegation.delegation_id.clone()),
            )
        } else {
            (text.trim().to_owned(), None, Vec::new(), None)
        };

        let agent_label = agent.map(|a| a.bin_name().to_owned());
        let mentions_json = serde_json::to_string(&mentions).unwrap_or_else(|_| "[]".into());
        let now = current_unix_ms();
        let message_seq = self
            .store
            .upsert_conversation_message(
                conversation_id,
                &message_id,
                Some(thread_id),
                "agent",
                agent_label.as_deref(),
                &body,
                now,
                reply_to.as_deref(),
                delegation_id.as_deref(),
                &mentions_json,
            )
            .await?;
        let _ = self.local_conversation_event_tx.send(LocalConversationEvent::ConversationMessageAppended {
            conversation_id: conversation_id.to_owned(),
            message_seq,
        });

        if delegation.is_some() {
            match teamwork
                .complete_delegation_for_thread(
                    conversation_id,
                    thread_id,
                    Some(&message_id),
                    text,
                )
                .await
            {
                Ok(Some(completed)) => {
                    self.deliver_to_source(&teamwork, &completed, thread_id, &body)
                        .await;
                }
                Ok(None) => {}
                Err(error) => {
                    debug!(
                        target: "minos_daemon::conversation_completion",
                        error = %error,
                        "complete_delegation skipped"
                    );
                }
            }
        }

        info!(
            target: "minos_daemon::conversation_completion",
            conversation_id,
            thread_id,
            message_id = %message_id,
            "recorded agent conversation result"
        );
        Ok(())
    }

    async fn deliver_to_source(
        &self,
        teamwork: &TeamworkStore,
        delegation: &minos_chat_store::TeamworkDelegation,
        target_thread_id: &str,
        visible_body: &str,
    ) {
        let Some(source_thread_id) = delegation.source_thread_id.as_deref() else {
            return;
        };
        if source_thread_id == target_thread_id {
            return;
        }
        let source_body = format!(
            "[{}#{}] {}",
            delegation.target_agent.bin_name(),
            short_thread_id(target_thread_id),
            visible_body
        );

        match self.try_send_to_source(source_thread_id, &source_body).await {
            Ok(()) => {
                if let Ok(delivery) = teamwork
                    .enqueue_source_delivery(
                        &delegation.conversation_id,
                        &delegation.delegation_id,
                        source_thread_id,
                        &source_body,
                    )
                    .await
                {
                    let _ = teamwork
                        .mark_source_delivery(
                            &delivery.delivery_id,
                            TeamworkSourceDeliveryStatus::Delivered,
                            None,
                        )
                        .await;
                }
            }
            Err(error) if should_queue_delivery(&error) => {
                warn!(
                    target: "minos_daemon::conversation_completion",
                    error = %error,
                    source_thread_id,
                    "source busy; queueing delegation result delivery"
                );
                let _ = teamwork
                    .enqueue_source_delivery(
                        &delegation.conversation_id,
                        &delegation.delegation_id,
                        source_thread_id,
                        &source_body,
                    )
                    .await;
            }
            Err(error) => {
                warn!(
                    target: "minos_daemon::conversation_completion",
                    error = %error,
                    source_thread_id,
                    "failed to deliver delegation result to source"
                );
                if let Ok(delivery) = teamwork
                    .enqueue_source_delivery(
                        &delegation.conversation_id,
                        &delegation.delegation_id,
                        source_thread_id,
                        &source_body,
                    )
                    .await
                {
                    let _ = teamwork
                        .mark_source_delivery(
                            &delivery.delivery_id,
                            TeamworkSourceDeliveryStatus::Failed,
                            Some(&error.to_string()),
                        )
                        .await;
                }
            }
        }
    }

    async fn try_send_to_source(&self, source_thread_id: &str, body: &str) -> anyhow::Result<()> {
        self.manager
            .send_user_message(source_thread_id, body.to_owned())
            .await
    }

    async fn flush_pending_source_deliveries(&self, source_thread_id: &str) {
        let Ok(teamwork) =
            open_teamwork_store(&self.store, "unused", &self.default_workspace).await
        else {
            return;
        };
        let Ok(pending) = teamwork
            .list_pending_source_deliveries_for_thread(source_thread_id)
            .await
        else {
            return;
        };
        for delivery in pending {
            match self
                .try_send_to_source(source_thread_id, &delivery.body)
                .await
            {
                Ok(()) => {
                    let _ = teamwork
                        .mark_source_delivery(
                            &delivery.delivery_id,
                            TeamworkSourceDeliveryStatus::Delivered,
                            None,
                        )
                        .await;
                }
                Err(error) if should_queue_delivery(&error) => {
                    // Keep pending.
                }
                Err(error) => {
                    let _ = teamwork
                        .mark_source_delivery(
                            &delivery.delivery_id,
                            TeamworkSourceDeliveryStatus::Failed,
                            Some(&error.to_string()),
                        )
                        .await;
                }
            }
        }
    }
}

fn should_queue_delivery(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("turn is already running")
        || message.contains("send_user_message rejected: state=Starting")
        || message.contains("send_user_message rejected: state=Resuming")
}

async fn open_teamwork_store(
    store: &LocalStore,
    conversation_id: &str,
    default_workspace: &std::path::Path,
) -> anyhow::Result<TeamworkStore> {
    let teamwork = TeamworkStore::open(store.db_path()).await?;
    let title = conversation_id;
    let workspace = default_workspace.display().to_string();
    let _ = teamwork
        .ensure_conversation(conversation_id, title, &workspace)
        .await;
    Ok(teamwork)
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id[..8.min(thread_id.len())].to_owned()
}

fn parse_agent_label(value: &str) -> Option<AgentName> {
    match value {
        "codex" => Some(AgentName::Codex),
        "claude" => Some(AgentName::Claude),
        "gemini" => Some(AgentName::Gemini),
        "opencode" => Some(AgentName::Opencode),
        "grok" => Some(AgentName::Grok),
        _ => None,
    }
}

fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_ui_protocol::DisplayPayload;

    fn assistant_start(id: &str) -> UiEventMessage {
        UiEventMessage::MessageStarted {
            message_id: id.into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        }
    }

    fn user_start(id: &str) -> UiEventMessage {
        UiEventMessage::MessageStarted {
            message_id: id.into(),
            role: MessageRole::User,
            started_at_ms: 0,
        }
    }

    fn delta(id: &str, text: &str) -> UiEventMessage {
        UiEventMessage::TextDelta {
            message_id: id.into(),
            text: DisplayPayload::inline(text),
        }
    }

    fn completed(id: &str) -> UiEventMessage {
        UiEventMessage::MessageCompleted {
            message_id: id.into(),
            finished_at_ms: 1,
        }
    }

    #[test]
    fn non_opencode_message_completed_without_boundary_does_not_try_record() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Grok,
            &[
                assistant_start("m1"),
                delta("m1", "partial"),
                completed("m1"),
            ],
        );
        assert!(!p.should_try_record_on_ingest(true, false));
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("partial".into())
        );
        assert!(!p.turn_recorded);
    }

    #[test]
    fn non_opencode_idle_first_then_completed_allows_record() {
        let mut p = ThreadProjection::default();
        p.agent = Some(AgentName::Codex);
        p.pending_boundary = true; // Idle latched first
        p.apply_events(
            AgentName::Codex,
            &[
                assistant_start("m1"),
                delta("m1", "final answer"),
                completed("m1"),
            ],
        );
        assert!(p.should_try_record_on_ingest(true, false));
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("final answer".into())
        );
    }

    #[test]
    fn multi_completed_defers_until_boundary_and_takes_last() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Grok,
            &[
                assistant_start("m1"),
                delta("m1", "step1"),
                completed("m1"),
                assistant_start("m2"),
                delta("m2", "final"),
                completed("m2"),
            ],
        );
        // Mid-turn completes must not try_record.
        assert!(!p.should_try_record_on_ingest(true, false));
        assert_eq!(p.last_completed().map(|(_, t)| t), Some("final".into()));

        p.pending_boundary = true;
        assert!(p.should_try_record_on_ingest(true, false));
        assert_eq!(p.last_completed().map(|(_, t)| t), Some("final".into()));
    }

    #[test]
    fn opencode_terminal_completed_can_record_without_idle() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Opencode,
            &[
                assistant_start("m1"),
                delta("m1", "done"),
                completed("m1"),
            ],
        );
        assert!(p.should_try_record_on_ingest(true, false));
    }

    #[test]
    fn begin_turn_clears_prior_completed_so_cancel_cannot_resurrect() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Claude,
            &[
                assistant_start("m1"),
                delta("m1", "old turn"),
                completed("m1"),
            ],
        );
        assert_eq!(p.last_completed().map(|(_, t)| t), Some("old turn".into()));

        p.begin_turn();
        assert!(p.last_completed().is_none());
        assert!(!p.turn_recorded);
        assert!(!p.pending_boundary);
        assert!(p.recorded_key.is_none());
    }

    #[test]
    fn user_message_started_resets_turn_scope() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Gemini,
            &[
                assistant_start("m1"),
                delta("m1", "old"),
                completed("m1"),
            ],
        );
        p.pending_boundary = true;
        p.turn_recorded = true;
        p.recorded_key = Some("m1".into());

        p.apply_events(AgentName::Gemini, &[user_start("u2")]);
        assert!(p.last_completed().is_none());
        assert!(!p.turn_recorded);
        assert!(!p.pending_boundary);
        assert!(p.recorded_key.is_none());
    }

    #[test]
    fn thread_closed_on_ingest_always_allows_try_record() {
        let mut p = ThreadProjection::default();
        p.agent = Some(AgentName::Codex);
        p.apply_events(
            AgentName::Codex,
            &[
                assistant_start("m1"),
                delta("m1", "bye"),
                completed("m1"),
            ],
        );
        assert!(!p.should_try_record_on_ingest(true, false));
        assert!(p.should_try_record_on_ingest(false, true));
    }

    #[test]
    fn claim_write_is_single_flight_per_turn() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Opencode,
            &[
                assistant_start("m1"),
                delta("m1", "final"),
                completed("m1"),
            ],
        );
        let first = p.claim_write().expect("first claim");
        assert_eq!(first.1, "final");
        assert!(p.write_in_flight);
        // Concurrent second claim (Idle racing MessageCompleted) must not
        // produce a second durable write with another message_id.
        assert!(p.claim_write().is_none());

        p.finish_write_ok();
        assert!(p.turn_recorded);
        assert!(!p.write_in_flight);
        assert!(p.claim_write().is_none());
    }

    #[test]
    fn claim_write_reuses_stable_durable_id_after_failed_write() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Opencode,
            &[
                assistant_start("m1"),
                delta("m1", "answer"),
                completed("m1"),
            ],
        );
        let first = p.claim_write().expect("claim");
        let durable = first.2.clone();
        p.finish_write_err();
        assert!(!p.turn_recorded);

        // A later MessageCompleted with a different key must still upsert the
        // same durable agent-result row for this turn.
        p.apply_events(
            AgentName::Opencode,
            &[
                assistant_start("m2"),
                delta("m2", "answer v2"),
                completed("m2"),
            ],
        );
        let second = p.claim_write().expect("retry claim");
        assert_eq!(second.2, durable);
        assert_eq!(second.1, "answer v2");
    }

    #[test]
    fn opencode_second_message_completed_does_not_try_after_recorded() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Opencode,
            &[
                assistant_start("m1"),
                delta("m1", "one"),
                completed("m1"),
            ],
        );
        assert!(p.should_try_record_on_ingest(true, false));
        let _ = p.claim_write();
        p.finish_write_ok();

        p.apply_events(
            AgentName::Opencode,
            &[
                assistant_start("m2"),
                delta("m2", "two"),
                completed("m2"),
            ],
        );
        // Ingest may still *want* to try, but claim blocks a second write.
        assert!(p.should_try_record_on_ingest(true, false));
        assert!(p.claim_write().is_none());
    }
}
