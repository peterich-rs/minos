//! Workspace filtering helpers.

use std::path::{Path, PathBuf};

use minos_protocol::LocalGroupChatMessage;

use crate::ui::UiState;

use super::AppState;

pub(crate) fn workspace_path_belongs_to_current_workspace(
    workspace: &Path,
    candidate: &Path,
) -> bool {
    workspace_paths_match(workspace, candidate)
}

pub(crate) fn group_message_belongs_to_current_workspace(
    workspace: &Path,
    message: &LocalGroupChatMessage,
) -> bool {
    message.workspace.as_deref().is_some_and(|candidate| {
        workspace_path_belongs_to_current_workspace(workspace, Path::new(candidate))
    })
}

pub(crate) fn filter_group_messages_for_current_workspace(
    workspace: &Path,
    messages: Vec<LocalGroupChatMessage>,
) -> Vec<LocalGroupChatMessage> {
    messages
        .into_iter()
        .filter(|message| group_message_belongs_to_current_workspace(workspace, message))
        .collect()
}

pub(crate) fn prune_external_threads(state: &mut AppState, ui: &mut UiState) -> bool {
    let selected_thread_id = ui.current_thread_id().map(str::to_owned);
    let mut removed_thread_ids = Vec::new();
    let current_workspace = state.workspace.clone();

    ui.threads.retain(|thread| {
        let keep = workspace_paths_match(&current_workspace, &thread.workspace);
        if !keep {
            removed_thread_ids.push(thread.thread_id.clone());
        }
        keep
    });

    if removed_thread_ids.is_empty() {
        return false;
    }

    for thread_id in &removed_thread_ids {
        ui.chat_states.remove(thread_id);
        state.hydrated_threads.remove(thread_id);
        state.thread_watermarks.remove(thread_id);
        state.recorded_agent_results.remove(thread_id);
    }

    ui.selected_thread = selected_thread_id
        .and_then(|thread_id| {
            ui.threads
                .iter()
                .position(|thread| thread.thread_id == thread_id)
        })
        .or_else(|| (!ui.threads.is_empty()).then_some(0));
    ui.agent_list_state.select(ui.selected_thread);
    true
}

pub(crate) fn remove_thread_local_state(
    state: &mut AppState,
    ui: &mut UiState,
    thread_id: &str,
) -> bool {
    let Some(index) = ui
        .threads
        .iter()
        .position(|thread| thread.thread_id == thread_id)
    else {
        return false;
    };

    ui.threads.remove(index);
    ui.chat_states.remove(thread_id);
    state.hydrated_threads.remove(thread_id);
    state.thread_watermarks.remove(thread_id);
    state.recorded_agent_results.remove(thread_id);

    ui.selected_thread = ui
        .selected_thread
        .and_then(|selected| match selected.cmp(&index) {
            std::cmp::Ordering::Less => Some(selected),
            std::cmp::Ordering::Equal => {
                (!ui.threads.is_empty()).then_some(index.min(ui.threads.len() - 1))
            }
            std::cmp::Ordering::Greater => Some(selected - 1),
        });
    ui.agent_list_state.select(ui.selected_thread);
    true
}

fn workspace_paths_match(a: &Path, b: &Path) -> bool {
    normalized_workspace_path(a) == normalized_workspace_path(b)
}

fn normalized_workspace_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
