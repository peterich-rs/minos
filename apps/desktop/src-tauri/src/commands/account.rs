//! Account session + Account IM WS commands.

use minos_protocol::realtime::ClientFrame;
use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::account_cloud;
use crate::app_state::AppState;
use crate::identity::{self, DesktopAccount};

#[tauri::command]
pub fn account_device_id() -> Result<String, String> {
    identity::device_id()
        .map(|id| id.to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn account_load_persisted() -> Result<Option<DesktopAccount>, String> {
    let device_id = identity::device_id().map_err(|e| e.to_string())?;
    let Some(refresh) = identity::load_refresh_token().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let Some(account_id) = identity::load_account_id().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    Ok(Some(DesktopAccount {
        device_id: device_id.to_string(),
        account_id,
        email: identity::load_email()
            .map_err(|e| e.to_string())?
            .unwrap_or_default(),
        access_token: String::new(),
        refresh_token: refresh,
        expires_in: 0,
        host_token: identity::load_host_token().map_err(|e| e.to_string())?,
    }))
}

#[tauri::command]
pub async fn account_exchange_supabase(
    state: State<'_, AppState>,
    supabase_access_token: String,
) -> Result<DesktopAccount, String> {
    let device_id = identity::device_id().map_err(|e| e.to_string())?;
    let session = account_cloud::exchange_supabase(device_id, &supabase_access_token)
        .await
        .map_err(|e| e.to_string())?;
    identity::persist_session(&session).map_err(|e| e.to_string())?;
    apply_host_token(&state, session.host_token.as_deref()).await?;
    Ok(session)
}

#[tauri::command]
pub async fn account_refresh_session(
    state: State<'_, AppState>,
    refresh_token: String,
) -> Result<DesktopAccount, String> {
    let device_id = identity::device_id().map_err(|e| e.to_string())?;
    let session = account_cloud::refresh(device_id, &refresh_token)
        .await
        .map_err(|e| e.to_string())?;
    identity::persist_session(&session).map_err(|e| e.to_string())?;
    apply_host_token(&state, session.host_token.as_deref()).await?;
    Ok(session)
}

#[tauri::command]
pub async fn account_sign_out(
    app: AppHandle,
    state: State<'_, AppState>,
    access_token: String,
    refresh_token: String,
) -> Result<(), String> {
    let device_id = identity::device_id().map_err(|e| e.to_string())?;
    let _ = account_cloud::logout(device_id, &access_token, &refresh_token).await;
    state.account_ws.stop().await;
    crate::account_realtime::clear_cursors();
    identity::clear_session().map_err(|e| e.to_string())?;
    let _ = state.daemon.host_clear_credential().await;
    let _ = app;
    Ok(())
}

#[tauri::command]
pub async fn account_ws_start(
    app: AppHandle,
    state: State<'_, AppState>,
    access_token: String,
    account_id: String,
) -> Result<(), String> {
    state
        .account_ws
        .start(app, access_token, account_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn account_ws_stop(state: State<'_, AppState>) -> Result<(), String> {
    state.account_ws.stop().await;
    Ok(())
}

#[tauri::command]
pub async fn account_ws_subscribe(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    state
        .account_ws
        .subscribe_conversation(conversation_id)
        .await;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendMessageArgs {
    pub client_operation_id: String,
    pub conversation_id: String,
    pub text: String,
    pub reply_to_message_id: Option<String>,
    pub mentions: Option<serde_json::Value>,
}

#[tauri::command]
pub async fn account_ws_append(
    state: State<'_, AppState>,
    args: AppendMessageArgs,
) -> Result<(), String> {
    let mentions = args
        .mentions
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    state
        .account_ws
        .send(ClientFrame::AppendMessage {
            client_operation_id: args.client_operation_id,
            conversation_id: args.conversation_id,
            text: args.text,
            mentions,
            reply_to_message_id: args.reply_to_message_id,
            attachment_ids: Vec::new(),
        })
        .await
        .map_err(|e| e.to_string())
}

async fn apply_host_token(state: &AppState, host_token: Option<&str>) -> Result<(), String> {
    let Some(token) = host_token.filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    if let Err(error) = state.daemon.host_apply_link_token(token.to_string()).await {
        tracing::warn!(
            target: "minos_desktop::account",
            %error,
            "daemon not ready to apply host_token; file store updated"
        );
    }
    Ok(())
}
