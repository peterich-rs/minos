use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{
    bound_room_id, optional_string_arg, required_string_arg, TeamworkMcpTool, ToolCallContext,
};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;
use crate::MessageReactionAction;

pub struct ReactToMessageTool;

impl TeamworkMcpTool for ReactToMessageTool {
    fn name(&self) -> &'static str {
        "react_to_message"
    }

    fn description(&self) -> &'static str {
        "Add or remove an emoji reaction on a Minos teamwork room message."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::ReactToMessage
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "message_id".into(),
            json!({
                "type": "string",
                "description": "Message id to react to. Either message_id or message_seq is required."
            }),
        );
        properties.insert(
            "message_seq".into(),
            json!({
                "type": "integer",
                "minimum": 1,
                "description": "Message sequence to react to. Either message_id or message_seq is required."
            }),
        );
        properties.insert(
            "emoji".into(),
            json!({
                "type": "string",
                "description": "The emoji reaction to add or remove."
            }),
        );
        properties.insert(
            "action".into(),
            json!({
                "type": "string",
                "enum": ["add", "remove"],
                "description": "Whether to add or remove the reaction."
            }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["emoji", "action"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let room_id = bound_room_id(&args, &ctx, self.name())?;
        let message_id = optional_string_arg(&args, "message_id")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let message_seq = args.get("message_seq").and_then(Value::as_u64);
        anyhow::ensure!(
            message_id.is_some() || message_seq.is_some(),
            "message_id or message_seq is required"
        );
        let emoji = required_string_arg(&args, "emoji")?.trim().to_owned();
        anyhow::ensure!(!emoji.is_empty(), "emoji must not be empty");
        let action = match required_string_arg(&args, "action")?.trim() {
            "add" => MessageReactionAction::Add,
            "remove" => MessageReactionAction::Remove,
            other => anyhow::bail!("unsupported reaction action: {other}"),
        };
        Ok(SocketRequest::ReactToMessage {
            room_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            message_id,
            message_seq,
            emoji,
            action,
        })
    }
}
