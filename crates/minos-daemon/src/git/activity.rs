//! Structured git activity embedded in conversation messages.

use minos_protocol::GitActivity;

const MARKER_PREFIX: &str = "<!--minos-git-activity:";
const MARKER_SUFFIX: &str = "-->";

/// Defense-in-depth caps (must stay in sync with MCP `post_git_update` tool).
const MAX_SUMMARY_LEN: usize = 2000;
const MAX_URL_LEN: usize = 2048;
const MAX_PATH_LEN: usize = 1024;
const MAX_TITLE_LEN: usize = 200;
const MAX_BRANCH_LEN: usize = 200;
const MAX_HEAD_LEN: usize = 128;
const MAX_SUBJECTS: usize = 50;
const MAX_SUBJECT_LEN: usize = 200;

fn ensure_len(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.chars().count() > max {
        return Err(format!("{field} exceeds max length {max}"));
    }
    Ok(())
}

/// Reject oversized agent-controlled activity fields before persistence/fan-out.
pub fn validate_activity(activity: &GitActivity) -> Result<(), String> {
    match activity {
        GitActivity::WorktreeCreated {
            branch,
            worktree_path,
            base_branch,
        } => {
            ensure_len("branch", branch, MAX_BRANCH_LEN)?;
            ensure_len("worktree_path", worktree_path, MAX_PATH_LEN)?;
            if let Some(b) = base_branch {
                ensure_len("base_branch", b, MAX_BRANCH_LEN)?;
            }
        }
        GitActivity::CommitsMade {
            subjects, head, ..
        } => {
            if subjects.len() > MAX_SUBJECTS {
                return Err(format!("subjects exceeds max count {MAX_SUBJECTS}"));
            }
            for (i, s) in subjects.iter().enumerate() {
                ensure_len(&format!("subjects[{i}]"), s, MAX_SUBJECT_LEN)?;
            }
            if let Some(h) = head {
                ensure_len("head", h, MAX_HEAD_LEN)?;
            }
        }
        GitActivity::PrOpened { url, title, .. } => {
            ensure_len("url", url, MAX_URL_LEN)?;
            if let Some(t) = title {
                ensure_len("title", t, MAX_TITLE_LEN)?;
            }
        }
        GitActivity::ChecksFailed { summary } => {
            ensure_len("summary", summary, MAX_SUMMARY_LEN)?;
        }
        GitActivity::ReadyForReview { branch, head } => {
            ensure_len("branch", branch, MAX_BRANCH_LEN)?;
            if let Some(h) = head {
                ensure_len("head", h, MAX_HEAD_LEN)?;
            }
        }
        GitActivity::Merged {
            merge_commit,
            branch,
        } => {
            if let Some(c) = merge_commit {
                ensure_len("merge_commit", c, MAX_HEAD_LEN)?;
            }
            if let Some(b) = branch {
                ensure_len("branch", b, MAX_BRANCH_LEN)?;
            }
        }
    }
    Ok(())
}

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
    validate_activity(activity)?;
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

    #[test]
    fn rejects_oversized_summary() {
        let activity = GitActivity::ChecksFailed {
            summary: "x".repeat(MAX_SUMMARY_LEN + 1),
        };
        assert!(format_activity_body(&activity).is_err());
    }
}
