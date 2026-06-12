use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{bound_room_id, required_string_arg, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct PostRoomUpdateTool;

impl TeamworkMcpTool for PostRoomUpdateTool {
    fn name(&self) -> &'static str {
        "post_room_update"
    }

    fn description(&self) -> &'static str {
        "Post a concise user-visible update into this Minos teamwork room."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::PostRoomUpdate
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "message".into(),
            json!({
                "type": "string",
                "description": "A concise user-visible update to post in the room."
            }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["message"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let room_id = bound_room_id(&args, &ctx, self.name())?;
        let message = required_string_arg(&args, "message")?.trim().to_owned();
        anyhow::ensure!(!message.is_empty(), "message must not be empty");
        Ok(SocketRequest::PostRoomUpdate {
            room_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            message,
        })
    }
}
