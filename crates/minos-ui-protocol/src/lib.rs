//! Minos unified UI event protocol.
//!
//! `UiEventMessage` is the single shape the mobile viewer and any future
//! admin surface consume to render agent activity. `translate_codex` /
//! `translate_claude` / `translate_gemini` map each CLI's native event
//! format onto this shape; the backend runs them on ingest and on
//! history read.
//!
//! See `docs/superpowers/specs/mobile-migration-and-ui-protocol-design.md`
//! §6.4 for the authoritative type definition.

#![forbid(unsafe_code)]

mod claude;
mod codex;
mod error;
mod gemini;
mod message;

pub use error::TranslationError;
pub use message::{MessageRole, ThreadEndReason, UiEventMessage};
pub use minos_domain::AgentName as AgentKind;

pub use claude::translate as translate_claude;
pub use codex::{translate as translate_codex, CodexTranslatorState};
pub use gemini::translate as translate_gemini;

/// One-shot dispatch convenience for the backend: given an agent kind
/// and one raw native event, return all resulting UI events. Used when
/// the caller does not carry per-thread translator state across calls
/// (e.g., a one-off history replay).
///
/// **Beware:** for codex, the translator is stateful across a thread
/// (tool-call argument buffering, open-message tracking). Use
/// [`CodexTranslatorState`] for live streams, not this function.
pub fn translate_stateless(
    agent: AgentKind,
    raw_payload: &serde_json::Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    match agent {
        AgentKind::Codex => {
            let mut s = CodexTranslatorState::new(String::new());
            translate_codex(&mut s, raw_payload)
        }
        AgentKind::Claude => translate_claude(raw_payload),
        AgentKind::Gemini => translate_gemini(raw_payload),
    }
}
