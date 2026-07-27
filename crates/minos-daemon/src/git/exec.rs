//! Safe, path-scoped git subprocess helpers.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Run `git` in `cwd` with fixed args. Returns stdout (trimmed) on success.
pub fn run_git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("git {:?} exited with status {}", args, output.status)
        };
        return Err(detail);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// True when `path` is inside a git work tree.
pub fn is_inside_work_tree(path: &Path) -> bool {
    run_git(path, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Absolute toplevel of the work tree containing `path`.
pub fn show_toplevel(path: &Path) -> Result<PathBuf, String> {
    let raw = run_git(path, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(raw))
}

/// Common git dir (handles linked worktrees).
pub fn git_common_dir(path: &Path) -> Result<PathBuf, String> {
    let raw = run_git(path, &["rev-parse", "--git-common-dir"])?;
    let p = PathBuf::from(&raw);
    if p.is_absolute() {
        Ok(p)
    } else {
        Ok(path.join(p))
    }
}

pub fn first_line(output: &str) -> Option<String> {
    output
        .lines()
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
