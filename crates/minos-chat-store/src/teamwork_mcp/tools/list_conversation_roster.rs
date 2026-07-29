use anyhow::Result;
use serde_json::{json, Value};

use super::{bound_conversation_id, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct ListConversationRosterTool;

impl TeamworkMcpTool for ListConversationRosterTool {
    fn name(&self) -> &'static str {
        "list_conversation_roster"
    }

    fn description(&self) -> &'static str {
        "List agents on this Minos conversation roster with optional role briefs. \
         Call before delegate_to_agent when you need the live teammate directory \
         (membership can change mid-conversation)."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::ListConversationRoster
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let conversation_id = bound_conversation_id(&args, &ctx, self.name())?;
        Ok(SocketRequest::ListConversationRoster { conversation_id })
    }
}
