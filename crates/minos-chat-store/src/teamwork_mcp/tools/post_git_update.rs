use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use super::{bound_conversation_id, required_string_arg, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

pub struct PostGitUpdateTool;

impl TeamworkMcpTool for PostGitUpdateTool {
    fn name(&self) -> &'static str {
        "post_git_update"
    }

    fn description(&self) -> &'static str {
        "Post a structured git milestone into this Minos conversation (worktree, commits, PR, review, merge). Prefer this over free-form post_conversation_update for git delivery status."
    }

    fn permission(&self) -> TeamworkMcpPermission {
        TeamworkMcpPermission::PostGitUpdate
    }

    fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "kind".into(),
            json!({
                "type": "string",
                "description": "Milestone kind",
                "enum": [
                    "worktree_created",
                    "commits_made",
                    "pr_opened",
                    "checks_failed",
                    "ready_for_review",
                    "merged"
                ]
            }),
        );
        properties.insert(
            "branch".into(),
            json!({ "type": "string", "description": "Branch name when applicable" }),
        );
        properties.insert(
            "worktree_path".into(),
            json!({ "type": "string", "description": "Worktree path (worktree_created)" }),
        );
        properties.insert(
            "base_branch".into(),
            json!({ "type": "string", "description": "Base branch (worktree_created)" }),
        );
        properties.insert(
            "count".into(),
            json!({ "type": "integer", "description": "Commit count (commits_made)" }),
        );
        properties.insert(
            "subjects".into(),
            json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Recent commit subjects (commits_made)"
            }),
        );
        properties.insert(
            "head".into(),
            json!({ "type": "string", "description": "HEAD sha when known" }),
        );
        properties.insert(
            "url".into(),
            json!({ "type": "string", "description": "Pull request URL (pr_opened)" }),
        );
        properties.insert(
            "number".into(),
            json!({ "type": "integer", "description": "Pull request number (pr_opened)" }),
        );
        properties.insert(
            "title".into(),
            json!({ "type": "string", "description": "PR title (pr_opened)" }),
        );
        properties.insert(
            "summary".into(),
            json!({ "type": "string", "description": "Failure summary (checks_failed)" }),
        );
        properties.insert(
            "merge_commit".into(),
            json!({ "type": "string", "description": "Merge commit (merged)" }),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": ["kind"]
        })
    }

    fn to_socket_request(&self, ctx: ToolCallContext, args: Value) -> Result<SocketRequest> {
        let conversation_id = bound_conversation_id(&args, &ctx, self.name())?;
        let kind = required_string_arg(&args, "kind")?.trim().to_ascii_lowercase();
        let activity = build_activity(&kind, &args)?;
        Ok(SocketRequest::PostGitUpdate {
            conversation_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            source_session_id: ctx.source_session_id,
            activity,
        })
    }
}

fn build_activity(kind: &str, args: &Value) -> Result<Value> {
    // Forward as JSON object with `kind` tag matching protocol GitActivity.
    let mut map = Map::new();
    map.insert("kind".into(), json!(kind));
    match kind {
        "worktree_created" => {
            map.insert(
                "branch".into(),
                json!(required_string_arg(args, "branch")?.trim()),
            );
            map.insert(
                "worktree_path".into(),
                json!(required_string_arg(args, "worktree_path")?.trim()),
            );
            if let Some(base) = args.get("base_branch").and_then(Value::as_str) {
                let base = base.trim();
                if !base.is_empty() {
                    map.insert("base_branch".into(), json!(base));
                }
            }
        }
        "commits_made" => {
            let count = args
                .get("count")
                .and_then(Value::as_u64)
                .context("count is required for commits_made")?;
            map.insert("count".into(), json!(count));
            if let Some(subjects) = args.get("subjects").cloned() {
                map.insert("subjects".into(), subjects);
            }
            if let Some(head) = args.get("head").and_then(Value::as_str) {
                let head = head.trim();
                if !head.is_empty() {
                    map.insert("head".into(), json!(head));
                }
            }
        }
        "pr_opened" => {
            map.insert("url".into(), json!(required_string_arg(args, "url")?.trim()));
            if let Some(n) = args.get("number").and_then(Value::as_u64) {
                map.insert("number".into(), json!(n));
            }
            if let Some(title) = args.get("title").and_then(Value::as_str) {
                let title = title.trim();
                if !title.is_empty() {
                    map.insert("title".into(), json!(title));
                }
            }
        }
        "checks_failed" => {
            map.insert(
                "summary".into(),
                json!(required_string_arg(args, "summary")?.trim()),
            );
        }
        "ready_for_review" => {
            map.insert(
                "branch".into(),
                json!(required_string_arg(args, "branch")?.trim()),
            );
            if let Some(head) = args.get("head").and_then(Value::as_str) {
                let head = head.trim();
                if !head.is_empty() {
                    map.insert("head".into(), json!(head));
                }
            }
        }
        "merged" => {
            if let Some(c) = args.get("merge_commit").and_then(Value::as_str) {
                let c = c.trim();
                if !c.is_empty() {
                    map.insert("merge_commit".into(), json!(c));
                }
            }
            if let Some(b) = args.get("branch").and_then(Value::as_str) {
                let b = b.trim();
                if !b.is_empty() {
                    map.insert("branch".into(), json!(b));
                }
            }
        }
        other => anyhow::bail!("unknown git activity kind: {other}"),
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use minos_domain::AgentName;
    use serde_json::json;

    use super::*;

    #[test]
    fn post_git_update_builds_pr_activity() {
        let request = PostGitUpdateTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "c1".into(),
                    source_agent: Some(AgentName::Codex),
                    source_session_id: Some("s1".into()),
                },
                json!({
                    "kind": "pr_opened",
                    "url": "https://github.com/org/repo/pull/1",
                    "number": 1,
                    "title": "Fix"
                }),
            )
            .expect("request");
        match request {
            SocketRequest::PostGitUpdate {
                conversation_id,
                source_agent,
                source_session_id,
                activity,
            } => {
                assert_eq!(conversation_id, "c1");
                assert_eq!(source_agent.as_deref(), Some("codex"));
                assert_eq!(source_session_id.as_deref(), Some("s1"));
                assert_eq!(activity["kind"], "pr_opened");
                assert_eq!(activity["url"], "https://github.com/org/repo/pull/1");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
