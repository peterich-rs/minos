use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{bound_conversation_id, required_string_arg, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct GetDelegationStatusTool;

impl TeamworkMcpTool for GetDelegationStatusTool {
    fn name(&self) -> &'static str {
        "get_delegation_status"
    }

    fn description(&self) -> &'static str {
        "Read the durable status for a previously delegated Minos teamwork task."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::GetDelegationStatus
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "delegation_id".into(),
            json!({
                "type": "string",
                "description": "The delegation id returned by delegate_to_agent."
            }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["delegation_id"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let conversation_id = bound_conversation_id(&args, &ctx, self.name())?;
        let delegation_id = required_string_arg(&args, "delegation_id")?
            .trim()
            .to_owned();
        anyhow::ensure!(!delegation_id.is_empty(), "delegation_id must not be empty");
        Ok(SocketRequest::GetDelegationStatus {
            conversation_id,
            delegation_id,
        })
    }
}
