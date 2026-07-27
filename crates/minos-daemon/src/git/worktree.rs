//! Create and remove conversation-scoped git worktrees.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use super::exec::{git_common_dir, is_inside_work_tree, run_git, show_toplevel};
use super::snapshot::detect_git_snapshot;

const WORKTREES_DIR_NAME: &str = ".minos-worktrees";

/// Per-repo mutex so concurrent `git worktree add` cannot race shared metadata.
static REPO_CREATE_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

fn lock_for_repo(toplevel: &Path) -> Arc<Mutex<()>> {
    let key = toplevel
        .canonicalize()
        .unwrap_or_else(|_| toplevel.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let map = REPO_CREATE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeCreateResult {
    pub path: PathBuf,
    pub branch: String,
    pub base_branch: Option<String>,
    pub created: bool,
}

#[derive(Debug, Clone, Default)]
pub struct OrphanWorktreePruneReport {
    pub scanned_roots: u32,
    pub pruned: u32,
    pub errors: Vec<String>,
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

/// Parent of the **main** worktree + `.minos-worktrees`.
///
/// Uses `git-common-dir` so a linked worktree still places new units next to the
/// primary checkout rather than under the linked path's parent.
pub fn worktrees_root_for_repo(repo_path: &Path) -> PathBuf {
    if let Ok(common) = git_common_dir(repo_path) {
        // common ≈ <main_worktree>/.git
        if let Some(main_wt) = common.parent() {
            if let Some(parent) = main_wt.parent() {
                return parent.join(WORKTREES_DIR_NAME);
            }
            return main_wt.join(WORKTREES_DIR_NAME);
        }
    }
    let parent = repo_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo_path.to_path_buf());
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

fn local_branch_exists(repo: &Path, branch: &str) -> bool {
    let refname = format!("refs/heads/{branch}");
    run_git(repo, &["show-ref", "--verify", "--quiet", &refname]).is_ok()
}

/// Create a linked worktree for a conversation work unit.
///
/// - Base: current HEAD of `project_workspace` (must be a git checkout).
/// - Path: `{main_repo_parent}/.minos-worktrees/{slug}-{short_id}`
/// - Branch: `minos/{slug}-{short_id}` (new branch from HEAD)
///
/// Idempotent: if the destination worktree already exists and is a git tree,
/// reuses it and returns `created: false`.
///
/// Serialized per repository (canonical toplevel) so concurrent creates do not
/// race `.git/worktrees` metadata.
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
    let repo_lock = lock_for_repo(&toplevel);
    let _guard = repo_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    create_conversation_worktree_locked(&toplevel, conversation_id, title)
}

fn create_conversation_worktree_locked(
    toplevel: &Path,
    conversation_id: &str,
    title: &str,
) -> Result<WorktreeCreateResult, String> {
    let base_branch = detect_git_snapshot(toplevel).0;
    let branch = default_branch_name(conversation_id, title);

    let short_id: String = conversation_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    let dir_name = format!("{}-{}", slugify_segment(title, 24), short_id);
    let root = worktrees_root_for_repo(toplevel);
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

    let path_str = path.to_string_lossy().into_owned();
    if local_branch_exists(toplevel, &branch) {
        run_git(toplevel, &["worktree", "add", &path_str, &branch])?;
    } else {
        run_git(
            toplevel,
            &["worktree", "add", "-b", &branch, &path_str, "HEAD"],
        )?;
    }
    Ok(WorktreeCreateResult {
        path,
        branch,
        base_branch,
        created: true,
    })
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

fn canonicalize_lossy(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Startup reconciliation: remove `.minos-worktrees/*` entries not referenced by
/// any conversation row. Does not delete `minos/*` branches (safer; branches may
/// still be useful after detach).
pub fn prune_orphan_worktrees(
    registered_worktree_paths: &[PathBuf],
    project_workspaces: &[PathBuf],
) -> OrphanWorktreePruneReport {
    let registered: HashSet<PathBuf> = registered_worktree_paths
        .iter()
        .map(|p| canonicalize_lossy(p))
        .collect();

    let mut report = OrphanWorktreePruneReport::default();
    let mut seen_roots = HashSet::new();

    for ws in project_workspaces {
        if !ws.is_dir() || !is_inside_work_tree(ws) {
            continue;
        }
        let Ok(toplevel) = show_toplevel(ws) else {
            continue;
        };
        let root = worktrees_root_for_repo(&toplevel);
        let root_key = canonicalize_lossy(&root);
        if !seen_roots.insert(root_key) {
            continue;
        }
        if !root.is_dir() {
            continue;
        }
        report.scanned_roots += 1;

        let entries = match std::fs::read_dir(&root) {
            Ok(e) => e,
            Err(e) => {
                report
                    .errors
                    .push(format!("read_dir {}: {e}", root.display()));
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let canon = canonicalize_lossy(&path);
            if registered.contains(&canon) {
                continue;
            }
            // Only prune Minos-managed linked worktrees (dir is a git worktree).
            if !is_inside_work_tree(&path) {
                continue;
            }
            match remove_conversation_worktree(&toplevel, &path) {
                Ok(()) => {
                    report.pruned += 1;
                    tracing::info!(
                        target: "minos_daemon::git",
                        path = %path.display(),
                        "pruned orphan conversation worktree"
                    );
                }
                Err(e) => {
                    report.errors.push(format!("{}: {e}", path.display()));
                }
            }
        }

        // Drop stale worktree admin files for already-deleted paths.
        let _ = run_git(&toplevel, &["worktree", "prune"]);
    }

    report
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

    #[test]
    fn prune_removes_unregistered_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_ok(&repo, &["init", "-b", "main"]);
        git_ok(&repo, &["config", "user.email", "test@example.com"]);
        git_ok(&repo, &["config", "user.name", "test"]);
        std::fs::write(repo.join("README"), "x").unwrap();
        git_ok(&repo, &["add", "README"]);
        git_ok(&repo, &["commit", "-m", "init"]);

        let orphan = create_conversation_worktree(&repo, "conv-orphan01", "Orphan")
            .expect("orphan wt");
        assert!(orphan.path.is_dir());

        let report = prune_orphan_worktrees(&[], &[repo.clone()]);
        assert_eq!(report.pruned, 1);
        assert!(!orphan.path.exists());
    }

    #[test]
    fn prune_keeps_registered_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_ok(&repo, &["init", "-b", "main"]);
        git_ok(&repo, &["config", "user.email", "test@example.com"]);
        git_ok(&repo, &["config", "user.name", "test"]);
        std::fs::write(repo.join("README"), "x").unwrap();
        git_ok(&repo, &["add", "README"]);
        git_ok(&repo, &["commit", "-m", "init"]);

        let kept = create_conversation_worktree(&repo, "conv-kept0001", "Kept").expect("wt");
        let report = prune_orphan_worktrees(&[kept.path.clone()], &[repo]);
        assert_eq!(report.pruned, 0);
        assert!(kept.path.is_dir());
    }
}
