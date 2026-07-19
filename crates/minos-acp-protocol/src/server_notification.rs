use crate::types::{
    ContentBlock, SessionId, SessionInfo, SessionMode, SessionModeId, ToolCallContent,
    ToolCallKind, ToolCallStatus,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionUpdateNotification {
    pub session_id: SessionId,
    pub update: SessionUpdate,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(
    tag = "sessionUpdate",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SessionUpdate {
    AgentMessageChunk {
        content: ContentBlock,
    },
    ToolCall {
        tool_call_id: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        title: Option<String>,
        kind: ToolCallKind,
        status: ToolCallStatus,
    },
    ToolCallUpdate {
        tool_call_id: String,
        status: ToolCallStatus,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        content: Option<Vec<ToolCallContent>>,
    },
    Plan {
        entries: Vec<PlanEntry>,
    },
    Thought {
        content: ContentBlock,
    },
    CurrentModeUpdate {
        current_mode_id: SessionModeId,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        available_modes: Option<Vec<SessionMode>>,
    },
    AvailableCommandsUpdate {
        #[serde(rename = "availableCommands", alias = "commands")]
        commands: Vec<SlashCommand>,
    },
    SessionInfoUpdate {
        info: SessionInfo,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub status: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn agent_message_chunk_deserializes() {
        let json =
            r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello"}}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        match update {
            SessionUpdate::AgentMessageChunk { content } => {
                assert_eq!(
                    content,
                    ContentBlock::Text {
                        text: "Hello".into()
                    }
                );
            }
            _ => panic!("expected agent_message_chunk"),
        }
    }

    #[test]
    fn tool_call_update_completed_deserializes() {
        let json = r#"{"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"completed","content":null}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        match update {
            SessionUpdate::ToolCallUpdate {
                tool_call_id,
                status,
                ..
            } => {
                assert_eq!(tool_call_id, "tc_1");
                assert_eq!(status, ToolCallStatus::Completed);
            }
            _ => panic!("expected tool_call_update"),
        }
    }

    #[test]
    fn plan_deserializes() {
        let json = r#"{"sessionUpdate":"plan","entries":[{"content":"Step 1","priority":"high","status":"pending"}]}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        match update {
            SessionUpdate::Plan { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].content, "Step 1");
            }
            _ => panic!("expected plan"),
        }
    }
}
