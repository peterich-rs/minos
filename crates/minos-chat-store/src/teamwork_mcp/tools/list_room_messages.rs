use anyhow::Result;
use serde_json::{json, Value};

use super::{bound_room_id, pagination_properties, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct ListRoomMessagesTool;

impl TeamworkMcpTool for ListRoomMessagesTool {
    fn name(&self) -> &'static str {
        "list_room_messages"
    }

    fn description(&self) -> &'static str {
        "Read messages from the Minos teamwork room bound to this MCP server, newest-first with cursor pagination."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::ListRoomMessages
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": pagination_properties()
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let room_id = bound_room_id(&args, &ctx, self.name())?;
        let before_seq = args.get("before_seq").and_then(Value::as_u64);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        Ok(SocketRequest::ListRoomMessages {
            room_id,
            before_seq,
            limit,
        })
    }
}
