//! Tauri commands for same-account Host Link (D02).

use crate::app_state::AppState;
use crate::daemon::{
    HostApplyLinkTokenDto, HostClearCredentialDto, HostPrepareLinkDto, HostSignLinkProofDto,
};
use tauri::State;

#[tauri::command]
pub async fn daemon_host_prepare_link(
    state: State<'_, AppState>,
) -> Result<HostPrepareLinkDto, String> {
    state
        .daemon
        .host_prepare_link()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_host_sign_link_proof(
    state: State<'_, AppState>,
    installation_id: String,
    nonce: String,
) -> Result<HostSignLinkProofDto, String> {
    state
        .daemon
        .host_sign_link_proof(installation_id, nonce)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_host_apply_link_token(
    state: State<'_, AppState>,
    host_installation_token: String,
) -> Result<HostApplyLinkTokenDto, String> {
    state
        .daemon
        .host_apply_link_token(host_installation_token)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_host_clear_credential(
    state: State<'_, AppState>,
) -> Result<HostClearCredentialDto, String> {
    state
        .daemon
        .host_clear_credential()
        .await
        .map_err(|e| e.to_string())
}
