//! Create and remove conversation-scoped git worktrees.

use std::path::{Path, PathBuf};

use super::exec::{is_inside_work_tree, run_git, show_toplevel};
use super::snapshot::detect_git_snapshot;

const WORKTREES_DIR_NAME: &str = ".minos-worktrees";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeCreateResult {
    pub path: PathBuf,
    pub branch: String,
    pub base_branch: Option<String>,
    pub created: bool,
}

/// Sanitize a free-form title into a short branch/slug segment.
pub fn slugify_segment(raw: &str, max_len: usize) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.chars() {
        let c = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else if ch == '-' || ch == '_' || ch == '/' || ch.is_whitespace() {
            '-'
        } else {
            continue;
        };
        if c == '-' {
            if prev_dash || out.is_empty() {
                continue;
            }
            prev_dash = true;
            out.push('-');
        } else {
            prev_dash = false;
            out.push(c);
        }
        if out.len() >= max_len {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "work".into()
    } else {
        out
    }
}

/// Parent of the repo toplevel + `.minos-worktrees`.
pub fn worktrees_root_for_repo(repo_toplevel: &Path) -> PathBuf {
    let parent = repo_toplevel
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo_toplevel.to_path_buf());
    parent.join(WORKTREES_DIR_NAME)
}

pub fn default_branch_name(conversation_id: &str, title: &str) -> String {
    let short_id: String = conversation_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    let slug = slugify_segment(title, 32);
    if short_id.is_empty() {
        format!("minos/{slug}")
    } else {
        format!("minos/{slug}-{short_id}")
    }
}

/// Create a linked worktree for a conversation work unit.
///
/// - Base: current HEAD of `project_workspace` (must be a git checkout).
/// - Path: `{repo_parent}/.minos-worktrees/{slug}-{short_id}`
/// - Branch: `minos/{slug}-{short_id}` (new branch from HEAD)
///
/// Idempotent: if the destination worktree already exists and is a git tree,
/// reuses it and returns `created: false`.
pub fn create_conversation_worktree(
    project_workspace: &Path,
    conversation_id: &str,
    title: &str,
) -> Result<WorktreeCreateResult, String> {
    if !is_inside_work_tree(project_workspace) {
        return Err(format!(
            "project workspace is not a git repository: {}",
            project_workspace.display()
        ));
    }
    let toplevel = show_toplevel(project_workspace)?;
    let base_branch = detect_git_snapshot(&toplevel).0;
    let branch = default_branch_name(conversation_id, title);

    let short_id: String = conversation_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    let dir_name = format!("{}-{}", slugify_segment(title, 24), short_id);
    let root = worktrees_root_for_repo(&toplevel);
    std::fs::create_dir_all(&root).map_err(|e| {
        format!(
            "failed to create worktrees root {}: {e}",
            root.display()
        )
    })?;
    let path = root.join(dir_name);

    if path.is_dir() && is_inside_work_tree(&path) {
        let (existing_branch, _) = detect_git_snapshot(&path);
        return Ok(WorktreeCreateResult {
            path,
            branch: existing_branch.unwrap_or(branch),
            base_branch,
            created: false,
        });
    }

    if path.exists() {
        return Err(format!(
            "worktree path already exists and is not a git worktree: {}",
            path.display()
        ));
    }

    // Prefer creating a new branch. If the branch name already exists, check it out.
    let path_str = path.to_string_lossy().into_owned();
    let create_new = run_git(
        &toplevel,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &path_str,
            "HEAD",
        ],
    );
    match create_new {
        Ok(_) => Ok(WorktreeCreateResult {
            path,
            branch,
            base_branch,
            created: true,
        }),
        Err(err) if err.contains("already exists") || err.contains("already checked out") => {
            // Branch exists — attach worktree without -b.
            run_git(
                &toplevel,
                &["worktree", "add", &path_str, &branch],
            )?;
            Ok(WorktreeCreateResult {
                path,
                branch,
                base_branch,
                created: true,
            })
        }
        Err(err) => Err(err),
    }
}

/// Remove a conversation worktree. Best-effort; ignores missing paths.
pub fn remove_conversation_worktree(
    project_workspace: &Path,
    worktree_path: &Path,
) -> Result<(), String> {
    if !worktree_path.exists() {
        return Ok(());
    }
    if !is_inside_work_tree(project_workspace) {
        // Fall back to plain directory remove when project is no longer a git repo.
        if worktree_path.is_dir() {
            std::fs::remove_dir_all(worktree_path)
                .map_err(|e| format!("failed to remove {}: {e}", worktree_path.display()))?;
        }
        return Ok(());
    }
    let toplevel = show_toplevel(project_workspace)?;
    let path_str = worktree_path.to_string_lossy().into_owned();
    match run_git(
        &toplevel,
        &["worktree", "remove", "--force", &path_str],
    ) {
        Ok(_) => Ok(()),
        Err(_) => {
            // Last resort: prune + delete directory.
            let _ = run_git(&toplevel, &["worktree", "prune"]);
            if worktree_path.is_dir() {
                std::fs::remove_dir_all(worktree_path)
                    .map_err(|e| format!("failed to remove {}: {e}", worktree_path.display()))?;
            }
            Ok(())
        }
    }
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
    fn slugify_strips_noise() {
        assert_eq!(slugify_segment("Hello World!!", 32), "hello-world");
        assert_eq!(slugify_segment("///", 32), "work");
    }

    #[test]
    fn create_worktree_makes_branch_and_path() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_ok(&repo, &["init", "-b", "main"]);
        git_ok(&repo, &["config", "user.email", "test@example.com"]);
        git_ok(&repo, &["config", "user.name", "test"]);
        std::fs::write(repo.join("README"), "x").unwrap();
        git_ok(&repo, &["add", "README"]);
        git_ok(&repo, &["commit", "-m", "init"]);

        let result = create_conversation_worktree(&repo, "conv-abcdef12", "Fix Auth Flow")
            .expect("create worktree");
        assert!(result.created);
        assert!(result.path.is_dir());
        assert!(is_inside_work_tree(&result.path));
        assert!(result.branch.starts_with("minos/fix-auth-flow-"));
        assert_eq!(result.base_branch.as_deref(), Some("main"));

        let (branch, wt) = detect_git_snapshot(&result.path);
        assert_eq!(branch.as_deref(), Some(result.branch.as_str()));
        assert!(wt.is_some());

        // Idempotent reuse
        let again = create_conversation_worktree(&repo, "conv-abcdef12", "Fix Auth Flow")
            .expect("reuse");
        assert!(!again.created);
        assert_eq!(again.path, result.path);
    }
}
