use crate::error::TranslationError;
use crate::message::UiEventMessage;

pub struct CodexTranslatorState {
    _thread_id: String,
}

impl CodexTranslatorState {
    pub fn new(thread_id: String) -> Self {
        Self {
            _thread_id: thread_id,
        }
    }
}

pub fn translate(
    _state: &mut CodexTranslatorState,
    _raw: &serde_json::Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    Err(TranslationError::NotImplemented {
        agent: minos_domain::AgentName::Codex,
    })
}
