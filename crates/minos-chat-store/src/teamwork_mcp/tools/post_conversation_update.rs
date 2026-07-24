use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{bound_conversation_id, required_string_arg, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct PostConversationUpdateTool;

impl TeamworkMcpTool for PostConversationUpdateTool {
    fn name(&self) -> &'static str {
        "post_conversation_update"
    }

    fn description(&self) -> &'static str {
        "Post a concise user-visible update into this Minos conversation."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::PostConversationUpdate
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "message".into(),
            json!({
                "type": "string",
                "description": "A concise user-visible update to post in the conversation."
            }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["message"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let conversation_id = bound_conversation_id(&args, &ctx, self.name())?;
        let message = required_string_arg(&args, "message")?.trim().to_owned();
        anyhow::ensure!(!message.is_empty(), "message must not be empty");
        Ok(SocketRequest::PostConversationUpdate {
            conversation_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            source_session_id: ctx.source_session_id,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use minos_domain::AgentName;
    use serde_json::json;

    use super::*;

    #[test]
    fn post_update_request_includes_bound_source_session_id() {
        let request = PostConversationUpdateTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "conversation-main".into(),
                    source_agent: Some(AgentName::Codex),
                    source_session_id: Some("thread-codex-1234".into()),
                },
                json!({"message": "done"}),
            )
            .expect("valid post update request");

        match request {
            SocketRequest::PostConversationUpdate {
                conversation_id,
                source_agent,
                source_session_id,
                message,
            } => {
                assert_eq!(conversation_id, "conversation-main");
                assert_eq!(source_agent.as_deref(), Some("codex"));
                assert_eq!(source_session_id.as_deref(), Some("thread-codex-1234"));
                assert_eq!(message, "done");
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }
}
