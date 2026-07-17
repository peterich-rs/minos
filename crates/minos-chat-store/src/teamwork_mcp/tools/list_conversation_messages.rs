use anyhow::Result;
use serde_json::{json, Value};

use super::{bound_conversation_id, pagination_properties, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct ListConversationMessagesTool;

impl TeamworkMcpTool for ListConversationMessagesTool {
    fn name(&self) -> &'static str {
        "list_conversation_messages"
    }

    fn description(&self) -> &'static str {
        "Read messages from the Minos conversation bound to this MCP server, newest-first with cursor pagination."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::ListConversationMessages
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": pagination_properties()
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let conversation_id = bound_conversation_id(&args, &ctx, self.name())?;
        let before_seq = args.get("before_seq").and_then(Value::as_u64);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        Ok(SocketRequest::ListConversationMessages {
            conversation_id,
            before_seq,
            limit,
        })
    }
}
