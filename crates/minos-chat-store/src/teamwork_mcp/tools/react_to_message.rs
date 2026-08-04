use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{bound_conversation_id, required_string_arg, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct ReactToMessageTool;

impl TeamworkMcpTool for ReactToMessageTool {
    fn name(&self) -> &'static str {
        "react_to_message"
    }

    fn description(&self) -> &'static str {
        "Add or toggle a lightweight emoji reaction on a conversation message that \
         @mentioned you. Use for brief acknowledgements (👍, ✅, 👀) instead of a full reply. \
         Hard-limited to messages that mention this agent."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::ReactToMessage
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "message_id".into(),
            json!({
                "type": "string",
                "description": "Target conversation message id (must @mention this agent)."
            }),
        );
        properties.insert(
            "emoji".into(),
            json!({
                "type": "string",
                "description": "Emoji reaction (1..=32 chars), e.g. 👍 ✅ 👀 ❤️."
            }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["message_id", "emoji"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let conversation_id = bound_conversation_id(&args, &ctx, self.name())?;
        let message_id = required_string_arg(&args, "message_id")?.trim().to_owned();
        let emoji = required_string_arg(&args, "emoji")?.trim().to_owned();
        anyhow::ensure!(!message_id.is_empty(), "message_id must not be empty");
        anyhow::ensure!(!emoji.is_empty(), "emoji must not be empty");
        anyhow::ensure!(
            emoji.chars().count() <= 32,
            "emoji must be at most 32 characters"
        );
        Ok(SocketRequest::ReactToMessage {
            conversation_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            source_session_id: ctx.source_session_id,
            message_id,
            emoji,
        })
    }
}

#[cfg(test)]
mod tests {
    use minos_domain::AgentName;
    use serde_json::json;

    use super::*;

    #[test]
    fn react_request_includes_bound_source() {
        let request = ReactToMessageTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "conversation-main".into(),
                    source_agent: Some(AgentName::Codex),
                    source_session_id: Some("thread-codex-1234".into()),
                },
                json!({"message_id": "msg-1", "emoji": "👍"}),
            )
            .expect("valid react request");

        match request {
            SocketRequest::ReactToMessage {
                conversation_id,
                source_agent,
                source_session_id,
                message_id,
                emoji,
            } => {
                assert_eq!(conversation_id, "conversation-main");
                assert_eq!(source_agent.as_deref(), Some("codex"));
                assert_eq!(source_session_id.as_deref(), Some("thread-codex-1234"));
                assert_eq!(message_id, "msg-1");
                assert_eq!(emoji, "👍");
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }
}
