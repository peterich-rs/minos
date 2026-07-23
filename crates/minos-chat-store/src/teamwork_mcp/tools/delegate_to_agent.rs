use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{
    agent_name_values, bound_conversation_id, optional_string_arg, parse_agent_arg,
    required_string_arg, TeamworkMcpTool, ToolCallContext,
};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct DelegateToAgentTool;

impl TeamworkMcpTool for DelegateToAgentTool {
    fn name(&self) -> &'static str {
        "delegate_to_agent"
    }

    fn description(&self) -> &'static str {
        "Delegate a focused task to another Minos agent in this conversation. \
         Prefer profile_id when targeting a host agent profile; optional target_profile \
         resolves by unique profile name. With only target_agent, Minos starts a new \
         conversation-bound session and applies the newest host profile for that runtime \
         when one exists (same convenience as desktop bare @agent). Launch fields \
         (model/effort/instructions) are server-owned via profile_id — do not pass them here."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::DelegateToAgent
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "target_agent".into(),
            json!({
                "type": "string",
                "enum": agent_name_values(),
                "description": "Runtime agent that should receive the work (codex/claude/…). \
Optional when profile_id or target_profile is set; when only this is set, the newest \
host profile for the runtime is applied if one exists."
            }),
        );
        properties.insert(
            "profile_id".into(),
            json!({
                "type": "string",
                "description": "Host agent profile id to bind at session create (stable). \
When set, target_agent must match the profile runtime if both are provided."
            }),
        );
        properties.insert(
            "target_profile".into(),
            json!({
                "type": "string",
                "description": "Host agent profile display name (case-insensitive unique match). \
Prefer profile_id when known. Ignored when profile_id is set."
            }),
        );
        properties.insert(
            "prompt".into(),
            json!({
                "type": "string",
                "description": "The task prompt to send to the target agent."
            }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["prompt"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let conversation_id = bound_conversation_id(&args, &ctx, self.name())?;
        let prompt = required_string_arg(&args, "prompt")?.trim().to_owned();
        anyhow::ensure!(!prompt.is_empty(), "prompt must not be empty");

        let target_agent = optional_string_arg(&args, "target_agent")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(parse_agent_arg)
            .transpose()?
            .map(|agent| agent.bin_name().to_owned());
        let profile_id = optional_string_arg(&args, "profile_id")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let target_profile = optional_string_arg(&args, "target_profile")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        anyhow::ensure!(
            target_agent.is_some() || profile_id.is_some() || target_profile.is_some(),
            "delegate_to_agent requires target_agent, profile_id, or target_profile"
        );

        Ok(SocketRequest::DelegateToAgent {
            conversation_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            source_session_id: ctx.source_session_id,
            target_agent,
            profile_id,
            target_profile,
            prompt,
        })
    }
}

#[cfg(test)]
mod tests {
    use minos_domain::AgentName;
    use serde_json::json;

    use super::*;

    #[test]
    fn delegate_request_includes_bound_source_session_id() {
        let request = DelegateToAgentTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "conversation-main".into(),
                    source_agent: Some(AgentName::Opencode),
                    source_session_id: Some("thread-opencode-1234".into()),
                },
                json!({"target_agent": "codex", "prompt": "say hi"}),
            )
            .expect("valid delegate request");

        match request {
            SocketRequest::DelegateToAgent {
                conversation_id,
                source_agent,
                source_session_id,
                target_agent,
                profile_id,
                target_profile,
                prompt,
            } => {
                assert_eq!(conversation_id, "conversation-main");
                assert_eq!(source_agent.as_deref(), Some("opencode"));
                assert_eq!(source_session_id.as_deref(), Some("thread-opencode-1234"));
                assert_eq!(target_agent.as_deref(), Some("codex"));
                assert!(profile_id.is_none());
                assert!(target_profile.is_none());
                assert_eq!(prompt, "say hi");
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn delegate_request_accepts_profile_id_without_target_agent() {
        let request = DelegateToAgentTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "conversation-main".into(),
                    source_agent: Some(AgentName::Codex),
                    source_session_id: Some("thread-codex-1".into()),
                },
                json!({
                    "profile_id": "profile-research",
                    "prompt": "deep dive"
                }),
            )
            .expect("profile_id alone is valid");

        match request {
            SocketRequest::DelegateToAgent {
                target_agent,
                profile_id,
                target_profile,
                prompt,
                ..
            } => {
                assert!(target_agent.is_none());
                assert_eq!(profile_id.as_deref(), Some("profile-research"));
                assert!(target_profile.is_none());
                assert_eq!(prompt, "deep dive");
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn delegate_request_accepts_target_profile_name() {
        let request = DelegateToAgentTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "c1".into(),
                    source_agent: None,
                    source_session_id: None,
                },
                json!({
                    "target_profile": "ResearchGrok",
                    "prompt": "analyze"
                }),
            )
            .expect("target_profile alone is valid");

        match request {
            SocketRequest::DelegateToAgent {
                target_agent,
                profile_id,
                target_profile,
                ..
            } => {
                assert!(target_agent.is_none());
                assert!(profile_id.is_none());
                assert_eq!(target_profile.as_deref(), Some("ResearchGrok"));
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn delegate_request_rejects_missing_target() {
        let err = DelegateToAgentTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "c1".into(),
                    source_agent: None,
                    source_session_id: None,
                },
                json!({"prompt": "help"}),
            )
            .expect_err("must name a target");
        assert!(err.to_string().contains("target_agent"));
    }

    #[test]
    fn input_schema_documents_profile_fields() {
        let schema = DelegateToAgentTool.input_schema();
        let props = schema
            .get("properties")
            .expect("properties")
            .as_object()
            .expect("object");
        assert!(props.contains_key("profile_id"));
        assert!(props.contains_key("target_profile"));
        assert!(props.contains_key("target_agent"));
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required");
        assert_eq!(required, &vec![json!("prompt")]);
    }
}
