use crate::error::TranslationError;
use crate::message::UiEventMessage;

pub fn translate(_raw: &serde_json::Value) -> Result<Vec<UiEventMessage>, TranslationError> {
    Err(TranslationError::NotImplemented {
        agent: minos_domain::AgentName::Gemini,
    })
}
