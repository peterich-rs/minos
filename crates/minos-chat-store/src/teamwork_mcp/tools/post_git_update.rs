use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use super::{bound_conversation_id, required_string_arg, TeamworkMcpTool, ToolCallContext};
use crate::mcp_socket::SocketRequest;
use crate::teamwork_mcp::permissions::TeamworkMcpPermission;

/// Per-field caps so agent-controlled git activity cannot flood chat storage / fan-out.
const MAX_SUMMARY_LEN: usize = 2000;
const MAX_URL_LEN: usize = 2048;
const MAX_PATH_LEN: usize = 1024;
const MAX_TITLE_LEN: usize = 200;
const MAX_BRANCH_LEN: usize = 200;
const MAX_HEAD_LEN: usize = 128;
const MAX_SUBJECTS: usize = 50;
const MAX_SUBJECT_LEN: usize = 200;

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
                "maxItems": MAX_SUBJECTS,
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
        let kind = required_string_arg(&args, "kind")?
            .trim()
            .to_ascii_lowercase();
        let activity = build_activity(&kind, &args)?;
        Ok(SocketRequest::PostGitUpdate {
            conversation_id,
            source_agent: ctx.source_agent.map(|agent| agent.bin_name().to_owned()),
            source_session_id: ctx.source_session_id,
            activity,
        })
    }
}

fn capped_required(args: &Value, name: &str, max_len: usize) -> Result<String> {
    let raw = required_string_arg(args, name)?.trim();
    anyhow::ensure!(!raw.is_empty(), "{name} must not be empty");
    anyhow::ensure!(
        raw.chars().count() <= max_len,
        "{name} exceeds max length {max_len}"
    );
    Ok(raw.to_owned())
}

fn capped_optional(args: &Value, name: &str, max_len: usize) -> Result<Option<String>> {
    let Some(raw) = args.get(name).and_then(Value::as_str) else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(
        raw.chars().count() <= max_len,
        "{name} exceeds max length {max_len}"
    );
    Ok(Some(raw.to_owned()))
}

fn capped_subjects(args: &Value) -> Result<Vec<String>> {
    let Some(subjects) = args.get("subjects") else {
        return Ok(Vec::new());
    };
    let arr = subjects
        .as_array()
        .context("subjects must be an array of strings")?;
    anyhow::ensure!(
        arr.len() <= MAX_SUBJECTS,
        "subjects exceeds max count {MAX_SUBJECTS}"
    );
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let s = item
            .as_str()
            .with_context(|| format!("subjects[{i}] must be a string"))?
            .trim();
        if s.is_empty() {
            continue;
        }
        anyhow::ensure!(
            s.chars().count() <= MAX_SUBJECT_LEN,
            "subjects[{i}] exceeds max length {MAX_SUBJECT_LEN}"
        );
        out.push(s.to_owned());
    }
    Ok(out)
}

fn build_activity(kind: &str, args: &Value) -> Result<Value> {
    // Forward as JSON object with `kind` tag matching protocol GitActivity.
    let mut map = Map::new();
    map.insert("kind".into(), json!(kind));
    match kind {
        "worktree_created" => {
            map.insert(
                "branch".into(),
                json!(capped_required(args, "branch", MAX_BRANCH_LEN)?),
            );
            map.insert(
                "worktree_path".into(),
                json!(capped_required(args, "worktree_path", MAX_PATH_LEN)?),
            );
            if let Some(base) = capped_optional(args, "base_branch", MAX_BRANCH_LEN)? {
                map.insert("base_branch".into(), json!(base));
            }
        }
        "commits_made" => {
            let count = args
                .get("count")
                .and_then(Value::as_u64)
                .context("count is required for commits_made")?;
            map.insert("count".into(), json!(count));
            let subjects = capped_subjects(args)?;
            if !subjects.is_empty() {
                map.insert("subjects".into(), json!(subjects));
            }
            if let Some(head) = capped_optional(args, "head", MAX_HEAD_LEN)? {
                map.insert("head".into(), json!(head));
            }
        }
        "pr_opened" => {
            map.insert(
                "url".into(),
                json!(capped_required(args, "url", MAX_URL_LEN)?),
            );
            if let Some(n) = args.get("number").and_then(Value::as_u64) {
                map.insert("number".into(), json!(n));
            }
            if let Some(title) = capped_optional(args, "title", MAX_TITLE_LEN)? {
                map.insert("title".into(), json!(title));
            }
        }
        "checks_failed" => {
            map.insert(
                "summary".into(),
                json!(capped_required(args, "summary", MAX_SUMMARY_LEN)?),
            );
        }
        "ready_for_review" => {
            map.insert(
                "branch".into(),
                json!(capped_required(args, "branch", MAX_BRANCH_LEN)?),
            );
            if let Some(head) = capped_optional(args, "head", MAX_HEAD_LEN)? {
                map.insert("head".into(), json!(head));
            }
        }
        "merged" => {
            if let Some(c) = capped_optional(args, "merge_commit", MAX_HEAD_LEN)? {
                map.insert("merge_commit".into(), json!(c));
            }
            if let Some(b) = capped_optional(args, "branch", MAX_BRANCH_LEN)? {
                map.insert("branch".into(), json!(b));
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

    #[test]
    fn rejects_oversized_summary() {
        let long = "x".repeat(MAX_SUMMARY_LEN + 1);
        let err = PostGitUpdateTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "c1".into(),
                    source_agent: None,
                    source_session_id: None,
                },
                json!({ "kind": "checks_failed", "summary": long }),
            )
            .expect_err("must reject");
        assert!(err.to_string().contains("max length"), "{err}");
    }

    #[test]
    fn rejects_too_many_subjects() {
        let subjects: Vec<String> = (0..MAX_SUBJECTS + 1).map(|i| format!("c{i}")).collect();
        let err = PostGitUpdateTool
            .to_socket_request(
                ToolCallContext {
                    conversation_id: "c1".into(),
                    source_agent: None,
                    source_session_id: None,
                },
                json!({ "kind": "commits_made", "count": 99, "subjects": subjects }),
            )
            .expect_err("must reject");
        assert!(err.to_string().contains("max count"), "{err}");
    }
}
