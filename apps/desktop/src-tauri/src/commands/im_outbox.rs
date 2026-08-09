//! Tauri commands for durable Desktop IM outbox (SQLite).

use crate::app_state::AppState;
use crate::im_outbox_store::ImOutboxEntryDto;
use tauri::State;

#[tauri::command]
pub fn im_outbox_list_all(state: State<'_, AppState>) -> Result<Vec<ImOutboxEntryDto>, String> {
    state
        .im_outbox
        .list_all()
        .map_err(|e| format!("im_outbox_list_all: {e:#}"))
}

#[tauri::command]
pub fn im_outbox_replace_all(
    state: State<'_, AppState>,
    entries: Vec<ImOutboxEntryDto>,
) -> Result<(), String> {
    state
        .im_outbox
        .replace_all(&entries)
        .map_err(|e| format!("im_outbox_replace_all: {e:#}"))
}

#[tauri::command]
pub fn im_outbox_upsert(state: State<'_, AppState>, entry: ImOutboxEntryDto) -> Result<(), String> {
    state
        .im_outbox
        .upsert(&entry)
        .map_err(|e| format!("im_outbox_upsert: {e:#}"))
}

#[tauri::command]
pub fn im_outbox_clear(state: State<'_, AppState>) -> Result<(), String> {
    state
        .im_outbox
        .clear()
        .map_err(|e| format!("im_outbox_clear: {e:#}"))
}
