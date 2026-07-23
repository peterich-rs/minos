//! Backend-side wrapper around `minos-ui-protocol`.
//!
//! Owns a per-session `CodexTranslatorState` for the live-ingest path. Each
//! `ingest::dispatch` call looks up (or creates) the per-session state and
//! feeds the raw payload through it. `drop_thread` evicts the state when a
//! thread ends — the history read path in C2 will reconstruct a fresh
//! state per call so replay is deterministic.

use std::sync::Arc;

use dashmap::DashMap;
use minos_domain::AgentName;
use minos_ui_protocol::{
    translate_claude, translate_codex, translate_gemini, translate_grok, translate_opencode,
    ClaudeTranslatorState, CodexTranslatorState, GeminiTranslatorState, GrokTranslatorState,
    OpencodeTranslatorState, TranslationError, UiEventMessage,
};
use serde_json::Value;

/// Per-thread translator-state store. Wrap in `Arc` so the HTTP `BackendState`
/// can hand a clone to every dispatched ingest call without locking.
pub struct SessionTranslators {
    codex: DashMap<String, CodexTranslatorState>,
    claude: DashMap<String, ClaudeTranslatorState>,
    opencode: DashMap<String, OpencodeTranslatorState>,
    gemini: DashMap<String, GeminiTranslatorState>,
    grok: DashMap<String, GrokTranslatorState>,
}

impl SessionTranslators {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            codex: DashMap::new(),
            claude: DashMap::new(),
            opencode: DashMap::new(),
            gemini: DashMap::new(),
            grok: DashMap::new(),
        })
    }

    /// Translate one raw event for `agent` within `session_id`, using (and
    /// mutating) the cached translator state.
    pub fn translate(
        &self,
        agent: AgentName,
        session_id: &str,
        payload: &Value,
    ) -> Result<Vec<UiEventMessage>, TranslationError> {
        match agent {
            AgentName::Codex => {
                let mut state = self
                    .codex
                    .entry(session_id.to_string())
                    .or_insert_with(|| CodexTranslatorState::new(session_id.to_string()));
                translate_codex(&mut state, payload)
            }
            AgentName::Claude => {
                let mut state = self
                    .claude
                    .entry(session_id.to_string())
                    .or_insert_with(|| ClaudeTranslatorState::new(session_id.to_string()));
                translate_claude(&mut state, payload)
            }
            AgentName::Gemini => {
                let mut state = self
                    .gemini
                    .entry(session_id.to_string())
                    .or_insert_with(|| GeminiTranslatorState::new(session_id.to_string()));
                translate_gemini(&mut state, payload)
            }
            AgentName::Grok => {
                let mut state = self
                    .grok
                    .entry(session_id.to_string())
                    .or_insert_with(|| GrokTranslatorState::new(session_id.to_string()));
                translate_grok(&mut state, payload)
            }
            AgentName::Opencode => {
                let mut state = self
                    .opencode
                    .entry(session_id.to_string())
                    .or_insert_with(|| OpencodeTranslatorState::new(session_id.to_string()));
                translate_opencode(&mut state, payload)
            }
        }
    }

    /// Drop the translator state for `session_id`. Call on `SessionClosed`.
    pub fn drop_thread(&self, session_id: &str) {
        self.codex.remove(session_id);
        self.claude.remove(session_id);
        self.opencode.remove(session_id);
        self.gemini.remove(session_id);
        self.grok.remove(session_id);
    }
}
