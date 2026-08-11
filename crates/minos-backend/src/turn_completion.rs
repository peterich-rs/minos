//! # TurnCompletionProjector
//!
//! Hub-side **single writer** for multi-end agent chat bubbles.
//!
//! After Host ingest lands raw agent events, the social group-completion
//! watcher drives this projector to insert one final `conversation_messages`
//! row (role=agent) with a stable idempotent `client_message_id`.
//!
//! Semantics align with `minos_daemon::conversation_completion` (local timeline):
//! - Tool / subagent interrupts mark prior completed segments as progress-only.
//! - Only the last clean (non-interrupted) assistant text is posted to IM.
//! - Progress-only turns produce no chat bubble.
//!
//! All runtimes (Codex / Claude / Gemini / OpenCode / Grok) share one projection
//! path via `minos-ui-protocol` translators — Desktop UI dual-write is **not**
//! a multi-end agent bubble writer.

use std::collections::{HashMap, HashSet};

use minos_domain::AgentName;
use minos_ui_protocol::{
    translate_claude, translate_codex, translate_gemini, translate_grok, translate_opencode,
    ClaudeTranslatorState, CodexTranslatorState, GeminiTranslatorState, GrokTranslatorState,
    MessageRole, OpencodeTranslatorState, UiEventMessage,
};
use serde_json::Value;

/// Outcome of probing a session's raw events for a postable final reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionProbe {
    /// Turn still running or final text not yet stable.
    Pending,
    /// Clean final assistant text ready to insert as an agent social message.
    Ready(String),
    /// Turn finished (or abandoned) with no clean final text — stop watching.
    DoneWithoutText,
}

/// Formal projector entry surface (stateless helpers + probe).
///
/// Callers (group completion watcher) own poll/backoff; this type documents the
/// single-writer contract and stable idempotency key shape.
#[derive(Debug, Default, Clone, Copy)]
pub struct TurnCompletionProjector;

impl TurnCompletionProjector {
    /// Stable Hub `client_message_id` for one dispatched turn.
    ///
    /// Frozen formula (IM reliability program):
    /// `agent-result:{conversation_id}:{session_id}:{origin_message_id}`
    /// where `origin_message_id` is the user Hub message that triggered the turn.
    #[must_use]
    pub fn agent_result_client_message_id(
        conversation_id: &str,
        session_id: &str,
        origin_message_id: impl std::fmt::Display,
    ) -> String {
        format!("agent-result:{conversation_id}:{session_id}:{origin_message_id}")
    }

    /// Probe raw event rows for a clean final assistant segment.
    pub fn probe(
        agent: AgentName,
        session_id: &str,
        rows: &[(u64, &Value)],
        trigger_seq: u64,
        session_terminal: bool,
        seq_stable: bool,
    ) -> Result<CompletionProbe, String> {
        probe_completion_from_raw_payloads(
            agent,
            session_id,
            rows,
            trigger_seq,
            session_terminal,
            seq_stable,
        )
    }
}

#[derive(Default)]
struct TurnProjection {
    assistant_text: HashMap<String, String>,
    assistant_roles: HashMap<String, MessageRole>,
    segment_closed: HashMap<String, bool>,
    tool_message_ids: HashMap<String, String>,
    /// tool_call_ids still in flight (placed, not completed).
    open_tools: HashSet<String>,
    completed: Vec<(String, String)>,
    interrupted_keys: HashSet<String>,
    last_error: Option<(String, String)>,
    saw_message_completed_after_trigger: bool,
    saw_session_closed: bool,
    /// Highest raw event seq that produced a UiEvent after `trigger_seq`.
    last_post_trigger_seq: u64,
}

impl TurnProjection {
    fn close_text_segment(&mut self, message_id: &str) {
        if message_id.is_empty() {
            return;
        }
        self.segment_closed.insert(message_id.to_owned(), true);
    }

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

    fn apply_event(&mut self, event: &UiEventMessage, event_seq: u64, trigger_seq: u64) {
        if event_seq > trigger_seq {
            self.last_post_trigger_seq = self.last_post_trigger_seq.max(event_seq);
        }

        match event {
            UiEventMessage::MessageStarted {
                message_id, role, ..
            } => {
                if matches!(role, MessageRole::User) {
                    // New user turn resets segment bookkeeping within the probe.
                    *self = Self {
                        last_post_trigger_seq: self.last_post_trigger_seq,
                        saw_session_closed: self.saw_session_closed,
                        ..Self::default()
                    };
                }
                self.assistant_roles.insert(message_id.clone(), *role);
                if matches!(role, MessageRole::Assistant) {
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
                        return;
                    }
                    let replace = matches!(event, UiEventMessage::TextReplace { .. });
                    self.append_assistant_text(message_id, rendered, replace);
                }
            }
            UiEventMessage::ToolCallPlaced {
                message_id,
                tool_call_id,
                ..
            } => {
                self.tool_message_ids
                    .insert(tool_call_id.clone(), message_id.clone());
                self.open_tools.insert(tool_call_id.clone());
                self.interrupt_prior_completed();
                self.close_text_segment(message_id);
            }
            UiEventMessage::ToolCallCompleted { tool_call_id, .. } => {
                self.open_tools.remove(tool_call_id);
                if let Some(message_id) = self.tool_message_ids.remove(tool_call_id) {
                    self.close_text_segment(&message_id);
                } else {
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
            UiEventMessage::SubagentSpawned { tool_call_id, .. } => {
                if let Some(message_id) = self.tool_message_ids.get(tool_call_id).cloned() {
                    self.close_text_segment(&message_id);
                    self.interrupt_prior_completed();
                }
            }
            UiEventMessage::MessageCompleted { message_id, .. } => {
                if event_seq > trigger_seq {
                    self.saw_message_completed_after_trigger = true;
                }
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
            UiEventMessage::SessionClosed { .. } => {
                self.saw_session_closed = true;
            }
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

    fn last_completed(&self) -> Option<(String, String)> {
        self.completed
            .iter()
            .rev()
            .find(|(key, _)| !self.interrupted_keys.contains(key))
            .cloned()
            .or_else(|| self.last_error.clone())
    }
}

/// Translate a stream of raw event payloads for `agent` into UI events and
/// project the last non-interrupted assistant segment.
pub fn probe_completion_from_raw_payloads(
    agent: AgentName,
    session_id: &str,
    rows: &[(u64, &Value)],
    trigger_seq: u64,
    session_terminal: bool,
    seq_stable: bool,
) -> Result<CompletionProbe, String> {
    let mut projection = TurnProjection::default();
    translate_and_apply(agent, session_id, rows, trigger_seq, &mut projection)?;

    let last = projection.last_completed();
    // Do not treat intermediate MessageCompleted as final while tools are still
    // open (Grok often completes progress text right before ToolCallPlaced).
    let tools_idle = projection.open_tools.is_empty();
    // Turn boundary (aligned with daemon conversation_completion latch):
    // - formal session terminal (idle/ended/…) or SessionClosed UI, OR
    // - quiet raw seq + tools idle + post-trigger MessageCompleted.
    // Ready may fire as soon as clean text exists at a boundary.
    // DoneWithoutText must wait for seq_stable when only formal terminal is set,
    // so Idle cannot race late ingest of the final MessageCompleted.
    let turn_ended = session_terminal || projection.saw_session_closed;
    let quiet_completed =
        seq_stable && tools_idle && projection.saw_message_completed_after_trigger;
    let boundary = turn_ended || quiet_completed;

    if let Some((_, text)) = last {
        if boundary && tools_idle {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(CompletionProbe::DoneWithoutText);
            }
            return Ok(CompletionProbe::Ready(trimmed.to_string()));
        }
        return Ok(CompletionProbe::Pending);
    }

    // No clean final text: only abandon after a stable quiet window, or when
    // the UI already closed the session (stream end is authoritative).
    if projection.saw_session_closed || (turn_ended && seq_stable) {
        return Ok(CompletionProbe::DoneWithoutText);
    }
    Ok(CompletionProbe::Pending)
}

fn translate_and_apply(
    agent: AgentName,
    session_id: &str,
    rows: &[(u64, &Value)],
    trigger_seq: u64,
    projection: &mut TurnProjection,
) -> Result<(), String> {
    match agent {
        AgentName::Codex => {
            let mut state = CodexTranslatorState::new(session_id.to_string());
            for &(seq, payload) in rows {
                let events = translate_codex(&mut state, payload).map_err(|e| e.to_string())?;
                for event in &events {
                    projection.apply_event(event, seq, trigger_seq);
                }
            }
        }
        AgentName::Claude => {
            let mut state = ClaudeTranslatorState::new(session_id.to_string());
            for &(seq, payload) in rows {
                let events = translate_claude(&mut state, payload).map_err(|e| e.to_string())?;
                for event in &events {
                    projection.apply_event(event, seq, trigger_seq);
                }
            }
        }
        AgentName::Gemini => {
            let mut state = GeminiTranslatorState::new(session_id.to_string());
            for &(seq, payload) in rows {
                let events = translate_gemini(&mut state, payload).map_err(|e| e.to_string())?;
                for event in &events {
                    projection.apply_event(event, seq, trigger_seq);
                }
            }
        }
        AgentName::Grok => {
            let mut state = GrokTranslatorState::new(session_id.to_string());
            for &(seq, payload) in rows {
                let events = translate_grok(&mut state, payload).map_err(|e| e.to_string())?;
                for event in &events {
                    projection.apply_event(event, seq, trigger_seq);
                }
            }
        }
        AgentName::Opencode => {
            let mut state = OpencodeTranslatorState::new(session_id.to_string());
            for &(seq, payload) in rows {
                let events = translate_opencode(&mut state, payload).map_err(|e| e.to_string())?;
                for event in &events {
                    projection.apply_event(event, seq, trigger_seq);
                }
            }
        }
    }
    Ok(())
}

/// Formal agent_session statuses that mean the turn will not produce more text.
pub fn is_session_terminal_status(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "ended" | "stopped" | "failed" | "completed" | "cancelled" | "canceled" | "closed" | "idle"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_ui_protocol::{DisplayPayload, MessageRole, UiEventMessage};

    fn apply_ui(events: &[UiEventMessage], trigger_seq: u64) -> TurnProjection {
        let mut p = TurnProjection::default();
        for (i, event) in events.iter().enumerate() {
            p.apply_event(event, (i as u64) + 1, trigger_seq);
        }
        p
    }

    fn assistant_start(id: &str) -> UiEventMessage {
        UiEventMessage::MessageStarted {
            message_id: id.into(),
            role: MessageRole::Assistant,
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
            finished_at_ms: 0,
        }
    }

    fn tool_placed(mid: &str, tc: &str) -> UiEventMessage {
        UiEventMessage::ToolCallPlaced {
            message_id: mid.into(),
            tool_call_id: tc.into(),
            name: "rg".into(),
            args_json: DisplayPayload::inline("{}"),
        }
    }

    fn tool_completed(tc: &str) -> UiEventMessage {
        UiEventMessage::ToolCallCompleted {
            tool_call_id: tc.into(),
            output: DisplayPayload::inline("ok"),
            is_error: false,
        }
    }

    #[test]
    fn grok_style_progress_then_tools_then_final_keeps_only_last_segment() {
        let events = [
            assistant_start("m1"),
            delta("m1", "正在定位…"),
            tool_placed("m1", "tc1"),
            tool_completed("tc1"),
            delta("m1", "发现 bug。"),
            tool_placed("m1", "tc2"),
            tool_completed("tc2"),
            delta("m1", "已完成修复。"),
            completed("m1"),
        ];
        let p = apply_ui(&events, 0);
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("已完成修复。".into())
        );
    }

    #[test]
    fn progress_only_before_tools_is_not_final() {
        let events = [
            assistant_start("m1"),
            delta("m1", "正在核对…"),
            tool_placed("m1", "tc1"),
            tool_completed("tc1"),
            completed("m1"),
        ];
        let p = apply_ui(&events, 0);
        assert!(p.last_completed().is_none());
    }

    #[test]
    fn continuous_text_concatenates() {
        let events = [
            assistant_start("m1"),
            delta("m1", "Hello "),
            delta("m1", "world"),
            completed("m1"),
        ];
        let p = apply_ui(&events, 0);
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("Hello world".into())
        );
    }

    #[test]
    fn probe_ready_when_stable_and_completed() {
        let events = [
            assistant_start("m1"),
            delta("m1", "final answer"),
            completed("m1"),
        ];
        let p = apply_ui(&events, 0);
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("final answer".into())
        );
        assert!(p.saw_message_completed_after_trigger);
    }

    #[test]
    fn terminal_without_text_is_done() {
        let probe =
            probe_completion_from_raw_payloads(AgentName::Codex, "sess", &[], 0, true, true)
                .unwrap();
        assert_eq!(probe, CompletionProbe::DoneWithoutText);
    }

    #[test]
    fn pending_while_running_without_stable() {
        let mut p = TurnProjection::default();
        p.apply_event(&assistant_start("m1"), 1, 0);
        p.apply_event(&delta("m1", "partial"), 2, 0);
        assert!(p.last_completed().is_none());
        assert!(!p.saw_message_completed_after_trigger);
    }

    #[test]
    fn projector_idempotency_key_is_stable() {
        let a = TurnCompletionProjector::agent_result_client_message_id("c1", "s1", "origin-msg");
        let b = TurnCompletionProjector::agent_result_client_message_id("c1", "s1", "origin-msg");
        assert_eq!(a, "agent-result:c1:s1:origin-msg");
        assert_eq!(a, b);
    }

    #[test]
    fn multi_segment_progress_then_final_probe_ready_when_stable() {
        // Grok-style: tool boundaries interrupt progress text; post-tool text uses a
        // fresh message_id (translator behavior). Only the final segment is Ready.
        let events = [
            assistant_start("m1"),
            delta("m1", "progress…"),
            tool_placed("m1", "tc1"),
            tool_completed("tc1"),
            assistant_start("m2"),
            delta("m2", "final clean answer"),
            completed("m2"),
        ];
        let p = apply_ui(&events, 0);
        assert_eq!(
            p.last_completed().map(|(_, t)| t),
            Some("final clean answer".into())
        );
        assert!(p.open_tools.is_empty());
        assert!(p.saw_message_completed_after_trigger);
        assert_eq!(
            CompletionProbe::Ready(p.last_completed().unwrap().1),
            CompletionProbe::Ready("final clean answer".into())
        );
    }

    #[test]
    fn intermediate_message_completed_before_tool_is_not_last_segment() {
        let events = [
            assistant_start("m1"),
            delta("m1", "即将调用工具"),
            completed("m1"),
            tool_placed("m1", "tc1"),
            // tools still open — must not treat interrupted segment as final
        ];
        let p = apply_ui(&events, 0);
        // Interrupted completed key + open tools → no clean final.
        assert!(p.last_completed().is_none());
        assert!(!p.open_tools.is_empty());
    }

    #[test]
    fn idle_without_seq_stable_stays_pending_when_no_text() {
        // Idle can arrive before final MessageCompleted is ingested — do not
        // DoneWithoutText until the raw seq quiet window elapses.
        let probe =
            probe_completion_from_raw_payloads(AgentName::Codex, "sess", &[], 0, true, false)
                .unwrap();
        assert_eq!(probe, CompletionProbe::Pending);
    }

    #[test]
    fn idle_with_seq_stable_and_no_text_is_done() {
        let probe =
            probe_completion_from_raw_payloads(AgentName::Codex, "sess", &[], 0, true, true)
                .unwrap();
        assert_eq!(probe, CompletionProbe::DoneWithoutText);
    }
}
