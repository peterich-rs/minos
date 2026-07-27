//! Diff helpers for conversation work units.

use std::path::Path;

use super::exec::run_git;

const MAX_DIFF_BYTES: usize = 256 * 1024;
const MAX_FILES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub status: String,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffResult {
    pub base: String,
    pub head: String,
    pub files: Vec<DiffFile>,
    pub patch: String,
    pub truncated: bool,
    pub file_count: u32,
}

/// Validate a git revision name passed as a bare CLI arg.
///
/// Rejects option-like strings (`-foo`) and characters outside the usual rev
/// charset so callers cannot inject extra `git` options.
pub fn validate_rev_name(rev: &str) -> Result<(), String> {
    let rev = rev.trim();
    if rev.is_empty() {
        return Err("revision name must not be empty".into());
    }
    if rev == "WORKTREE" || rev == "worktree" {
        return Ok(());
    }
    if rev.starts_with('-') {
        return Err(format!("invalid revision name (leading dash): {rev}"));
    }
    if rev.len() > 256 {
        return Err("revision name too long".into());
    }
    let ok = rev.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '.' | '_' | '/' | '-' | '~' | '^' | '{' | '}' | '@' | '+')
    });
    if !ok {
        return Err(format!("invalid revision name: {rev}"));
    }
    Ok(())
}

/// Diff via git's three-dot form `base...head` (symmetric difference from
/// merge-base of base and head to head). This is the usual PR-style range,
/// not the two-dot `base..head` history walk.
///
/// When `head` is empty / `"WORKTREE"`, diffs the working tree against `base`.
pub fn get_diff(repo: &Path, base: Option<&str>, head: Option<&str>) -> Result<DiffResult, String> {
    let base = base
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("HEAD")
        .to_owned();
    validate_rev_name(&base)?;
    let head = head.map(str::trim).filter(|s| !s.is_empty());
    if let Some(h) = head {
        validate_rev_name(h)?;
    }

    let name_status = match head {
        None | Some("WORKTREE") | Some("worktree") => {
            run_git(repo, &["diff", "--name-status", &base])?
        }
        Some(h) => run_git(repo, &["diff", "--name-status", &format!("{base}...{h}")])?,
    };

    let mut files = Vec::new();
    for line in name_status.lines().take(MAX_FILES) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let status = parts.next().unwrap_or("M").to_owned();
        let path = parts.next().unwrap_or("").to_owned();
        if path.is_empty() {
            continue;
        }
        files.push(DiffFile {
            path,
            status,
            patch: String::new(),
            truncated: false,
        });
    }

    let full_patch = match head {
        None | Some("WORKTREE") | Some("worktree") => {
            run_git(repo, &["diff", "--no-ext-diff", &base]).unwrap_or_default()
        }
        Some(h) => {
            run_git(repo, &["diff", "--no-ext-diff", &format!("{base}...{h}")]).unwrap_or_default()
        }
    };

    let truncated = full_patch.len() > MAX_DIFF_BYTES;
    let patch = if truncated {
        let mut end = MAX_DIFF_BYTES;
        while end > 0 && !full_patch.is_char_boundary(end) {
            end -= 1;
        }
        full_patch[..end].to_owned()
    } else {
        full_patch
    };

    // Attach a bounded per-file patch slice when cheap.
    for file in &mut files {
        let marker = format!("diff --git a/{} b/{}", file.path, file.path);
        if let Some(start) = patch.find(&marker) {
            let rest = &patch[start..];
            let next = rest[marker.len()..]
                .find("\ndiff --git ")
                .map(|i| marker.len() + i + 1)
                .unwrap_or(rest.len());
            let slice = &rest[..next];
            if slice.len() > 32 * 1024 {
                let mut end = 32 * 1024;
                while end > 0 && !slice.is_char_boundary(end) {
                    end -= 1;
                }
                file.patch = slice[..end].to_owned();
                file.truncated = true;
            } else {
                file.patch = slice.to_owned();
            }
        }
    }

    Ok(DiffResult {
        base,
        head: head.unwrap_or("WORKTREE").to_owned(),
        file_count: files.len() as u32,
        files,
        patch,
        truncated,
    })
}

/// Recent commit subjects on the current branch (for milestones).
pub fn recent_commit_subjects(repo: &Path, max: usize) -> Result<Vec<String>, String> {
    let max = max.clamp(1, 50);
    let n = max.to_string();
    let out = run_git(repo, &["log", &format!("-{n}"), "--pretty=format:%s"])?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rev_rejects_option_injection() {
        assert!(validate_rev_name("-upload-pack=evil").is_err());
        assert!(validate_rev_name("main").is_ok());
        assert!(validate_rev_name("abc123~1").is_ok());
        assert!(validate_rev_name("WORKTREE").is_ok());
    }
}
