//! Backend-side wrapper around `minos-ui-protocol`.
//!
//! Owns a per-thread `CodexTranslatorState` for the live-ingest path. Each
//! `ingest::dispatch` call looks up (or creates) the per-thread state and
//! feeds the raw payload through it. `drop_thread` evicts the state when a
//! thread ends — the history read path in C2 will reconstruct a fresh
//! state per call so replay is deterministic.

use std::sync::Arc;

use dashmap::DashMap;
use minos_domain::AgentName;
use minos_ui_protocol::{
    translate_claude, translate_codex, translate_gemini, CodexTranslatorState, TranslationError,
    UiEventMessage,
};
use serde_json::Value;

/// Per-thread translator-state store. Wrap in `Arc` so the HTTP `RelayState`
/// can hand a clone to every dispatched ingest call without locking.
pub struct ThreadTranslators {
    codex: DashMap<String, CodexTranslatorState>,
}

impl ThreadTranslators {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            codex: DashMap::new(),
        })
    }

    /// Translate one raw event for `agent` within `thread_id`, using (and
    /// mutating) the cached translator state. Unknown agents fall through
    /// to `translate_claude` / `translate_gemini` stubs, which return
    /// `TranslationError::NotImplemented` until those CLIs land.
    pub fn translate(
        &self,
        agent: AgentName,
        thread_id: &str,
        payload: &Value,
    ) -> Result<Vec<UiEventMessage>, TranslationError> {
        match agent {
            AgentName::Codex => {
                let mut state = self
                    .codex
                    .entry(thread_id.to_string())
                    .or_insert_with(|| CodexTranslatorState::new(thread_id.to_string()));
                translate_codex(&mut state, payload)
            }
            AgentName::Claude => translate_claude(payload),
            AgentName::Gemini => translate_gemini(payload),
        }
    }

    /// Drop the translator state for `thread_id`. Call on `ThreadClosed`.
    pub fn drop_thread(&self, thread_id: &str) {
        self.codex.remove(thread_id);
    }
}
