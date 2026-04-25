use thiserror::Error;

/// Errors emitted by translators. Lifted into `minos_domain::MinosError::
/// TranslationFailed` at the backend boundary (see `minos-backend`'s
/// ingest dispatch).
#[derive(Debug, Error)]
pub enum TranslationError {
    #[error("unsupported native event method: {method}")]
    UnsupportedMethod { method: String },

    #[error("malformed native event: {reason}")]
    Malformed { reason: String },

    #[error("translator not implemented for agent {agent:?}")]
    NotImplemented { agent: minos_domain::AgentName },
}
