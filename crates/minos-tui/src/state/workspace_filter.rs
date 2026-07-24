//! Workspace filtering helpers.
//!
//! Path comparisons canonicalize when needed, but never re-canonicalize the same
//! path repeatedly inside one prune/match pass (that used to stall the main
//! loop every daemon list_sessions tick).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ui::UiState;

use super::AppState;

pub(crate) fn workspace_path_belongs_to_current_workspace(
    workspace: &Path,
    candidate: &Path,
) -> bool {
    workspace_paths_match(workspace, candidate)
}

pub(crate) fn prune_external_threads(state: &mut AppState, ui: &mut UiState) -> bool {
    let selected_session_id = ui.current_session_id().map(str::to_owned);
    let mut removed_session_ids = Vec::new();
    let mut matcher = WorkspaceMatcher::from_state(state, ui);

    ui.session_panel.list.items.retain(|thread| {
        let keep = matcher.contains(&thread.workspace);
        if !keep {
            removed_session_ids.push(thread.session_id.clone());
        }
        keep
    });

    if removed_session_ids.is_empty() {
        return false;
    }

    for session_id in &removed_session_ids {
        ui.session_panel.chat_states.remove(session_id);
        state.hydrated_threads.remove(session_id);
        state.session_watermarks.remove(session_id);
        state.recorded_agent_results.remove(session_id);
        state.session_conversations.remove(session_id);
    }

    let next = selected_session_id
        .and_then(|session_id| {
            ui.session_panel
                .list
                .items
                .iter()
                .position(|thread| thread.session_id == session_id)
        })
        .or_else(|| (!ui.session_panel.list.items.is_empty()).then_some(0));
    ui.session_panel.list.select(next);
    true
}

pub(crate) fn remove_thread_local_state(
    state: &mut AppState,
    ui: &mut UiState,
    session_id: &str,
) -> bool {
    let Some(index) = ui
        .session_panel
        .list
        .items
        .iter()
        .position(|thread| thread.session_id == session_id)
    else {
        return false;
    };

    ui.session_panel.list.items.remove(index);
    ui.session_panel.chat_states.remove(session_id);
    state.hydrated_threads.remove(session_id);
    state.session_watermarks.remove(session_id);
    state.recorded_agent_results.remove(session_id);
    state.session_conversations.remove(session_id);

    let next = ui
        .session_panel
        .list
        .selected
        .and_then(|selected| match selected.cmp(&index) {
            std::cmp::Ordering::Less => Some(selected),
            std::cmp::Ordering::Equal => (!ui.session_panel.list.items.is_empty())
                .then_some(index.min(ui.session_panel.list.items.len() - 1)),
            std::cmp::Ordering::Greater => Some(selected - 1),
        });
    ui.session_panel.list.select(next);
    true
}

pub(crate) fn workspace_path_belongs_to_known_workspace(
    state: &AppState,
    ui: &UiState,
    candidate: &Path,
) -> bool {
    WorkspaceMatcher::from_state(state, ui).contains(candidate)
}

/// Pre-normalized known workspaces + per-call canonicalize cache.
///
/// Built once per prune/apply so matching is O(sessions + workspaces) syscalls,
/// not O(sessions × workspaces).
pub(crate) struct WorkspaceMatcher {
    known: HashSet<PathBuf>,
    cache: PathNormCache,
}

impl WorkspaceMatcher {
    pub(crate) fn from_state(state: &AppState, ui: &UiState) -> Self {
        let mut cache = PathNormCache::default();
        let mut known = HashSet::with_capacity(ui.projects.items.len() + 1);
        known.insert(cache.normalize(&state.workspace));
        for project in &ui.projects.items {
            known.insert(cache.normalize(&project.workspace_path));
        }
        Self { known, cache }
    }

    pub(crate) fn contains(&mut self, candidate: &Path) -> bool {
        let normalized = self.cache.normalize(candidate);
        self.known.contains(&normalized)
    }
}

fn workspace_paths_match(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    let mut cache = PathNormCache::default();
    cache.normalize(a) == cache.normalize(b)
}

#[derive(Default)]
struct PathNormCache {
    map: HashMap<PathBuf, PathBuf>,
}

impl PathNormCache {
    fn normalize(&mut self, path: &Path) -> PathBuf {
        if let Some(hit) = self.map.get(path) {
            return hit.clone();
        }
        let normalized = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.map.insert(path.to_path_buf(), normalized.clone());
        // Canonical form maps to itself so reverse lookups stay O(1).
        self.map
            .entry(normalized.clone())
            .or_insert_with(|| normalized.clone());
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_paths_match_fast_path_equal_paths() {
        let a = Path::new("/tmp/same");
        let b = Path::new("/tmp/same");
        assert!(workspace_paths_match(a, b));
    }

    #[test]
    fn path_norm_cache_reuses_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        let mut cache = PathNormCache::default();
        let first = cache.normalize(&path);
        let second = cache.normalize(&path);
        assert_eq!(first, second);
        // Original path key + optional self-entry for the canonical form.
        assert!(cache.map.len() <= 2);
        let before = cache.map.len();
        let _ = cache.normalize(&path);
        assert_eq!(cache.map.len(), before);
    }

    #[test]
    fn matcher_contains_uses_known_set_not_nested_scan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path().to_path_buf();
        let mut cache = PathNormCache::default();
        let mut known = HashSet::new();
        known.insert(cache.normalize(&workspace));
        let mut matcher = WorkspaceMatcher { known, cache };
        assert!(matcher.contains(&workspace));
        assert!(!matcher.contains(Path::new("/definitely/not/this/workspace")));
    }

    /// Simulate sessions×workspaces comparisons: cache must stay O(unique paths).
    #[test]
    fn repeated_normalize_of_same_path_is_cached() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ws");
        std::fs::create_dir_all(&path).expect("mkdir");
        let mut cache = PathNormCache::default();
        for _ in 0..50 {
            for _ in 0..10 {
                let _ = cache.normalize(&path);
            }
        }
        assert!(cache.map.len() <= 2);
    }
}
