//! Best-effort git context for conversation create-time snapshots.

use std::path::Path;
use std::process::Command;

/// Detect git branch and optional linked-worktree path for a workspace.
///
/// Returns `(branch, worktree_path)`. Both are `None` when the path is not a
/// git work tree or `git` is unavailable. Branch is a create-time snapshot
/// (does not track later checkouts). `worktree_path` is set only for linked
/// worktrees (where `.git` is a file), using `git rev-parse --show-toplevel`.
pub fn detect_git_snapshot(workspace: &Path) -> (Option<String>, Option<String>) {
    if !workspace.is_dir() {
        return (None, None);
    }

    let inside = git_stdout(workspace, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !inside {
        return (None, None);
    }

    let branch = match git_stdout(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(name) if name == "HEAD" => {
            // Detached HEAD — show short SHA instead of the literal "HEAD".
            git_stdout(workspace, &["rev-parse", "--short", "HEAD"]).ok()
        }
        Ok(name) if !name.is_empty() => Some(name),
        _ => None,
    };

    // Linked worktree: `.git` is a file pointing at the common git dir.
    let worktree_path = if workspace.join(".git").is_file() {
        git_stdout(workspace, &["rev-parse", "--show-toplevel"]).ok()
    } else {
        None
    };

    (branch, worktree_path)
}

fn git_stdout(cwd: &Path, args: &[&str]) -> Result<String, ()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if text.is_empty() {
        Err(())
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn non_git_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let (branch, wt) = detect_git_snapshot(tmp.path());
        assert!(branch.is_none());
        assert!(wt.is_none());
    }

    #[test]
    fn git_repo_captures_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let run = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .expect("git");
            assert!(st.success(), "git {args:?} failed");
        };
        run(&["init", "-b", "feature/snapshot-test"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "test"]);
        std::fs::write(root.join("README"), "x").unwrap();
        run(&["add", "README"]);
        run(&["commit", "-m", "init"]);

        let (branch, wt) = detect_git_snapshot(root);
        assert_eq!(branch.as_deref(), Some("feature/snapshot-test"));
        assert!(wt.is_none(), "main checkout is not a linked worktree");
    }
}
