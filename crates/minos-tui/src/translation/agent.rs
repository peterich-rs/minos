use minos_domain::AgentName;
use minos_ui_protocol::{
    ClaudeTranslatorState, CodexTranslatorState, GeminiTranslatorState, OpencodeTranslatorState,
    UiEventMessage,
};
use tracing::warn;

pub enum AgentTranslationState {
    Codex(CodexTranslatorState),
    Claude(ClaudeTranslatorState),
    Gemini(GeminiTranslatorState),
    Opencode(OpencodeTranslatorState),
}

impl AgentTranslationState {
    pub fn new(agent: AgentName, thread_id: String) -> Self {
        match agent {
            AgentName::Codex => Self::Codex(CodexTranslatorState::new(thread_id)),
            AgentName::Claude => Self::Claude(ClaudeTranslatorState::new(thread_id)),
            AgentName::Gemini => Self::Gemini(GeminiTranslatorState::new(thread_id)),
            AgentName::Opencode => Self::Opencode(OpencodeTranslatorState::new(thread_id)),
        }
    }

    pub fn translate(&mut self, payload: &serde_json::Value) -> Vec<UiEventMessage> {
        match self {
            Self::Codex(s) => translate_with_log("codex", payload, || {
                minos_ui_protocol::translate_codex(s, payload)
            }),
            Self::Claude(s) => translate_with_log("claude", payload, || {
                minos_ui_protocol::translate_claude(s, payload)
            }),
            Self::Gemini(s) => translate_with_log("gemini", payload, || {
                minos_ui_protocol::translate_gemini(s, payload)
            }),
            Self::Opencode(s) => translate_with_log("opencode", payload, || {
                minos_ui_protocol::translate_opencode(s, payload)
            }),
        }
    }
}

fn translate_with_log<F>(agent: &str, payload: &serde_json::Value, f: F) -> Vec<UiEventMessage>
where
    F: FnOnce() -> Result<Vec<UiEventMessage>, minos_ui_protocol::TranslationError>,
{
    match f() {
        Ok(events) => events,
        Err(error) => {
            warn!(
                target: "minos_tui::translation",
                agent,
                error = %error,
                payload = %payload,
                "ui translation failed"
            );
            Vec::new()
        }
    }
}
