use crate::types::{
    ContentBlock, SessionId, SessionInfo, SessionMode, SessionModeId, ToolCall, ToolCallContent,
    ToolCallKind, ToolCallLocation, ToolCallStatus, ToolCallUpdate,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionUpdateNotification {
    pub session_id: SessionId,
    pub update: SessionUpdate,
    /// xAI / ACP extension metadata (`streamStartMs`, `eventId`, …).
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none", default)]
    pub meta: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(
    tag = "sessionUpdate",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SessionUpdate {
    UserMessageChunk {
        content: ContentBlock,
    },
    AgentMessageChunk {
        content: ContentBlock,
    },
    /// Wire name `agent_thought_chunk` (preferred) — also accept legacy `thought`.
    #[serde(alias = "thought")]
    AgentThoughtChunk {
        content: ContentBlock,
    },
    /// Flattened tool_call fields + optional rich ACP fields.
    ToolCall {
        tool_call_id: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        title: Option<String>,
        #[serde(default)]
        kind: ToolCallKind,
        #[serde(default)]
        status: ToolCallStatus,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        content: Option<Vec<ToolCallContent>>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        locations: Option<Vec<ToolCallLocation>>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        raw_input: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        raw_output: Option<Value>,
        #[serde(rename = "_meta", skip_serializing_if = "Option::is_none", default)]
        meta: Option<Value>,
    },
    ToolCallUpdate {
        tool_call_id: String,
        #[serde(default)]
        status: ToolCallStatus,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        content: Option<Vec<ToolCallContent>>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        kind: Option<ToolCallKind>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        raw_input: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        raw_output: Option<Value>,
        #[serde(rename = "_meta", skip_serializing_if = "Option::is_none", default)]
        meta: Option<Value>,
    },
    Plan {
        entries: Vec<PlanEntry>,
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

impl SessionUpdate {
    /// Convert a rich [`ToolCall`] into the wire `ToolCall` variant.
    #[must_use]
    pub fn from_tool_call(tc: ToolCall) -> Self {
        Self::ToolCall {
            tool_call_id: tc.tool_call_id,
            title: tc.title,
            kind: tc.kind,
            status: tc.status,
            content: tc.content,
            locations: tc.locations,
            raw_input: tc.raw_input,
            raw_output: tc.raw_output,
            meta: tc.meta,
        }
    }

    /// Convert a rich [`ToolCallUpdate`] into the wire update variant.
    #[must_use]
    pub fn from_tool_call_update(u: ToolCallUpdate) -> Self {
        Self::ToolCallUpdate {
            tool_call_id: u.tool_call_id,
            status: u.status,
            content: u.content,
            title: u.title,
            kind: u.kind,
            raw_input: u.raw_input,
            raw_output: u.raw_output,
            meta: u.meta,
        }
    }
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

    #[test]
    fn agent_thought_chunk_and_legacy_thought_alias() {
        let modern =
            r#"{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"hmm"}}"#;
        match serde_json::from_str::<SessionUpdate>(modern).unwrap() {
            SessionUpdate::AgentThoughtChunk { content } => {
                assert_eq!(content, ContentBlock::Text { text: "hmm".into() });
            }
            _ => panic!("expected agent_thought_chunk"),
        }
        let legacy = r#"{"sessionUpdate":"thought","content":{"type":"text","text":"legacy"}}"#;
        match serde_json::from_str::<SessionUpdate>(legacy).unwrap() {
            SessionUpdate::AgentThoughtChunk { content } => {
                assert_eq!(
                    content,
                    ContentBlock::Text {
                        text: "legacy".into()
                    }
                );
            }
            _ => panic!("expected thought alias → AgentThoughtChunk"),
        }
    }

    #[test]
    fn tool_call_with_raw_input_and_locations() {
        let json = r#"{
            "sessionUpdate":"tool_call",
            "toolCallId":"tc1",
            "title":"bash",
            "kind":"execute",
            "status":"pending",
            "rawInput":{"description":"Run tests","command":"cargo test"},
            "locations":[{"path":"src/main.rs","line":10}]
        }"#;
        match serde_json::from_str::<SessionUpdate>(json).unwrap() {
            SessionUpdate::ToolCall {
                tool_call_id,
                kind,
                raw_input,
                locations,
                ..
            } => {
                assert_eq!(tool_call_id, "tc1");
                assert_eq!(kind, ToolCallKind::Execute);
                assert_eq!(
                    raw_input
                        .as_ref()
                        .and_then(|v| v.get("description"))
                        .and_then(|v| v.as_str()),
                    Some("Run tests")
                );
                assert_eq!(
                    locations
                        .as_ref()
                        .and_then(|l| l.first())
                        .and_then(|l| l.path.as_deref()),
                    Some("src/main.rs")
                );
            }
            _ => panic!("expected tool_call"),
        }
    }

    #[test]
    fn session_notification_meta_round_trip() {
        let json = r#"{
            "sessionId":"s1",
            "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}},
            "_meta":{"streamStartMs":100,"totalTokens":42}
        }"#;
        let n: SessionUpdateNotification = serde_json::from_str(json).unwrap();
        assert_eq!(n.session_id, "s1");
        assert_eq!(
            n.meta
                .as_ref()
                .and_then(|m| m.get("streamStartMs"))
                .and_then(|v| v.as_i64()),
            Some(100)
        );
    }
}
