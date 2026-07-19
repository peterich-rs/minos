use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{bound_conversation_id, required_string_arg, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct WaitDelegationTool;

impl TeamworkMcpTool for WaitDelegationTool {
    fn name(&self) -> &'static str {
        "wait_delegation"
    }

    fn description(&self) -> &'static str {
        "Block until a previously delegated Minos teamwork task reaches a terminal status (completed, cancelled, or failed), or until timeout_ms elapses. Prefer this when the next critical-path step needs the delegated result. The final content is also pushed into the source agent session when delivery succeeds."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::WaitDelegation
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
            "timeout_ms".into(),
            json!({
                "type": "integer",
                "minimum": 1000,
                "maximum": 600_000,
                "description": "Maximum time to wait in milliseconds. Defaults to 30000."
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
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_i64)
            .unwrap_or(30_000);
        anyhow::ensure!(
            (1_000..=600_000).contains(&timeout_ms),
            "timeout_ms must be between 1000 and 600000"
        );
        Ok(SocketRequest::WaitDelegation {
            conversation_id,
            delegation_id,
            timeout_ms,
        })
    }
}
