use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{bound_room_id, required_string_arg, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct CheckUserFeedbackTool;

impl TeamworkMcpTool for CheckUserFeedbackTool {
    fn name(&self) -> &'static str {
        "check_user_feedback"
    }

    fn description(&self) -> &'static str {
        "Check whether the user has answered a question created by ask_user_question."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::CheckUserFeedback
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "feedback_id".into(),
            json!({
                "type": "string",
                "description": "The feedback id returned by ask_user_question."
            }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["feedback_id"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let room_id = bound_room_id(&args, &ctx, self.name())?;
        let feedback_id = required_string_arg(&args, "feedback_id")?.trim().to_owned();
        anyhow::ensure!(!feedback_id.is_empty(), "feedback_id must not be empty");
        Ok(SocketRequest::CheckUserFeedback {
            room_id,
            feedback_id,
        })
    }
}
