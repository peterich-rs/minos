use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{
    agent_name_values, bound_room_id, parse_agent_arg, required_string_arg, TeamworkMcpTool,
    ToolCallContext,
};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct DelegateToAgentTool;

impl TeamworkMcpTool for DelegateToAgentTool {
    fn name(&self) -> &'static str {
        "delegate_to_agent"
    }

    fn description(&self) -> &'static str {
        "Delegate a focused task to another Minos agent in this teamwork room."
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
        let room_id = bound_room_id(&args, &ctx, self.name())?;
        let target_agent = parse_agent_arg(required_string_arg(&args, "target_agent")?)?;
        let prompt = required_string_arg(&args, "prompt")?.trim().to_owned();
        anyhow::ensure!(!prompt.is_empty(), "prompt must not be empty");
        Ok(SocketRequest::DelegateToAgent {
            room_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            target_agent: target_agent.bin_name().to_owned(),
            prompt,
        })
    }
}
