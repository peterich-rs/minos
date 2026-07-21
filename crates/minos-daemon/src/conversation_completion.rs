//! Daemon-owned agent result writeback and teamwork delegation completion.
//!
//! When a top-level conversation agent finishes a **turn** (not each intermediate
//! assistant message), this module:
//! 1. Upserts a durable conversation message (`agent-result:…`)
//! 2. Completes any running teamwork delegation for that thread
//! 3. Delivers the result to the source thread (or queues if busy)
//!
//! Timeline order is the durable insert order of `chat_messages.message_seq`
//! (finish/write order). Delegation results set `reply_to_message_id` to the
//! request message so UIs can show causality without reordering history.
//!
//! ## Final text = last assistant segment
//!
//! Grok/Gemini ACP emit intermediate `agent_message_chunk` progress between
//! tools (e.g. "正在定位…"). The Grok translator completes the open assistant
//! message at each tool boundary and opens a fresh `message_id` for post-tool
//! text (see `agent_msg_resets_after_tool` in grok.rs). Session `ChatState`
//! splits these into separate bubbles and only treats the **last** assistant
//! text block as the completed answer. Conversation writeback must match that:
//!
//! - `MessageCompleted` records any open, non-empty text segment under its own
//!   `message_id`.
//! - A subsequent tool / subagent interrupt marks all prior completed segments
//!   as **interrupted** — they were progress narration, not the final answer.
//! - `last_completed()` skips interrupted segments; if no clean (non-interrupted)
//!   segment remains, the turn is treated as progress-only and nothing is
//!   written back (no progress dump, no delegation completion).
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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use minos_agent_runtime::{AgentManager, ThreadState};
use minos_chat_store::{TeamworkSourceDeliveryStatus, TeamworkStore};
use minos_domain::AgentName;
use minos_protocol::{ConversationMention, LocalConversationEvent};
use minos_ui_protocol::{MessageRole, UiEventMessage};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, info, warn};

use crate::store::LocalStore;

#[derive(Default)]
struct ThreadProjection {
    agent: Option<AgentName>,
    /// Current open assistant text segment by message_id (within the turn).
    /// After a tool/reasoning interrupt, the next TextDelta starts a fresh
    /// segment so intermediate progress is not concatenated into the final body.
    assistant_text: HashMap<String, String>,
    assistant_roles: HashMap<String, MessageRole>,
    /// message_ids whose current text segment is closed (next text starts new).
    segment_closed: HashMap<String, bool>,
    /// tool_call_id → assistant message_id (so ToolCallCompleted can close segment).
    tool_message_ids: HashMap<String, String>,
    /// Ordered completed assistant results (message_key, text) for this turn.
    completed: Vec<(String, String)>,
    /// message_keys whose segment was followed by a non-text interrupt (tool /
    /// subagent). These represent intermediate progress narration, not the
    /// final answer — excluded from `last_completed()` unless a later, clean
    /// text segment re-recorded under a fresh key.
    interrupted_keys: HashSet<String>,
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
        self.segment_closed.clear();
        self.tool_message_ids.clear();
        self.completed.clear();
        self.interrupted_keys.clear();
        self.last_error = None;
        self.recorded_key = None;
        self.turn_recorded = false;
        self.write_in_flight = false;
        self.pending_boundary = false;
        self.turn_write_id = None;
    }

    fn close_text_segment(&mut self, message_id: &str) {
        if message_id.is_empty() {
            return;
        }
        self.segment_closed.insert(message_id.to_owned(), true);
    }

    /// A non-text timeline item (tool / subagent) arrived after some assistant
    /// text segments were already completed. Those completed segments represent
    /// intermediate progress narration, not the final turn answer — mark them
    /// interrupted so `last_completed()` skips them. If a later clean text
    /// segment completes under a fresh key, it becomes the new answer.
    fn interrupt_prior_completed(&mut self) {
        for (key, _) in &self.completed {
            self.interrupted_keys.insert(key.clone());
        }
    }

    fn append_assistant_text(&mut self, message_id: &str, text: String, replace: bool) {
        if message_id.is_empty() {
            return;
        }
        let start_new = self
            .segment_closed
            .get(message_id)
            .copied()
            .unwrap_or(false)
            || !self.assistant_text.contains_key(message_id);
        if replace || start_new {
            self.assistant_text.insert(message_id.to_owned(), text);
            self.segment_closed.insert(message_id.to_owned(), false);
        } else {
            self.assistant_text
                .entry(message_id.to_owned())
                .or_default()
                .push_str(&text);
        }
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
                        // Do not seed an empty segment — first TextDelta opens it.
                        self.segment_closed
                            .entry(message_id.clone())
                            .or_insert(true);
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
                        if rendered.is_empty() {
                            continue;
                        }
                        let replace = matches!(event, UiEventMessage::TextReplace { .. });
                        self.append_assistant_text(message_id, rendered, replace);
                    }
                }
                // Tool/subagent events break the open assistant text bubble
                // (same as ChatState). Next TextDelta is a new final-answer segment.
                // Pure reasoning (`ReasoningDelta`/`ReasoningReplace`) does NOT
                // close the segment — Grok intentionally reuses the same
                // message_id for thought interleaved with text, and Codex/Claude
                // emit reasoning under a different id that doesn't affect the
                // open text segment's lifecycle either way.
                UiEventMessage::ToolCallPlaced { message_id, .. } => {
                    if let UiEventMessage::ToolCallPlaced {
                        message_id,
                        tool_call_id,
                        ..
                    } = event
                    {
                        self.tool_message_ids
                            .insert(tool_call_id.clone(), message_id.clone());
                        // A tool interrupt invalidates any progress completed
                        // earlier in this turn (Grok emits MessageCompleted on
                        // the prior assistant message right before ToolCallPlaced).
                        self.interrupt_prior_completed();
                    }
                    self.close_text_segment(message_id);
                }
                UiEventMessage::ToolCallCompleted { tool_call_id, .. } => {
                    if let Some(message_id) = self.tool_message_ids.remove(tool_call_id) {
                        self.close_text_segment(&message_id);
                    } else {
                        // Unknown tool — close every open assistant segment so a
                        // post-tool final answer is not glued to progress text.
                        let ids: Vec<String> = self
                            .assistant_roles
                            .iter()
                            .filter(|(_, role)| matches!(role, MessageRole::Assistant))
                            .map(|(id, _)| id.clone())
                            .collect();
                        for id in ids {
                            self.close_text_segment(&id);
                        }
                    }
                }
                UiEventMessage::SubagentSpawned {
                    parent_thread_id: _,
                    tool_call_id,
                    ..
                } => {
                    if let Some(message_id) = self.tool_message_ids.get(tool_call_id).cloned() {
                        self.close_text_segment(&message_id);
                        self.interrupt_prior_completed();
                    }
                }
                UiEventMessage::MessageCompleted { message_id, .. } => {
                    // Only the last **open** segment. If tools/reasoning closed
                    // the segment and no further TextDelta arrived, do not fall
                    // back to pre-interrupt progress narration.
                    let segment_open = !self
                        .segment_closed
                        .get(message_id)
                        .copied()
                        .unwrap_or(false);
                    if segment_open {
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
            .iter()
            .rev()
            .find(|(key, _)| !self.interrupted_keys.contains(key))
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
            ThreadState::Starting | ThreadState::Resuming | ThreadState::Running { .. } => {
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
            .write_result(&conversation_id, thread_id, &durable_id, &text, agent)
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
        let teamwork =
            open_teamwork_store(&self.store, conversation_id, &self.default_workspace).await?;
        let delegation = teamwork
            .running_delegation_for_thread(conversation_id, thread_id)
            .await?;

        // durable_id is turn-scoped (claimed once) so concurrent try_record paths
        // upsert the same chat_messages row instead of inserting siblings.
        let message_id = format!("agent-result:{conversation_id}:{thread_id}:{durable_id}");
        let (body, reply_to, mentions, delegation_id) =
            if let Some(delegation) = delegation.as_ref() {
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
        let _ = self.local_conversation_event_tx.send(
            LocalConversationEvent::ConversationMessageAppended {
                conversation_id: conversation_id.to_owned(),
                message_seq,
            },
        );

        if delegation.is_some() {
            match teamwork
                .complete_delegation_for_thread(conversation_id, thread_id, Some(&message_id), text)
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

        match self
            .try_send_to_source(source_thread_id, &source_body)
            .await
        {
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

    fn tool_placed(message_id: &str, tool_call_id: &str) -> UiEventMessage {
        UiEventMessage::ToolCallPlaced {
            message_id: message_id.into(),
            tool_call_id: tool_call_id.into(),
            name: "read_file".into(),
            args_json: DisplayPayload::inline("{}"),
        }
    }

    fn tool_completed(tool_call_id: &str) -> UiEventMessage {
        UiEventMessage::ToolCallCompleted {
            tool_call_id: tool_call_id.into(),
            output: DisplayPayload::inline("ok"),
            is_error: false,
        }
    }

    fn reasoning(message_id: &str, text: &str) -> UiEventMessage {
        UiEventMessage::ReasoningDelta {
            message_id: message_id.into(),
            text: DisplayPayload::inline(text),
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
        assert_eq!(p.last_completed().map(|(_, t)| t), Some("partial".into()));
        assert!(!p.turn_recorded);
    }

    #[test]
    fn non_opencode_idle_first_then_completed_allows_record() {
        let mut p = ThreadProjection {
            agent: Some(AgentName::Codex),
            pending_boundary: true, // Idle latched first
            ..Default::default()
        };
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
            &[assistant_start("m1"), delta("m1", "done"), completed("m1")],
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
            &[assistant_start("m1"), delta("m1", "old"), completed("m1")],
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
        let mut p = ThreadProjection {
            agent: Some(AgentName::Codex),
            ..Default::default()
        };
        p.apply_events(
            AgentName::Codex,
            &[assistant_start("m1"), delta("m1", "bye"), completed("m1")],
        );
        assert!(!p.should_try_record_on_ingest(true, false));
        assert!(p.should_try_record_on_ingest(false, true));
    }

    #[test]
    fn claim_write_is_single_flight_per_turn() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Opencode,
            &[assistant_start("m1"), delta("m1", "final"), completed("m1")],
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
            &[assistant_start("m1"), delta("m1", "one"), completed("m1")],
        );
        assert!(p.should_try_record_on_ingest(true, false));
        let _ = p.claim_write();
        p.finish_write_ok();

        p.apply_events(
            AgentName::Opencode,
            &[assistant_start("m2"), delta("m2", "two"), completed("m2")],
        );
        // Ingest may still *want* to try, but claim blocks a second write.
        assert!(p.should_try_record_on_ingest(true, false));
        assert!(p.claim_write().is_none());
    }

    #[test]
    fn grok_style_progress_then_tools_then_final_keeps_only_last_segment() {
        // Mirrors session ChatState: intermediate agent_message_chunk progress
        // between tools must not be concatenated into conversation body.
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Grok,
            &[
                assistant_start("m1"),
                delta("m1", "正在定位详情页 composer…"),
                tool_placed("m1", "tc1"),
                tool_completed("tc1"),
                delta("m1", "发现键盘高度被错误塞进 bar。"),
                tool_placed("m1", "tc2"),
                tool_completed("tc2"),
                reasoning("m1", "I'll give the user a concise summary."),
                delta(
                    "m1",
                    "已完成：\n- 分支: bugfix/ios-topic-detail-quick-reply-keyboard\n- 提交: 811bd57",
                ),
                completed("m1"),
            ],
        );
        p.pending_boundary = true;
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some(
                "已完成：\n- 分支: bugfix/ios-topic-detail-quick-reply-keyboard\n- 提交: 811bd57"
                    .into()
            )
        );
        let claimed = p.claim_write().expect("claim final segment");
        assert_eq!(
            claimed.1,
            "已完成：\n- 分支: bugfix/ios-topic-detail-quick-reply-keyboard\n- 提交: 811bd57"
        );
    }

    #[test]
    fn continuous_text_without_interrupt_stays_concatenated() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Grok,
            &[
                assistant_start("m1"),
                delta("m1", "Hello "),
                delta("m1", "world"),
                completed("m1"),
            ],
        );
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("Hello world".into())
        );
    }

    #[test]
    fn progress_only_before_tools_without_final_segment_is_not_recorded() {
        // Turn ends after tools with no post-tool answer → do not dump progress.
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Grok,
            &[
                assistant_start("m1"),
                delta("m1", "正在核对当前代码与相关提交…"),
                tool_placed("m1", "tc1"),
                tool_completed("tc1"),
                completed("m1"),
            ],
        );
        assert!(p.last_completed().is_none());
        assert!(p.claim_write().is_none());
    }

    #[test]
    fn grok_multi_message_id_progress_then_tool_without_final_is_not_recorded() {
        // Real Grok translator shape (proven by grok.rs `agent_msg_resets_after_tool`):
        //   MessageStarted(m1) → TextDelta(m1, progress) → MessageCompleted(m1)
        //   → MessageStarted(m2) → ToolCallPlaced(m2, tc1) → ToolCallCompleted(tc1)
        // The tool event completes m1 first; m1's segment is open at that moment.
        // Turn ends with no post-tool final text → must NOT write progress back.
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Grok,
            &[
                assistant_start("m1"),
                delta("m1", "正在核对当前代码与相关提交…"),
                completed("m1"),
                assistant_start("m2"),
                tool_placed("m2", "tc1"),
                tool_completed("tc1"),
            ],
        );
        p.pending_boundary = true;
        assert!(
            p.last_completed().is_none(),
            "progress-only Grok turn must not surface as conversation result"
        );
        assert!(p.claim_write().is_none());
    }

    #[test]
    fn grok_multi_message_id_progress_then_tool_then_final_keeps_only_last() {
        // Real Grok shape with post-tool final answer under a fresh message_id:
        //   m1 progress → MessageCompleted(m1) → tool on m2 → m3 final text →
        //   MessageCompleted(m3). Only m3 must be written back.
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Grok,
            &[
                assistant_start("m1"),
                delta("m1", "正在定位详情页 composer…"),
                completed("m1"),
                assistant_start("m2"),
                tool_placed("m2", "tc1"),
                tool_completed("tc1"),
                assistant_start("m3"),
                delta("m3", "已完成：修复键盘高度"),
                completed("m3"),
            ],
        );
        p.pending_boundary = true;
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("已完成：修复键盘高度".into())
        );
        let claimed = p.claim_write().expect("claim final segment");
        assert_eq!(claimed.1, "已完成：修复键盘高度");
    }

    #[test]
    fn grok_thought_between_text_segments_keeps_both_halves() {
        // Grok deliberately reuses the same message_id for thought interleaved
        // with text (see grok.rs comment ~697-699). ReasoningDelta must NOT
        // close the text segment — both halves concatenate into the final body.
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Grok,
            &[
                assistant_start("m1"),
                delta("m1", "First half of answer."),
                reasoning("m1", "Let me think about the second half."),
                delta("m1", " Second half."),
                completed("m1"),
            ],
        );
        p.pending_boundary = true;
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("First half of answer. Second half.".into())
        );
    }

    #[test]
    fn simple_answer_without_tools_still_records() {
        let mut p = ThreadProjection::default();
        p.apply_events(
            AgentName::Codex,
            &[
                assistant_start("m1"),
                delta("m1", "fixed the bug"),
                completed("m1"),
            ],
        );
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("fixed the bug".into())
        );
    }
}
