use anyhow::{Context, Result};
use minos_domain::AgentName;
use serde_json::Value;

use super::permissions::TeamworkMcpPermissions;
use super::tools::{
    CancelDelegationTool, DelegateToAgentTool, GetDelegationStatusTool,
    ListConversationMessagesTool, ListConversationRosterTool, PostConversationUpdateTool,
    PostGitUpdateTool, ReactToMessageTool, TeamworkMcpTool, WaitDelegationTool,
};
use crate::mcp_socket::SocketRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillInjectWhen {
    ServerEnabled,
    ToolEnabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillRef {
    pub id: &'static str,
    pub path: &'static str,
    pub inject_when: SkillInjectWhen,
}

pub const MINOS_TEAMWORK_SKILL: SkillRef = SkillRef {
    id: minos_prompt_runtime::TEAMWORK_SKILL_ID,
    path: minos_prompt_runtime::TEAMWORK_SKILL_REPO_PATH,
    inject_when: SkillInjectWhen::ServerEnabled,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallContext {
    pub conversation_id: String,
    pub source_agent: Option<AgentName>,
    pub source_session_id: Option<String>,
}

pub struct TeamworkMcpToolCatalog {
    tools: Vec<Box<dyn TeamworkMcpTool>>,
}

impl TeamworkMcpToolCatalog {
    pub fn default_catalog() -> Self {
        Self {
            tools: vec![
                Box::new(ListConversationMessagesTool),
                Box::new(ListConversationRosterTool),
                Box::new(DelegateToAgentTool),
                Box::new(GetDelegationStatusTool),
                Box::new(WaitDelegationTool),
                Box::new(CancelDelegationTool),
                Box::new(PostConversationUpdateTool),
                Box::new(PostGitUpdateTool),
                Box::new(ReactToMessageTool),
            ],
        }
    }

    pub fn tool_schemas(&self, permissions: TeamworkMcpPermissions) -> Vec<Value> {
        self.tools
            .iter()
            .filter(|tool| permissions.allows(tool.permission()))
            .map(|tool| tool.schema())
            .collect()
    }

    pub fn socket_request_for_call(
        &self,
        permissions: TeamworkMcpPermissions,
        ctx: ToolCallContext,
        name: &str,
        args: Value,
    ) -> Result<SocketRequest> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == name)
            .with_context(|| format!("unknown Minos MCP tool: {name}"))?;
        anyhow::ensure!(
            permissions.allows(tool.permission()),
            "{name} is disabled by this MCP server policy"
        );
        tool.to_socket_request(ctx, args)
    }

    pub fn skill_refs(&self, permissions: TeamworkMcpPermissions) -> Vec<SkillRef> {
        let mut refs = vec![MINOS_TEAMWORK_SKILL];
        for tool in &self.tools {
            if permissions.allows(tool.permission()) {
                refs.extend(tool.skill_refs());
            }
        }
        refs.sort_by_key(|skill| skill.id);
        refs.dedup_by_key(|skill| skill.id);
        refs
    }
}

impl Default for TeamworkMcpToolCatalog {
    fn default() -> Self {
        Self::default_catalog()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn catalog_lists_enabled_tool_schemas() {
        let catalog = TeamworkMcpToolCatalog::default_catalog();
        let schemas = catalog.tool_schemas(TeamworkMcpPermissions {
            list_conversation_messages: true,
            list_conversation_roster: true,
            delegate_to_agent: false,
            get_delegation_status: true,
            wait_delegation: true,
            cancel_delegation: true,
            post_conversation_update: true,
            post_git_update: true,
            react_to_message: true,
        });
        let names: Vec<_> = schemas
            .iter()
            .filter_map(|schema| schema.get("name").and_then(Value::as_str))
            .collect();

        assert_eq!(
            names,
            vec![
                "list_conversation_messages",
                "list_conversation_roster",
                "get_delegation_status",
                "wait_delegation",
                "cancel_delegation",
                "post_conversation_update",
                "post_git_update",
                "react_to_message"
            ]
        );
    }

    #[test]
    fn catalog_rejects_disabled_tool_calls() {
        let catalog = TeamworkMcpToolCatalog::default_catalog();
        let err = catalog
            .socket_request_for_call(
                TeamworkMcpPermissions {
                    list_conversation_messages: true,
                    list_conversation_roster: true,
                    delegate_to_agent: false,
                    get_delegation_status: true,
                    wait_delegation: true,
                    cancel_delegation: true,
                    post_conversation_update: true,
                    post_git_update: true,
                    react_to_message: true,
                },
                ToolCallContext {
                    conversation_id: "conversation-main".into(),
                    source_agent: None,
                    source_session_id: None,
                },
                "delegate_to_agent",
                json!({"target_agent": "codex", "prompt": "help"}),
            )
            .expect_err("disabled tool should fail");

        assert!(err.to_string().contains("delegate_to_agent is disabled"));
    }

    #[test]
    fn catalog_exposes_server_skill_ref() {
        let catalog = TeamworkMcpToolCatalog::default_catalog();
        let refs = catalog.skill_refs(TeamworkMcpPermissions::default());

        assert_eq!(refs, vec![MINOS_TEAMWORK_SKILL]);
    }
}
