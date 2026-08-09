//! Daemon-owned agent result writeback and teamwork delegation completion.
//!
//! When a top-level conversation agent finishes a **turn** (not each intermediate
//! assistant message), this module:
//! 1. Upserts a durable conversation message (`agent-result:…`) **local-only**
//! 2. Completes any running teamwork delegation for that session
//! 3. Delivers the result to the source thread (or queues if busy)
//!
//! ## Multi-end (Linked) vs local-only
//!
//! - **Local-only Desktop / unauthenticated**: this module is the conversation
//!   timeline writer for agent final text (Host SQLite SSOT for the workbench).
//! - **Linked multi-end IM**: Hub [`TurnCompletionProjector`] is the multi-end
//!   writer for other devices. Local `agent-result:…` rows still write for the
//!   Host workbench timeline. Desktop merge: Hub wins on same `message_id`,
//!   otherwise **gap-fills** local agent-result so native Desktop runs are not
//!   blank while Hub projection lags.
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
//! Runtime `SessionState::Idle`/`Closed` and ingest `MessageCompleted` race across
//! independent tasks. Neither alone is a safe write trigger for every agent:
//!
//! - Idle may arrive **before** the final `MessageCompleted` has been projected.
//! - `MessageCompleted` may fire mid-session for some agents / paths and must not
//!   durable-write conversation or complete a delegation early.
//!
//! Unified model:
//! - `Idle`/`Closed` → set `pending_boundary` and `try_record` if text is ready.
//! - Ingest terminal events → accumulate; `try_record` only when
//!   `pending_boundary`, `SessionClosed`, or Opencode terminal complete.
//! - `Running` / user `MessageStarted` → reset turn-scoped projection fields.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use minos_agent_runtime::{AgentManager, SessionState};
use minos_chat_store::{TeamworkSourceDeliveryStatus, TeamworkStore};
use minos_domain::AgentName;
use minos_protocol::{ConversationMention, LocalConversationEvent};
use minos_ui_protocol::{MessageRole, UiEventMessage};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, info, warn};

use crate::store::LocalStore;

#[derive(Default)]
struct SessionProjection {
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
    /// Prefer frozen `origin_message_id` (user Hub message) when set.
    turn_write_id: Option<String>,
    /// Origin user message id for agent-result:{conv}:{session}:{origin}.
    origin_message_id: Option<String>,
    /// Staged origin applied on the next [`begin_turn`] (survives user MessageStarted reset).
    pending_origin_message_id: Option<String>,
    /// Hub collab / Linked turns must not fall back to message_key/t{ms}.
    /// Set when a Hub conversation is bound or origin is staged for collab.
    require_canonical_origin: bool,
}

impl SessionProjection {
    /// Clear turn-scoped fields so a new Running/user turn cannot resurrect
    /// the previous turn's `last_completed` after reset of write flags.
    ///
    /// Origin handling (Desktop/Hub):
    /// - Freshly staged `pending_origin_message_id` always wins (promote).
    /// - The same logical turn often gets **two** `begin_turn` calls: runtime
    ///   `Running` and synth/user `MessageStarted`. The second must **not**
    ///   wipe an already-pinned origin while `require_canonical_origin` stays
    ///   true — that combination skips `agent-result` entirely (Grok/Claude
    ///   Desktop turns finish in the session but never land in conversation).
    /// - A true subsequent turn without a newly staged origin drops the pin so
    ///   durable ids cannot leak across turns.
    fn begin_turn(&mut self) {
        let staged = self.pending_origin_message_id.take();
        // Compute before clearing turn-scoped buffers.
        let redundant_same_turn = staged.is_none()
            && self.origin_message_id.is_some()
            && self.completed.is_empty()
            && self.assistant_text.is_empty()
            && !self.turn_recorded
            && !self.write_in_flight;

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

        if let Some(origin) = staged {
            self.origin_message_id = Some(origin.clone());
            self.turn_write_id = Some(origin);
        } else if redundant_same_turn {
            // Keep pinned origin / durable id across Running + user MessageStarted.
            if let Some(origin) = self.origin_message_id.clone() {
                self.turn_write_id = Some(origin);
            }
        } else {
            self.origin_message_id = None;
            self.turn_write_id = None;
        }
    }

    /// Stage origin for the next turn (or apply immediately if turn already open).
    fn set_origin_message_id(&mut self, origin: impl Into<String>) {
        let origin = origin.into();
        if origin.is_empty() {
            return;
        }
        self.pending_origin_message_id = Some(origin.clone());
        // If a turn is already open (post-begin_turn), pin durable id now.
        // Also pin immediately so a redundant begin_turn (before promote) can
        // recognize same-turn and preserve the Desktop/Hub message id.
        self.origin_message_id = Some(origin.clone());
        self.turn_write_id = Some(origin);
        // Do NOT sticky-set require_canonical_origin here. Hub/collab sessions
        // call note_require_canonical_origin / set_require_canonical_origin
        // separately. Local turns may stage an origin without forbidding
        // message_key fallback on a later origin-less turn.
    }

    /// Mark this session as Hub collab so completion refuses non-canonical ids.
    fn set_require_canonical_origin(&mut self) {
        self.require_canonical_origin = true;
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
    ///
    /// Frozen formula suffix = `origin_message_id` when known (always preferred,
    /// even over a previously claimed fallback). Else message key or timestamp
    /// for pure local/non-collab paths without origin.
    ///
    /// Returns `None` when collab requires origin and none is available (skip
    /// non-canonical agent-result rather than invent message_key/t{ms}).
    fn ensure_turn_write_id(&mut self, fallback_key: &str) -> Option<String> {
        // Origin always wins (collab / Desktop-native with wired user message id).
        if let Some(origin) = self
            .origin_message_id
            .clone()
            .or_else(|| self.pending_origin_message_id.clone())
        {
            self.origin_message_id = Some(origin.clone());
            self.turn_write_id = Some(origin.clone());
            return Some(origin);
        }
        if self.require_canonical_origin {
            // Fail-visible: collab must not mint non-canonical agent-result ids.
            return None;
        }
        if let Some(id) = self.turn_write_id.clone() {
            return Some(id);
        }
        // Prefer first completed message key; fall back to a timestamped id.
        let id = if fallback_key.is_empty() {
            format!("t{}", current_unix_ms())
        } else {
            fallback_key.to_owned()
        };
        self.turn_write_id = Some(id.clone());
        Some(id)
    }

    /// Atomically claim the right to write this turn's conversation result.
    /// Returns `(source_message_key, text, durable_write_id)`.
    fn claim_write(&mut self) -> Option<(String, String, String)> {
        if self.turn_recorded || self.write_in_flight {
            return None;
        }
        let (key, text) = self.last_completed()?;
        let durable = self.ensure_turn_write_id(&key)?;
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
                    // A new user message starts a turn even if SessionState::Running
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
                    parent_session_id: _,
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
                UiEventMessage::SessionClosed { .. } => {}
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
        // SessionClosed is always a boundary (thread gone).
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
    projections: Arc<Mutex<HashMap<String, SessionProjection>>>,
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
        session_id: &str,
        agent: AgentName,
        ui_events: &[UiEventMessage],
    ) {
        let has_message_completed = ui_events
            .iter()
            .any(|event| matches!(event, UiEventMessage::MessageCompleted { .. }));
        let has_thread_closed = ui_events
            .iter()
            .any(|event| matches!(event, UiEventMessage::SessionClosed { .. }));

        let should_try = {
            let mut projections = self.projections.lock().await;
            let projection = projections.entry(session_id.to_owned()).or_default();
            projection.apply_events(agent, ui_events);
            projection.should_try_record_on_ingest(has_message_completed, has_thread_closed)
        };
        if should_try {
            self.try_record_result(session_id).await;
        }
    }

    /// Note the user message id that triggered the current agent turn.
    /// Used for frozen `agent-result:{conv}:{session}:{origin_message_id}` ids.
    pub(crate) async fn note_turn_origin(&self, session_id: &str, origin_message_id: &str) {
        if origin_message_id.is_empty() {
            return;
        }
        let mut projections = self.projections.lock().await;
        let projection = projections.entry(session_id.to_owned()).or_default();
        projection.set_origin_message_id(origin_message_id);
    }

    /// Hub collab session: completion must not fall back to message_key/t{ms}.
    pub(crate) async fn note_require_canonical_origin(&self, session_id: &str) {
        let mut projections = self.projections.lock().await;
        let projection = projections.entry(session_id.to_owned()).or_default();
        projection.set_require_canonical_origin();
    }

    pub(crate) async fn on_session_state(&self, session_id: &str, state: &SessionState) {
        match state {
            SessionState::Starting | SessionState::Resuming | SessionState::Running { .. } => {
                let mut projections = self.projections.lock().await;
                projections
                    .entry(session_id.to_owned())
                    .or_default()
                    .begin_turn();
            }
            SessionState::Idle | SessionState::Closed { .. } => {
                {
                    let mut projections = self.projections.lock().await;
                    let projection = projections.entry(session_id.to_owned()).or_default();
                    projection.pending_boundary = true;
                }
                self.try_record_result(session_id).await;
                self.flush_pending_source_deliveries(session_id).await;
            }
            SessionState::Suspended { .. } => {
                // Crash/suspend does not complete a turn into the conversation
                // timeline (product: no partial write on instance death). Still
                // flush any queued source deliveries so teamwork handoffs are
                // not stuck behind a dead session.
                self.flush_pending_source_deliveries(session_id).await;
            }
        }
    }

    async fn try_record_result(&self, session_id: &str) {
        let thread_row = match self.store.get_session(session_id).await {
            Ok(Some(row)) => row,
            Ok(None) => return,
            Err(error) => {
                warn!(
                    target: "minos_daemon::conversation_completion",
                    error = %error,
                    session_id,
                    "failed to load thread for completion"
                );
                return;
            }
        };
        if thread_row.parent_session_id.is_some() {
            return;
        }
        if thread_row.conversation_id.trim().is_empty() {
            return;
        }
        let conversation_id = thread_row.conversation_id.clone();

        let (message_key, text, durable_id, agent) = {
            let mut projections = self.projections.lock().await;
            let Some(projection) = projections.get_mut(session_id) else {
                return;
            };
            let require_origin = projection.require_canonical_origin;
            let has_origin = projection
                .origin_message_id
                .as_ref()
                .or(projection.pending_origin_message_id.as_ref())
                .is_some();
            let Some((key, text, durable)) = projection.claim_write() else {
                // Already recorded, write in flight, no completed text, or collab
                // missing origin (hard contract: skip non-canonical agent-result).
                if require_origin && !has_origin && projection.last_completed().is_some() {
                    warn!(
                        target: "minos_daemon::conversation_completion",
                        session_id,
                        conversation_id = %conversation_id,
                        "skip agent-result write: collab requires origin_message_id (no message_key/t{{ms}} fallback)"
                    );
                }
                return;
            };
            let agent = projection
                .agent
                .or_else(|| parse_agent_label(&thread_row.agent));
            (key, text, durable, agent)
        };

        if text.trim().is_empty() {
            let mut projections = self.projections.lock().await;
            if let Some(projection) = projections.get_mut(session_id) {
                projection.finish_write_err();
            }
            return;
        }

        if let Err(error) = self
            .write_result(&conversation_id, session_id, &durable_id, &text, agent)
            .await
        {
            warn!(
                target: "minos_daemon::conversation_completion",
                error = %error,
                conversation_id = %conversation_id,
                session_id,
                message_key = %message_key,
                "failed to write agent conversation result"
            );
            let mut projections = self.projections.lock().await;
            if let Some(projection) = projections.get_mut(session_id) {
                projection.finish_write_err();
            }
            return;
        }

        let mut projections = self.projections.lock().await;
        if let Some(projection) = projections.get_mut(session_id) {
            projection.finish_write_ok();
        }
    }

    async fn write_result(
        &self,
        conversation_id: &str,
        session_id: &str,
        durable_id: &str,
        text: &str,
        agent: Option<AgentName>,
    ) -> anyhow::Result<()> {
        let teamwork =
            open_teamwork_store(&self.store, conversation_id, &self.default_workspace).await?;
        let delegation = teamwork
            .running_delegation_for_thread(conversation_id, session_id)
            .await?;

        // durable_id is turn-scoped (claimed once) so concurrent try_record paths
        // upsert the same chat_messages row instead of inserting siblings.
        let message_id = format!("agent-result:{conversation_id}:{session_id}:{durable_id}");
        let (body, reply_to, mentions, delegation_id) =
            if let Some(delegation) = delegation.as_ref() {
                let source_agent = delegation.source_agent;
                let source_session = delegation.source_session_id.clone();
                let short = source_session
                    .as_deref()
                    .map(short_session_id)
                    .unwrap_or_else(|| "unknown".into());
                let body = match source_agent {
                    Some(source) => format!("@{}#{} {}", source.bin_name(), short, text.trim()),
                    None => text.trim().to_owned(),
                };
                let mentions = source_agent
                    .map(|source| {
                        vec![ConversationMention {
                            agent: source,
                            session_id: source_session.clone(),
                            session_short_id: Some(short),
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
                Some(session_id),
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

        // Result bubble + delegation completion are fail-closed together:
        // if complete fails after the upsert, return Err so claim_write can
        // retry (upsert is idempotent on message_id). Source delivery uses
        // outbox-first with stable delivery id so partial retries are safe.
        if let Some(ref del) = delegation {
            teamwork
                .complete_delegation_for_thread(
                    conversation_id,
                    session_id,
                    Some(&message_id),
                    text,
                )
                .await
                .map_err(|error| {
                    anyhow::anyhow!("complete_delegation failed after result bubble: {error}")
                })?;
            // Whether complete just ran or was already done (Ok(None)), ensure
            // source delivery outbox exists and attempt send.
            self.deliver_to_source(&teamwork, del, session_id, &body)
                .await?;
        }

        info!(
            target: "minos_daemon::conversation_completion",
            conversation_id,
            session_id,
            message_id = %message_id,
            "recorded agent conversation result"
        );
        Ok(())
    }

    /// Outbox-first source delivery: durable row with stable id before provider send.
    async fn deliver_to_source(
        &self,
        teamwork: &TeamworkStore,
        delegation: &minos_chat_store::TeamworkDelegation,
        target_session_id: &str,
        visible_body: &str,
    ) -> anyhow::Result<()> {
        let Some(source_session_id) = delegation.source_session_id.as_deref() else {
            return Ok(());
        };
        if source_session_id == target_session_id {
            return Ok(());
        }
        let source_body = format!(
            "[{}#{}] {}",
            delegation.target_agent.bin_name(),
            short_session_id(target_session_id),
            visible_body
        );

        // Create/update delivery record BEFORE provider send (outbox pattern).
        let delivery = teamwork
            .enqueue_source_delivery(
                &delegation.conversation_id,
                &delegation.delegation_id,
                source_session_id,
                &source_body,
            )
            .await?;
        if delivery.status == TeamworkSourceDeliveryStatus::Delivered {
            debug!(
                target: "minos_daemon::conversation_completion",
                delivery_id = %delivery.delivery_id,
                "source delivery already delivered; skip resend"
            );
            return Ok(());
        }

        match self
            .try_send_to_source(source_session_id, &source_body)
            .await
        {
            Ok(()) => {
                teamwork
                    .mark_source_delivery(
                        &delivery.delivery_id,
                        TeamworkSourceDeliveryStatus::Delivered,
                        None,
                    )
                    .await?;
            }
            Err(error) if should_queue_delivery(&error) => {
                warn!(
                    target: "minos_daemon::conversation_completion",
                    error = %error,
                    source_session_id,
                    delivery_id = %delivery.delivery_id,
                    "source busy; delivery remains pending for flush"
                );
            }
            Err(error) => {
                warn!(
                    target: "minos_daemon::conversation_completion",
                    error = %error,
                    source_session_id,
                    delivery_id = %delivery.delivery_id,
                    "failed to deliver delegation result to source"
                );
                let _ = teamwork
                    .mark_source_delivery(
                        &delivery.delivery_id,
                        TeamworkSourceDeliveryStatus::Failed,
                        Some(&error.to_string()),
                    )
                    .await;
            }
        }
        Ok(())
    }

    async fn try_send_to_source(&self, source_session_id: &str, body: &str) -> anyhow::Result<()> {
        self.manager
            .send_user_message(source_session_id, body.to_owned())
            .await
    }

    async fn flush_pending_source_deliveries(&self, source_session_id: &str) {
        let Ok(teamwork) =
            open_teamwork_store(&self.store, "unused", &self.default_workspace).await
        else {
            return;
        };
        let Ok(pending) = teamwork
            .list_pending_source_deliveries_for_thread(source_session_id)
            .await
        else {
            return;
        };
        for delivery in pending {
            match self
                .try_send_to_source(source_session_id, &delivery.body)
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

fn short_session_id(session_id: &str) -> String {
    session_id[..8.min(session_id.len())].to_owned()
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection {
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
        p.apply_events(
            AgentName::Opencode,
            &[assistant_start("m1"), delta("m1", "done"), completed("m1")],
        );
        assert!(p.should_try_record_on_ingest(true, false));
    }

    #[test]
    fn origin_message_id_wins_over_fallback_message_key() {
        // Simulate race: fallback claimed first, then origin noted.
        let mut p = SessionProjection {
            turn_write_id: Some("m1".into()),
            ..Default::default()
        };
        p.set_origin_message_id("user-hub-msg-42");
        let durable = p.ensure_turn_write_id("m1").expect("origin");
        assert_eq!(durable, "user-hub-msg-42");
        // claim_write path uses same ensure.
        p.completed.push(("m1".into(), "final text".into()));
        p.turn_recorded = false;
        p.write_in_flight = false;
        let claimed = p.claim_write().expect("claim");
        assert_eq!(claimed.2, "user-hub-msg-42");
    }

    #[test]
    fn collab_without_origin_skips_non_canonical_write() {
        let mut p = SessionProjection::default();
        p.set_require_canonical_origin();
        p.completed.push(("assistant-key".into(), "final".into()));
        assert!(p.claim_write().is_none());
        assert!(p.ensure_turn_write_id("assistant-key").is_none());
    }

    #[test]
    fn local_non_collab_still_allows_message_key_fallback() {
        let mut p = SessionProjection::default();
        p.completed.push(("assistant-key".into(), "final".into()));
        let claimed = p.claim_write().expect("local claim");
        assert_eq!(claimed.2, "assistant-key");
    }

    #[test]
    fn begin_turn_clears_prior_completed_so_cancel_cannot_resurrect() {
        let mut p = SessionProjection::default();
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
    fn desktop_origin_survives_running_then_user_message_started_begin_turns() {
        // Repro of Host Desktop @agent turns: sendUserMessage notes origin, runtime
        // flips Running (begin_turn), then synth/user MessageStarted begin_turn
        // again. Second begin must not clear the pinned origin — otherwise
        // claim_write loses the durable Hub id even though the session finished.
        let mut p = SessionProjection::default();
        p.set_require_canonical_origin(); // Hub collab bind (separate from origin pin)
        p.set_origin_message_id("msg_user_origin_1");

        p.begin_turn(); // Running
        assert_eq!(p.origin_message_id.as_deref(), Some("msg_user_origin_1"));
        assert!(p.pending_origin_message_id.is_none());

        p.apply_events(AgentName::Grok, &[user_start("synth-user")]); // second begin
        assert_eq!(
            p.origin_message_id.as_deref(),
            Some("msg_user_origin_1"),
            "redundant same-turn begin_turn must keep Desktop origin"
        );
        assert!(p.require_canonical_origin);

        p.apply_events(
            AgentName::Grok,
            &[
                assistant_start("m_final"),
                delta("m_final", "你好！我是 Grok。"),
                completed("m_final"),
            ],
        );
        p.pending_boundary = true;
        let claimed = p
            .claim_write()
            .expect("Grok Desktop turn must claim agent-result after dual begin_turn");
        assert_eq!(claimed.1, "你好！我是 Grok。");
        assert_eq!(claimed.2, "msg_user_origin_1");
    }

    #[test]
    fn origin_noted_between_running_and_user_started_still_pins() {
        // note_turn_origin can race after Running begin_turn; pin must still
        // survive the subsequent user MessageStarted begin.
        let mut p = SessionProjection::default();
        p.begin_turn(); // Running before origin staged
        p.set_origin_message_id("msg_late_origin");
        p.apply_events(AgentName::Grok, &[user_start("u1")]);
        assert_eq!(p.origin_message_id.as_deref(), Some("msg_late_origin"));

        p.apply_events(
            AgentName::Grok,
            &[assistant_start("a1"), delta("a1", "ok"), completed("a1")],
        );
        let claimed = p.claim_write().expect("late origin still durable");
        assert_eq!(claimed.2, "msg_late_origin");
    }

    #[test]
    fn true_next_turn_without_new_origin_drops_previous_pin() {
        // After a completed turn, a later begin without a freshly staged origin
        // must not reuse the previous Desktop message id.
        let mut p = SessionProjection::default();
        p.set_require_canonical_origin();
        p.set_origin_message_id("msg_turn_a");
        p.begin_turn();
        p.apply_events(
            AgentName::Claude,
            &[assistant_start("m1"), delta("m1", "a"), completed("m1")],
        );
        let _ = p.claim_write();
        p.finish_write_ok();

        p.begin_turn(); // next turn, no new origin staged
        assert!(p.origin_message_id.is_none());
        assert!(p.turn_write_id.is_none());
        // Hub collab require is sticky; without origin, claim must skip fallback.
        p.apply_events(
            AgentName::Claude,
            &[assistant_start("m2"), delta("m2", "b"), completed("m2")],
        );
        assert!(p.claim_write().is_none());
    }

    #[test]
    fn local_origin_pin_does_not_force_canonical_on_later_turns() {
        // Origin alone is not collab shape — only note_require_canonical_origin is.
        let mut p = SessionProjection::default();
        p.set_origin_message_id("msg_local_a");
        assert!(!p.require_canonical_origin);
        p.begin_turn();
        p.apply_events(
            AgentName::Claude,
            &[assistant_start("m1"), delta("m1", "a"), completed("m1")],
        );
        let claimed = p.claim_write().expect("first turn with origin");
        assert_eq!(claimed.2, "msg_local_a");
        p.finish_write_ok();

        p.begin_turn(); // next local turn, no origin
        p.apply_events(
            AgentName::Claude,
            &[assistant_start("m2"), delta("m2", "b"), completed("m2")],
        );
        let claimed = p.claim_write().expect("local fallback still allowed");
        assert_eq!(claimed.2, "m2");
    }

    #[test]
    fn user_message_started_resets_turn_scope() {
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection {
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
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
        let mut p = SessionProjection::default();
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
