//! Path completion shared across event, effect, and UI layers.
//!
//! Lives outside `ui::` so `AppEvent` / effect executors never depend on the
//! render layer for candidate types or directory listing.

use std::path::Path;

/// A single path completion candidate returned by async directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathCandidate {
    pub name: String,
    pub is_dir: bool,
}

/// List path completion candidates for a path token under `workspace_root`.
///
/// `token` is the full path prefix up to the cursor (e.g. `src/fo`). Returns
/// up to 8 case-insensitive substring matches in the resolved directory.
pub fn list_path_candidates(token: &str, workspace_root: &Path) -> Option<Vec<PathCandidate>> {
    let last_slash = token.rfind('/')?;
    let dir_prefix = &token[..=last_slash];
    let partial_name = token[last_slash + 1..].to_ascii_lowercase();

    let resolved: std::path::PathBuf = if let Some(stripped) = dir_prefix.strip_prefix("~/") {
        dirs::home_dir()?.join(stripped)
    } else if dir_prefix.starts_with('/') {
        std::path::PathBuf::from(dir_prefix)
    } else {
        workspace_root.join(dir_prefix)
    };

    let entries = std::fs::read_dir(&resolved).ok()?;
    let mut candidates: Vec<PathCandidate> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.to_ascii_lowercase().contains(partial_name.as_str()) {
                return None;
            }
            let is_dir = entry.file_type().ok()?.is_dir();
            Some(PathCandidate { name, is_dir })
        })
        .collect();
    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    candidates.truncate(8);
    if candidates.is_empty() {
        return None;
    }
    Some(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn list_path_candidates_matches_partial_name() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/foo.rs"), "fn main() {}").expect("write");
        fs::write(root.join("src/bar.rs"), "").expect("write");

        let candidates = list_path_candidates("src/fo", root).expect("candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "foo.rs");
        assert!(!candidates[0].is_dir);
    }
}
