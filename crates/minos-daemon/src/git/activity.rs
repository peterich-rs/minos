//! Structured git activity embedded in conversation messages.

use minos_protocol::GitActivity;

const MARKER_PREFIX: &str = "<!--minos-git-activity:";
const MARKER_SUFFIX: &str = "-->";

/// Human-readable summary for timeline preview.
pub fn activity_summary(activity: &GitActivity) -> String {
    match activity {
        GitActivity::WorktreeCreated {
            branch,
            worktree_path,
            base_branch,
        } => {
            let base = base_branch
                .as_deref()
                .map(|b| format!(" from `{b}`"))
                .unwrap_or_default();
            format!("Git: worktree created on `{branch}`{base} → {worktree_path}")
        }
        GitActivity::CommitsMade {
            count, subjects, ..
        } => {
            let preview = subjects
                .first()
                .map(|s| format!(": {s}"))
                .unwrap_or_default();
            format!("Git: {count} commit(s){preview}")
        }
        GitActivity::PrOpened { url, number, .. } => match number {
            Some(n) => format!("Git: PR #{n} opened — {url}"),
            None => format!("Git: PR opened — {url}"),
        },
        GitActivity::ChecksFailed { summary } => format!("Git: checks failed — {summary}"),
        GitActivity::ReadyForReview { branch, .. } => {
            format!("Git: ready for review on `{branch}`")
        }
        GitActivity::Merged {
            merge_commit,
            branch,
        } => {
            let br = branch
                .as_deref()
                .map(|b| format!(" (`{b}`)"))
                .unwrap_or_default();
            match merge_commit {
                Some(c) => format!("Git: merged{br} — {c}"),
                None => format!("Git: merged{br}"),
            }
        }
    }
}

/// Encode activity as a timeline body (summary + embedded JSON).
pub fn format_activity_body(activity: &GitActivity) -> Result<String, String> {
    let json = serde_json::to_string(activity).map_err(|e| e.to_string())?;
    Ok(format!(
        "{}\n\n{MARKER_PREFIX}{json}{MARKER_SUFFIX}",
        activity_summary(activity)
    ))
}

/// Parse embedded activity from a conversation message body.
pub fn parse_activity_body(body: &str) -> Option<GitActivity> {
    let start = body.find(MARKER_PREFIX)? + MARKER_PREFIX.len();
    let rest = &body[start..];
    let end = rest.find(MARKER_SUFFIX)?;
    let json = rest[..end].trim();
    serde_json::from_str(json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_activity_body() {
        let activity = GitActivity::WorktreeCreated {
            branch: "minos/fix-1".into(),
            worktree_path: "/tmp/wt".into(),
            base_branch: Some("main".into()),
        };
        let body = format_activity_body(&activity).unwrap();
        assert!(body.starts_with("Git: worktree created"));
        let parsed = parse_activity_body(&body).expect("parse");
        assert_eq!(parsed, activity);
    }
}
