use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{bound_room_id, required_string_arg, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct AskUserQuestionTool;

impl TeamworkMcpTool for AskUserQuestionTool {
    fn name(&self) -> &'static str {
        "ask_user_question"
    }

    fn description(&self) -> &'static str {
        "Ask the user a concise non-blocking question in the Minos teamwork room."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::AskUserQuestion
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "question".into(),
            json!({
                "type": "string",
                "description": "The concise question to post for the user."
            }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["question"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let room_id = bound_room_id(&args, &ctx, self.name())?;
        let question = required_string_arg(&args, "question")?.trim().to_owned();
        anyhow::ensure!(!question.is_empty(), "question must not be empty");
        Ok(SocketRequest::AskUserQuestion {
            room_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            question,
        })
    }
}
