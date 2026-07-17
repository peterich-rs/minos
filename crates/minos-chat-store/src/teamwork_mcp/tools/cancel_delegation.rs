use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{
    bound_conversation_id, optional_string_arg, required_string_arg, TeamworkMcpTool,
    ToolCallContext,
};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct CancelDelegationTool;

impl TeamworkMcpTool for CancelDelegationTool {
    fn name(&self) -> &'static str {
        "cancel_delegation"
    }

    fn description(&self) -> &'static str {
        "Mark a delegated Minos teamwork task as cancelled and interrupt its thread when available."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::CancelDelegation
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
        properties.insert(
            "reason".into(),
            json!({
                "type": "string",
                "description": "Optional cancellation reason."
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
        let reason = optional_string_arg(&args, "reason")
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned);
        Ok(SocketRequest::CancelDelegation {
            conversation_id,
            delegation_id,
            reason,
        })
    }
}
