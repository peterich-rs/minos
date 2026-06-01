use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicKind {
    Account,
    Conversation,
    Project,
    AgentSession,
    Host,
}

impl TopicKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Conversation => "conversation",
            Self::Project => "project",
            Self::AgentSession => "agent_session",
            Self::Host => "host",
        }
    }

    pub fn parse(value: &str) -> Result<Self, TopicParseError> {
        match value {
            "account" => Ok(Self::Account),
            "conversation" => Ok(Self::Conversation),
            "project" => Ok(Self::Project),
            "agent_session" => Ok(Self::AgentSession),
            "host" => Ok(Self::Host),
            _ => Err(TopicParseError::UnknownKind),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RealtimeTopic {
    Account(String),
    Conversation(String),
    Project(String),
    AgentSession(String),
    Host(String),
}

impl RealtimeTopic {
    #[must_use]
    pub const fn kind(&self) -> TopicKind {
        match self {
            Self::Account(_) => TopicKind::Account,
            Self::Conversation(_) => TopicKind::Conversation,
            Self::Project(_) => TopicKind::Project,
            Self::AgentSession(_) => TopicKind::AgentSession,
            Self::Host(_) => TopicKind::Host,
        }
    }

    #[must_use]
    pub fn topic_string(&self) -> String {
        match self {
            Self::Account(id) => format!("account:{id}"),
            Self::Conversation(id) => format!("conversation:{id}"),
            Self::Project(id) => format!("project:{id}"),
            Self::AgentSession(id) => format!("agent_session:{id}"),
            Self::Host(id) => format!("host:{id}"),
        }
    }

    #[must_use]
    pub fn partition_key(&self) -> &str {
        match self {
            Self::Account(id)
            | Self::Conversation(id)
            | Self::Project(id)
            | Self::AgentSession(id)
            | Self::Host(id) => id,
        }
    }

    pub fn parse(value: &str) -> Result<Self, TopicParseError> {
        let (kind, partition_key) = value
            .split_once(':')
            .ok_or(TopicParseError::InvalidFormat)?;
        if partition_key.is_empty() {
            return Err(TopicParseError::MissingPartitionKey);
        }
        match TopicKind::parse(kind)? {
            TopicKind::Account => Ok(Self::Account(partition_key.to_string())),
            TopicKind::Conversation => Ok(Self::Conversation(partition_key.to_string())),
            TopicKind::Project => Ok(Self::Project(partition_key.to_string())),
            TopicKind::AgentSession => Ok(Self::AgentSession(partition_key.to_string())),
            TopicKind::Host => Ok(Self::Host(partition_key.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TopicParseError {
    #[error("unknown_topic_kind")]
    UnknownKind,
    #[error("invalid_topic_format")]
    InvalidFormat,
    #[error("missing_partition_key")]
    MissingPartitionKey,
}
