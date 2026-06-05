use serde::{Deserialize, Serialize};

pub type SessionId = String;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        data: String,
        mime_type: String,
    },
    Audio {
        data: String,
        mime_type: String,
    },
    Resource {
        resource: ResourceContent,
    },
    ResourceLink {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        name: Option<String>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub text: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallKind {
    Edit,
    Diff,
    Terminal,
    Other,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallUpdate {
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    pub kind: ToolCallKind,
    pub status: ToolCallStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content: Option<Vec<ToolCallContent>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ToolCallContent {
    Content { content: ContentBlock },
    Terminal { terminal_id: String },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestPermissionOutcome {
    Allow,
    Deny,
    Cancelled,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(default)]
    pub fs: FsCapabilities,
    #[serde(default)]
    pub terminal: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct FsCapabilities {
    #[serde(default)]
    pub read_text_file: bool,
    #[serde(default)]
    pub write_text_file: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    #[serde(default)]
    pub load_session: bool,
    #[serde(default)]
    pub prompt_capabilities: PromptCapabilities,
    #[serde(default)]
    pub mcp_capabilities: McpCapabilities,
    #[serde(default)]
    pub auth: AgentAuthCapabilities,
    #[serde(default)]
    pub session_capabilities: SessionCapabilities,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    #[serde(default)]
    pub image: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub embedded_context: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpCapabilities {
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub sse: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentAuthCapabilities {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub logout: Option<LogoutCapabilities>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogoutCapabilities {}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionCapabilities {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub close: Option<CloseCapability>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub list: Option<ListCapability>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resume: Option<ResumeCapability>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub additional_directories: Option<AdditionalDirectoriesCapability>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CloseCapability {}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ListCapability {}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResumeCapability {}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AdditionalDirectoriesCapability {}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AuthMethod {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[serde(alias = "name")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    #[serde(flatten)]
    pub transport: McpTransport,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(
    tag = "transportType",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: std::collections::HashMap<String, String>,
    },
    Http {
        url: String,
    },
    Sse {
        url: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionModeState {
    pub current_mode_id: SessionModeId,
    pub available_modes: Vec<SessionMode>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionMode {
    pub id: SessionModeId,
    #[serde(alias = "name")]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

pub type SessionModeId = String;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigOption {
    pub id: SessionConfigId,
    #[serde(alias = "name")]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub values: Vec<SessionConfigValue>,
    pub current_value_id: SessionConfigValueId,
}

pub type SessionConfigId = String;
pub type SessionConfigValueId = String;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigValue {
    pub id: SessionConfigValueId,
    #[serde(alias = "name")]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn stop_reason_round_trips() {
        let reason = StopReason::EndTurn;
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(json, r#""end_turn""#);
        let back: StopReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, StopReason::EndTurn);
    }

    #[test]
    fn content_block_text_round_trips() {
        let block = ContentBlock::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains(r#""type":"text""#));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn tool_call_status_round_trips() {
        let status = ToolCallStatus::InProgress;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""in_progress""#);
        let back: ToolCallStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ToolCallStatus::InProgress);
    }

    #[test]
    fn client_capabilities_default() {
        let caps = ClientCapabilities::default();
        assert!(!caps.fs.read_text_file);
        assert!(!caps.fs.write_text_file);
        assert!(!caps.terminal);
    }

    #[test]
    fn permission_outcome_round_trips() {
        let outcome = RequestPermissionOutcome::Allow;
        let json = serde_json::to_string(&outcome).unwrap();
        assert_eq!(json, r#""allow""#);
        let back: RequestPermissionOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, RequestPermissionOutcome::Allow);
    }
}
