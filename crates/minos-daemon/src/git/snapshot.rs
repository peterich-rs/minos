//! Best-effort git context for conversation snapshots and live status.

use std::path::{Path, PathBuf};

use super::exec::{first_line, is_inside_work_tree, run_git, show_toplevel};

/// Detect git branch and optional linked-worktree path for a workspace.
///
/// Returns `(branch, worktree_path)`. Both are `None` when the path is not a
/// git work tree or `git` is unavailable. Branch is a snapshot of HEAD at call
/// time. `worktree_path` is set only for linked worktrees (where `.git` is a
/// file), using `git rev-parse --show-toplevel`.
pub fn detect_git_snapshot(workspace: &Path) -> (Option<String>, Option<String>) {
    if !workspace.is_dir() || !is_inside_work_tree(workspace) {
        return (None, None);
    }

    let branch = match run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(name) if name == "HEAD" => {
            // Detached HEAD — show short SHA instead of the literal "HEAD".
            run_git(workspace, &["rev-parse", "--short", "HEAD"]).ok()
        }
        Ok(name) if !name.is_empty() => Some(name),
        _ => None,
    };

    // Linked worktree: `.git` is a file pointing at the common git dir.
    let worktree_path = if workspace.join(".git").is_file() {
        show_toplevel(workspace)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    } else {
        None
    };

    (branch, worktree_path)
}

/// Live status for a checkout used as a conversation work unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveGitStatus {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub short_head: Option<String>,
    pub dirty: bool,
    pub has_untracked: bool,
    pub ahead_count: u32,
    pub behind_count: u32,
    pub upstream: Option<String>,
    pub is_linked_worktree: bool,
}

pub fn detect_live_status(workspace: &Path) -> Result<LiveGitStatus, String> {
    if !workspace.is_dir() {
        return Err(format!("path is not a directory: {}", workspace.display()));
    }
    if !is_inside_work_tree(workspace) {
        return Err(format!(
            "path is not a git work tree: {}",
            workspace.display()
        ));
    }

    let branch = match run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(name) if name == "HEAD" => run_git(workspace, &["rev-parse", "--short", "HEAD"]).ok(),
        Ok(name) if !name.is_empty() => Some(name),
        _ => None,
    };
    let head = run_git(workspace, &["rev-parse", "HEAD"]).ok();
    let short_head = run_git(workspace, &["rev-parse", "--short", "HEAD"]).ok();

    let porcelain = run_git(workspace, &["status", "--porcelain=v1"]).unwrap_or_default();
    let dirty = porcelain.lines().any(|line| {
        let trimmed = line.trim_end();
        !trimmed.is_empty() && !trimmed.starts_with("??")
    });
    let has_untracked = porcelain.lines().any(|line| line.starts_with("??"));

    let upstream = run_git(
        workspace,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .ok();

    let (ahead_count, behind_count) = if upstream.is_some() {
        match run_git(
            workspace,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        ) {
            Ok(raw) => {
                let mut parts = raw.split_whitespace();
                let ahead = parts
                    .next()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                let behind = parts
                    .next()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                (ahead, behind)
            }
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };

    Ok(LiveGitStatus {
        path: show_toplevel(workspace).unwrap_or_else(|_| workspace.to_path_buf()),
        branch,
        head,
        short_head,
        dirty: dirty || has_untracked,
        has_untracked,
        ahead_count,
        behind_count,
        upstream,
        is_linked_worktree: workspace.join(".git").is_file(),
    })
}

/// Resolve the primary checkout path for a conversation work unit.
pub fn resolve_work_path(
    worktree_path: Option<&str>,
    project_workspace: Option<&str>,
) -> Option<PathBuf> {
    if let Some(wt) = worktree_path.map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(wt);
        if p.is_dir() {
            return Some(p);
        }
    }
    project_workspace
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

pub fn current_branch_name(workspace: &Path) -> Option<String> {
    first_line(&run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?).and_then(|name| {
        if name == "HEAD" {
            run_git(workspace, &["rev-parse", "--short", "HEAD"]).ok()
        } else {
            Some(name)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git_ok(cwd: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("git");
        assert!(st.success(), "git {args:?} failed");
    }

    #[test]
    fn non_git_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let (branch, wt) = detect_git_snapshot(tmp.path());
        assert!(branch.is_none());
        assert!(wt.is_none());
    }

    #[test]
    fn git_repo_captures_branch_and_live_status() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git_ok(root, &["init", "-b", "feature/snapshot-test"]);
        git_ok(root, &["config", "user.email", "test@example.com"]);
        git_ok(root, &["config", "user.name", "test"]);
        std::fs::write(root.join("README"), "x").unwrap();
        git_ok(root, &["add", "README"]);
        git_ok(root, &["commit", "-m", "init"]);

        let (branch, wt) = detect_git_snapshot(root);
        assert_eq!(branch.as_deref(), Some("feature/snapshot-test"));
        assert!(wt.is_none(), "main checkout is not a linked worktree");

        let live = detect_live_status(root).expect("live status");
        assert_eq!(live.branch.as_deref(), Some("feature/snapshot-test"));
        assert!(!live.dirty);
        assert!(live.head.is_some());

        std::fs::write(root.join("README"), "dirty").unwrap();
        let live = detect_live_status(root).expect("dirty status");
        assert!(live.dirty);
    }
}
