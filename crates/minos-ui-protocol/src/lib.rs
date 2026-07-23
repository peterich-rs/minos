//! Minos unified UI event protocol.
//!
//! `UiEventMessage` is the single shape the mobile viewer and any future
//! admin surface consume to render agent activity. `translate_codex` /
//! `translate_claude` / `translate_gemini` map each CLI's native event
//! format onto this shape; the backend runs them on ingest and on
//! history read.
//!
//! The Rust types in this crate are the authoritative definition of the
//! UI event contract.

#![forbid(unsafe_code)]

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

mod ansi;
mod claude;
mod codex;
mod error;
mod gemini;
mod grok;
mod message;
mod opencode;

pub use ansi::strip_ansi_escapes;
pub use error::TranslationError;
pub use message::{
    ArtifactRef, DisplayPayload, MessageRole, SubagentStatus, SessionEndReason, UiEventMessage,
};
pub use minos_domain::AgentName as AgentKind;

pub use claude::{translate as translate_claude, ClaudeTranslatorState};
pub use codex::{translate as translate_codex, CodexTranslatorState};
pub use gemini::{translate as translate_gemini, GeminiTranslatorState};
pub use grok::{translate as translate_grok, GrokTranslatorState};
pub use opencode::{translate as translate_opencode, OpencodeTranslatorState};

/// One-shot dispatch convenience for the backend: given an agent kind
/// and one raw native event, return all resulting UI events. Used when
/// the caller does not carry per-session translator state across calls
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
        AgentKind::Claude => {
            let mut s = ClaudeTranslatorState::new(String::new());
            translate_claude(&mut s, raw_payload)
        }
        AgentKind::Gemini => {
            let mut s = GeminiTranslatorState::new(String::new());
            translate_gemini(&mut s, raw_payload)
        }
        AgentKind::Grok => {
            let mut s = GrokTranslatorState::new(String::new());
            translate_grok(&mut s, raw_payload)
        }
        AgentKind::Opencode => {
            let mut s = OpencodeTranslatorState::new(String::new());
            translate_opencode(&mut s, raw_payload)
        }
    }
}
