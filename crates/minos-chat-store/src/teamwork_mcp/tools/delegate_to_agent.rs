use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{
    agent_name_values, bound_conversation_id, parse_agent_arg, required_string_arg,
    TeamworkMcpTool, ToolCallContext,
};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct DelegateToAgentTool;

impl TeamworkMcpTool for DelegateToAgentTool {
    fn name(&self) -> &'static str {
        "delegate_to_agent"
    }

    fn description(&self) -> &'static str {
        "Delegate a focused task to another Minos agent in this conversation."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::DelegateToAgent
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "target_agent".into(),
            json!({
                "type": "string",
                "enum": agent_name_values(),
                "description": "The Minos agent that should receive the delegated work."
            }),
        );
        properties.insert(
            "prompt".into(),
            json!({
                "type": "string",
                "description": "The task prompt to send to the target agent."
            }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["target_agent", "prompt"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let conversation_id = bound_conversation_id(&args, &ctx, self.name())?;
        let target_agent = parse_agent_arg(required_string_arg(&args, "target_agent")?)?;
        let prompt = required_string_arg(&args, "prompt")?.trim().to_owned();
        anyhow::ensure!(!prompt.is_empty(), "prompt must not be empty");
        Ok(SocketRequest::DelegateToAgent {
            conversation_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            source_thread_id: ctx.source_thread_id,
            target_agent: target_agent.bin_name().to_owned(),
            prompt,
        })
    }
}

#[cfg(test)]
mod tests {
    use minos_domain::AgentName;
    use serde_json::json;

    use super::*;

    #[test]
    fn delegate_request_includes_bound_source_thread_id() {
        let request = DelegateToAgentTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "conversation-main".into(),
                    source_agent: Some(AgentName::Opencode),
                    source_thread_id: Some("thread-opencode-1234".into()),
                },
                json!({"target_agent": "codex", "prompt": "say hi"}),
            )
            .expect("valid delegate request");

        match request {
            SocketRequest::DelegateToAgent {
                conversation_id,
                source_agent,
                source_thread_id,
                target_agent,
                prompt,
            } => {
                assert_eq!(conversation_id, "conversation-main");
                assert_eq!(source_agent.as_deref(), Some("opencode"));
                assert_eq!(source_thread_id.as_deref(), Some("thread-opencode-1234"));
                assert_eq!(target_agent, "codex");
                assert_eq!(prompt, "say hi");
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }
}
