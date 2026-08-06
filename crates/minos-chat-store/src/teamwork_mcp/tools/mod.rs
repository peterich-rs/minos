mod cancel_delegation;
mod delegate_to_agent;
mod get_delegation_status;
mod list_conversation_messages;
mod list_conversation_roster;
mod post_conversation_update;
mod post_git_update;
mod react_to_message;
mod wait_delegation;

use anyhow::{Context, Result};
use minos_domain::AgentName;
use serde_json::{Map, Value};

use super::catalog::{SkillRef, ToolCallContext};
use super::permissions::TeamworkMcpPermission;
use crate::mcp_socket::SocketRequest;

pub use cancel_delegation::CancelDelegationTool;
pub use delegate_to_agent::DelegateToAgentTool;
pub use get_delegation_status::GetDelegationStatusTool;
pub use list_conversation_messages::ListConversationMessagesTool;
pub use list_conversation_roster::ListConversationRosterTool;
pub use post_conversation_update::PostConversationUpdateTool;
pub use post_git_update::PostGitUpdateTool;
pub use react_to_message::ReactToMessageTool;
pub use wait_delegation::WaitDelegationTool;

pub trait TeamworkMcpTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn permission(&self) -> TeamworkMcpPermission;
    fn input_schema(&self) -> Value;
    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest>;

    fn schema(&self) -> Value {
        serde_json::json!({
            "name": self.name(),
            "description": self.description(),
            "inputSchema": self.input_schema(),
        })
    }

    fn skill_refs(&self) -> &'static [SkillRef] {
        &[]
    }
}

fn bound_conversation_id(args: &Value, ctx: &ToolCallContext, tool_name: &str) -> Result<String> {
    anyhow::ensure!(
        args.get("conversation_id").is_none(),
        "{tool_name} does not accept conversation_id; this MCP server is bound to one conversation at startup"
    );
    Ok(ctx.conversation_id.clone())
}

fn required_string_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .with_context(|| format!("{name} is required"))
}

fn optional_string_arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

fn parse_agent_arg(value: &str) -> Result<AgentName> {
    let normalized = value.trim().to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
        .with_context(|| format!("unknown agent: {value}"))
}

fn agent_name_values() -> Vec<&'static str> {
    AgentName::all()
        .iter()
        .map(|agent| agent.bin_name())
        .collect()
}

fn pagination_properties() -> Map<String, Value> {
    Map::from_iter([
        (
            "before_seq".into(),
            serde_json::json!({
                "type": "integer",
                "minimum": 1,
                "description": "Return messages with seq lower than this cursor."
            }),
        ),
        (
            "limit".into(),
            serde_json::json!({
                "type": "integer",
                "minimum": 1,
                "maximum": 500,
                "description": "Maximum messages to return. Defaults to 100."
            }),
        ),
    ])
}
