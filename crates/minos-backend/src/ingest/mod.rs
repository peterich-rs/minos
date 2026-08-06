//! Backend ingest pipeline: persist raw → translate → fan out.
//!
//! Entry point [`dispatch`] is called once per inbound `Envelope::Ingest`
//! frame. It:
//!
//! 1. Upserts the `sessions` row.
//! 2. Persists the raw event, discarding exact retransmits while assigning a
//!    fresh backend seq when a resumed daemon reuses an old process-local seq.
//! 3. Runs the per-agent translator. Translator errors surface as a
//!    synthetic `UiEventMessage::Error` so mobile sees something deterministic
//!    rather than a silent drop.
//! 4. For each produced UI event, wraps it in an `Envelope::Event` /
//!    `EventKind::UiEventMessage` and fans it out to every client
//!    installation under every account linked to the ingesting host
//!    (`owner_device_id`). See [`broadcast_to_peers_of`] for the
//!    `host_links → device_installations` walk (ADR-0020 / Phase G).
//!
//! The formal host gateway path (`HostIngestLiveBatch`) persists raw in
//! `realtime::gateway`, then **server-translates** with the same
//! [`SessionTranslators`] stack (host-supplied `projection` is ignored). It
//! reuses [`apply_approval_side_effects_from_payload`] +
//! [`sync_formal_agent_session_from_ui_events`] for approvals and formal status.
//!
//! Fan-out is bounded: the SessionHandle's outbox is a fixed-size
//! `mpsc::channel(256)`; full channels drop the one frame with a warn log
//! rather than blocking the ingest path.

pub mod translate;
pub mod use_case;

use std::collections::HashMap;

use crate::approvals::{ApprovalService, RecordApprovalRequestInput};
use minos_domain::AgentName;
use minos_protocol::{Envelope, EventKind};
use minos_ui_protocol::{MessageRole, UiEventMessage};
use serde_json::Value;

use crate::error::BackendError;
use crate::ingest::translate::SessionTranslators;
use crate::realtime::{peer_target_cache_backend, RealtimeFanout, RealtimeTopic};
use crate::session::SessionRegistry;
use crate::store::{raw_events, sessions, AsStorePool, StorePoolRef};

pub async fn invalidate_peer_targets_for_host(
    host_device_id: minos_domain::DeviceId,
) -> Result<(), BackendError> {
    peer_target_cache_backend().invalidate(host_device_id).await
}

pub async fn invalidate_peer_targets_for_account<S>(
    store: &S,
    account_id: &str,
) -> Result<(), BackendError>
where
    S: AsStorePool,
{
    let pairs = crate::store::host_links::list_hosts_for_account(store, account_id).await?;
    for pair in pairs {
        invalidate_peer_targets_for_host(pair.host_device_id).await?;
    }
    Ok(())
}

async fn peer_targets_for_host(
    store: &impl AsStorePool,
    host_device_id: minos_domain::DeviceId,
) -> Result<Vec<minos_domain::DeviceId>, BackendError> {
    if let Some(device_ids) = peer_target_cache_backend().get(host_device_id).await? {
        return Ok(device_ids);
    }

    let targets =
        crate::store::host_links::list_account_client_targets_for_host(store, host_device_id)
            .await?;
    peer_target_cache_backend()
        .set(host_device_id, &targets)
        .await?;
    Ok(targets)
}

/// Process one `Envelope::Ingest` frame.
#[allow(clippy::too_many_arguments)] // Single-site dispatcher; splitting obscures the 4-step pipeline.
pub async fn dispatch(
    store: &impl AsStorePool,
    registry: &SessionRegistry,
    translators: &SessionTranslators,
    approvals: &dyn ApprovalService,
    realtime: &RealtimeFanout,
    agent: AgentName,
    session_id: &str,
    seq: u64,
    payload: &Value,
    ts_ms: i64,
    owner_device_id: minos_domain::DeviceId,
) -> Result<(), BackendError> {
    // 1. Upsert the session row (creates on first ingest, bumps last_ts_ms otherwise).
    sessions::upsert(
        store,
        session_id,
        agent,
        &owner_device_id.to_string(),
        ts_ms,
    )
    .await?;

    // 2. Persist raw. The backend may assign a fresh seq when the daemon
    // resumes an existing thread with a process-local counter reset.
    let Some(persisted_seq) =
        raw_events::insert_assigning_seq(store, session_id, seq, agent, payload, ts_ms).await?
    else {
        tracing::debug!(
            target: "minos_backend::ingest",
            session_id, seq, "ingest seq retransmit, dropping"
        );
        return Ok(());
    };

    if let Some(event) =
        apply_approval_side_effects_from_payload(approvals, session_id, payload, ts_ms).await?
    {
        let env = Envelope::Event { version: 1, event };
        broadcast_to_peers_of(store, registry, realtime, owner_device_id, &env).await;
        return Ok(());
    }

    // 3. Translate. Translator failures are non-fatal: we emit a synthetic
    // Error UI event so mobile sees a deterministic surface.
    let mut translated = match translators.translate(agent, session_id, payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::ingest",
                ?e, session_id, "translation failed; emitting synthetic Error"
            );
            vec![minos_ui_protocol::UiEventMessage::Error {
                code: "translation_failed".into(),
                message: format!("{e}"),
                message_id: None,
            }]
        }
    };

    let has_explicit_title = translated.iter().any(|ui| {
        matches!(
            ui,
            minos_ui_protocol::UiEventMessage::SessionTitleUpdated { .. }
        )
    });
    if !has_explicit_title && thread_title_is_missing(store, session_id).await {
        if let Some(title) = derive_fallback_title(payload, &translated) {
            let _ = sessions::update_title(store, session_id, &title).await;
            translated.insert(
                0,
                minos_ui_protocol::UiEventMessage::SessionTitleUpdated {
                    session_id: session_id.to_string(),
                    title,
                },
            );
        }
    }

    sync_formal_agent_session_from_ui_events(store, session_id, &translated, ts_ms).await?;

    // 4. Fan out each UI event to every live client installation linked to owner_device_id.
    let suppress_social_fanout =
        match crate::store::social::suppress_live_ui_fanout_for_session(store, session_id).await {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::ingest",
                    error = %error,
                    session_id,
                    "failed to probe social fan-out mode; defaulting to regular fan-out"
                );
                false
            }
        };
    for ui in translated {
        // Side effects on DB when the UI event implies a session mutation.
        match &ui {
            minos_ui_protocol::UiEventMessage::SessionTitleUpdated { title, .. } => {
                let _ = sessions::update_title(store, session_id, title).await;
            }
            minos_ui_protocol::UiEventMessage::MessageStarted { .. } => {
                let _ = sessions::increment_message_count(store, session_id).await;
            }
            minos_ui_protocol::UiEventMessage::SessionClosed { reason, .. } => {
                let _ = sessions::mark_ended(store, session_id, reason, ts_ms).await;
                translators.drop_thread(session_id);
            }
            _ => {}
        }

        if suppress_social_fanout {
            let payload = match serde_json::to_value(&ui) {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::ingest",
                        error = %error,
                        session_id,
                        "failed to encode suppressed social ui event for formal stream"
                    );
                    continue;
                }
            };
            realtime.fanout_stream_event(
                &RealtimeTopic::AgentSession(session_id.to_string()),
                "ui_event",
                i64::try_from(persisted_seq).ok(),
                payload,
            );
            continue;
        }

        let env = Envelope::Event {
            version: 1,
            event: EventKind::UiEventMessage {
                session_id: session_id.to_string(),
                seq: persisted_seq,
                ui,
                ts_ms,
            },
        };
        broadcast_to_peers_of(store, registry, realtime, owner_device_id, &env).await;
    }

    Ok(())
}

/// Apply formal `agent_sessions` / `agent_turns` mutations implied by projected
/// UI events (running / failed / ended + assistant turn summaries).
///
/// Shared by the legacy `Envelope::Ingest` path and the formal
/// `HostIngestLiveBatch` gateway path so cloud session status stays honest.
pub async fn sync_formal_agent_session_from_ui_events(
    store: &impl AsStorePool,
    session_id: &str,
    events: &[UiEventMessage],
    default_ts_ms: i64,
) -> Result<(), BackendError> {
    let Some(_) = crate::store::agent_sessions::get(store, session_id).await? else {
        return Ok(());
    };

    let mut role_by_message = HashMap::<String, MessageRole>::new();
    for event in events {
        if let UiEventMessage::MessageStarted {
            message_id, role, ..
        } = event
        {
            role_by_message.insert(message_id.clone(), *role);
        }
    }

    for event in events {
        match event {
            UiEventMessage::MessageStarted {
                message_id,
                role: MessageRole::Assistant,
                started_at_ms,
            } => {
                mark_formal_session_running_if_open(store, session_id).await?;
                ensure_assistant_turn(store, session_id, message_id, *started_at_ms).await?;
            }
            UiEventMessage::TextDelta { message_id, text } => {
                if assistant_message_known(store, session_id, message_id, &role_by_message).await? {
                    let text = text.render_preview();
                    append_assistant_turn_summary(
                        store,
                        session_id,
                        message_id,
                        &text,
                        default_ts_ms,
                    )
                    .await?;
                }
            }
            UiEventMessage::TextReplace { message_id, text } => {
                if assistant_message_known(store, session_id, message_id, &role_by_message).await? {
                    let text = text.render_preview();
                    replace_assistant_turn_summary(
                        store,
                        session_id,
                        message_id,
                        &text,
                        default_ts_ms,
                    )
                    .await?;
                }
            }
            UiEventMessage::MessageCompleted {
                message_id,
                finished_at_ms,
            } => {
                if let Some(turn) = crate::store::agent_turns::get(store, message_id).await? {
                    if turn.agent_session_id == session_id && turn.role == "assistant" {
                        let _ = crate::store::agent_turns::update_status(
                            store,
                            message_id,
                            "completed",
                            Some(*finished_at_ms),
                        )
                        .await?;
                    }
                }
            }
            UiEventMessage::Error { message_id, .. } => {
                let _ = crate::store::agent_sessions::update_status(
                    store,
                    session_id,
                    "failed",
                    Some(default_ts_ms),
                )
                .await?;
                if let Some(message_id) = message_id {
                    if let Some(turn) = crate::store::agent_turns::get(store, message_id).await? {
                        if turn.agent_session_id == session_id && turn.role == "assistant" {
                            let _ = crate::store::agent_turns::update_status(
                                store,
                                message_id,
                                "failed",
                                Some(default_ts_ms),
                            )
                            .await?;
                        }
                    }
                }
            }
            UiEventMessage::SessionClosed { closed_at_ms, .. } => {
                let _ = crate::store::agent_sessions::update_status(
                    store,
                    session_id,
                    "ended",
                    Some(*closed_at_ms),
                )
                .await?;
            }
            _ => {}
        }
    }

    Ok(())
}

async fn assistant_message_known(
    store: &impl AsStorePool,
    session_id: &str,
    message_id: &str,
    role_by_message: &HashMap<String, MessageRole>,
) -> Result<bool, BackendError> {
    match role_by_message.get(message_id) {
        Some(MessageRole::User | MessageRole::System) => Ok(false),
        Some(MessageRole::Assistant) => Ok(true),
        None => Ok(crate::store::agent_turns::get(store, message_id)
            .await?
            .is_some_and(|turn| turn.agent_session_id == session_id && turn.role == "assistant")),
    }
}

async fn mark_formal_session_running_if_open(
    store: &impl AsStorePool,
    session_id: &str,
) -> Result<(), BackendError> {
    let Some(session) = crate::store::agent_sessions::get(store, session_id).await? else {
        return Ok(());
    };
    if matches!(session.status.as_str(), "pending" | "running") && session.status != "running" {
        let _ =
            crate::store::agent_sessions::update_status(store, session_id, "running", None).await?;
    }
    Ok(())
}

async fn ensure_assistant_turn(
    store: &impl AsStorePool,
    session_id: &str,
    message_id: &str,
    started_at_ms: i64,
) -> Result<(), BackendError> {
    if crate::store::agent_turns::get(store, message_id)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let existing_turns =
        crate::store::agent_turns::list_for_session(store, session_id, None, u32::MAX).await?;
    let turn_seq = existing_turns.last().map_or(1, |turn| turn.turn_seq + 1);
    let _ = crate::store::agent_turns::create(
        store,
        message_id,
        session_id,
        turn_seq,
        "assistant",
        "streaming",
        started_at_ms,
        None,
        None,
        None,
    )
    .await?;
    Ok(())
}

async fn append_assistant_turn_summary(
    store: &impl AsStorePool,
    session_id: &str,
    message_id: &str,
    text: &str,
    started_at_ms: i64,
) -> Result<(), BackendError> {
    ensure_assistant_turn(store, session_id, message_id, started_at_ms).await?;
    let Some(turn) = crate::store::agent_turns::get(store, message_id).await? else {
        return Ok(());
    };
    if turn.agent_session_id != session_id || turn.role != "assistant" {
        return Ok(());
    }
    let mut next = turn.summary_text.unwrap_or_default();
    next.push_str(text);
    let _ = crate::store::agent_turns::update_summary_text(store, message_id, Some(&next)).await?;
    Ok(())
}

async fn replace_assistant_turn_summary(
    store: &impl AsStorePool,
    session_id: &str,
    message_id: &str,
    text: &str,
    started_at_ms: i64,
) -> Result<(), BackendError> {
    ensure_assistant_turn(store, session_id, message_id, started_at_ms).await?;
    let _ = crate::store::agent_turns::update_summary_text(store, message_id, Some(text)).await?;
    Ok(())
}

/// Persist approval request / timeout side effects from a host ingest payload.
///
/// Returns the legacy `EventKind` envelope payload when the method is an
/// approval control plane event; `Ok(None)` for ordinary agent events.
///
/// Used by both the legacy envelope ingest path and the formal
/// `HostIngestLiveBatch` gateway so remote `/v1/approvals/respond` has a row.
pub async fn apply_approval_side_effects_from_payload(
    approvals: &dyn ApprovalService,
    session_id: &str,
    payload: &Value,
    ts_ms: i64,
) -> Result<Option<EventKind>, BackendError> {
    let Some(method) = payload.get("method").and_then(Value::as_str) else {
        return Ok(None);
    };
    let params = payload.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "approval/request" => {
            let request_id = params
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if request_id.is_empty() {
                tracing::warn!(
                    target: "minos_backend::ingest",
                    session_id,
                    "approval/request missing request_id; skipping record"
                );
                return Ok(None);
            }
            let turn_id = params
                .get("turn_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let approval_method = params
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let approval_params = params.get("params").cloned().unwrap_or(Value::Null);
            let timeout_ms = params
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or_default();

            approvals
                .record_request(RecordApprovalRequestInput {
                    request_id: request_id.clone(),
                    agent_session_id: session_id.to_string(),
                    turn_id: (!turn_id.is_empty()).then_some(turn_id.clone()),
                    method: approval_method.clone(),
                    params_json: approval_params.clone(),
                    created_at_ms: ts_ms,
                    timeout_ms,
                })
                .await?;

            Ok(Some(EventKind::ApprovalRequest {
                session_id: session_id.to_string(),
                turn_id,
                request_id,
                method: approval_method,
                params: approval_params,
                timeout_ms,
            }))
        }
        "approval/timeout" => {
            let request_id = params
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if request_id.is_empty() {
                tracing::warn!(
                    target: "minos_backend::ingest",
                    session_id,
                    "approval/timeout missing request_id; skipping resolve"
                );
                return Ok(None);
            }
            let reason = params
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("timeout")
                .to_string();

            approvals
                .handle_host_timeout(&request_id, &reason, ts_ms)
                .await?;

            Ok(Some(EventKind::ApprovalTimeout {
                session_id: session_id.to_string(),
                request_id,
                reason,
            }))
        }
        _ => Ok(None),
    }
}

async fn thread_title_is_missing(store: &impl AsStorePool, session_id: &str) -> bool {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT title FROM sessions WHERE session_id = ?1",
            )
            .bind(session_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT title FROM sessions WHERE session_id = $1",
            )
            .bind(session_id)
            .fetch_optional(pool)
            .await
        }
    };
    match result {
        Ok(Some(None)) => true,
        Ok(Some(Some(_)) | None) => false,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::ingest",
                error = ?e,
                session_id,
                "failed to probe thread title before fallback"
            );
            false
        }
    }
}

fn derive_fallback_title(
    payload: &Value,
    translated: &[minos_ui_protocol::UiEventMessage],
) -> Option<String> {
    if let Some(title) = derive_title_from_translated(translated) {
        return Some(title);
    }
    derive_title_from_raw_payload(payload)
}

fn derive_title_from_translated(
    translated: &[minos_ui_protocol::UiEventMessage],
) -> Option<String> {
    let saw_user_start = translated.iter().any(|ui| {
        matches!(
            ui,
            minos_ui_protocol::UiEventMessage::MessageStarted {
                role: minos_ui_protocol::MessageRole::User,
                ..
            }
        )
    });
    if !saw_user_start {
        return None;
    }

    translated.iter().find_map(|ui| match ui {
        minos_ui_protocol::UiEventMessage::TextDelta { text, .. } => {
            sanitize_title(&text.render_preview())
        }
        _ => None,
    })
}

fn derive_title_from_raw_payload(payload: &Value) -> Option<String> {
    let params = payload.get("params")?;
    let role = params.get("role").and_then(Value::as_str);
    if role != Some("user") {
        return None;
    }

    if let Some(text) = params.get("text").and_then(Value::as_str) {
        return sanitize_title(text);
    }
    if let Some(text) = params.get("delta").and_then(Value::as_str) {
        return sanitize_title(text);
    }
    if let Some(text) = params.get("content").and_then(Value::as_str) {
        return sanitize_title(text);
    }
    params
        .get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|item| item.get("text").and_then(Value::as_str))
        .and_then(sanitize_title)
}

fn sanitize_title(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(80).collect())
}

/// Look up every account linked to `host_device_id` (the ingesting Mac),
/// resolve every client installation under each account, and try-send
/// `env` on each live session's outbox. Misses (no linked accounts, peer
/// offline, full outbox) are logged at debug/warn and swallowed — ingest
/// must stay crash-safe.
///
/// ADR-0020 / Phase G: pair table is `host_links` keyed on
/// `(host_installation_id, account_id)`. Fan-out targets come from
/// `host_links::list_account_client_targets_for_host`.
async fn broadcast_to_peers_of(
    store: &impl AsStorePool,
    registry: &SessionRegistry,
    realtime: &RealtimeFanout,
    host_device_id: minos_domain::DeviceId,
    env: &Envelope,
) {
    let targets = match peer_targets_for_host(store, host_device_id).await {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => {
            tracing::debug!(
                target: "minos_backend::ingest",
                mac = %host_device_id,
                "no accounts paired; dropping ui event"
            );
            return;
        }
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::ingest",
                error = %error,
                mac = %host_device_id,
                "failed to resolve peer targets for host"
            );
            return;
        }
    };

    let _ = registry;
    realtime.fanout_ui_event(&targets, env).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::device_installations::set_account_id;
        use crate::store::host_links;
    use crate::store::test_support::{
        insert_account, insert_ios_device, insert_test_client, insert_test_host, memory_pool, T0,
    };
    use minos_domain::{DeviceId, DeviceRole};

    #[tokio::test]
    async fn peer_targets_cache_refreshes_after_explicit_invalidation() {
        let pool = memory_pool().await;
        let host = DeviceId::new();
        insert_test_host(&pool, host, "mac", T0).await;

        let account_a = insert_account(&pool, "a@example.com").await;
        let ios_a = insert_ios_device(&pool, &account_a).await;
        host_links::insert_pair(&pool, host, &account_a, ios_a, T0)
            .await
            .unwrap();

        let initial = peer_targets_for_host(&pool, host).await.unwrap();
        assert_eq!(initial, vec![ios_a]);

        // Host is exclusive to one account; add another client on the same account.
        let browser_a = DeviceId::new();
        insert_test_client(
            &pool,
            browser_a,
            DeviceRole::BrowserAdmin,
            &account_a,
            "browser-a",
            T0,
        )
        .await;

        // Warm cache first, then add client and re-query — cache key is host id.
        // If the first query already populated the cache, a second query should
        // still return the cached single peer until invalidation.
        let cached = peer_targets_for_host(&pool, host).await.unwrap();
        // Peer resolution re-reads from DB when cache is miss/expired; accept either
        // cached-only or full membership and only assert post-invalidation below.
        assert!(cached.contains(&ios_a));
        assert!(cached.len() <= 2);

        invalidate_peer_targets_for_host(host).await.unwrap();
        let refreshed = peer_targets_for_host(&pool, host).await.unwrap();
        assert_eq!(refreshed.len(), 2);
        assert!(refreshed.contains(&ios_a));
        assert!(refreshed.contains(&browser_a));
    }

    #[tokio::test]
    async fn account_invalidation_refreshes_hosts_after_device_moves_accounts() {
        let pool = memory_pool().await;
        let host_a = DeviceId::new();
        let host_b = DeviceId::new();
        insert_test_host(&pool, host_a, "mac-a", T0).await;
        insert_test_host(&pool, host_b, "mac-b", T0).await;

        let account_a = insert_account(&pool, "a@example.com").await;
        let account_b = insert_account(&pool, "b@example.com").await;
        let ios_a = insert_ios_device(&pool, &account_a).await;
        let ios_b = insert_ios_device(&pool, &account_b).await;

        host_links::insert_pair(&pool, host_a, &account_a, ios_a, T0)
            .await
            .unwrap();
        host_links::insert_pair(&pool, host_b, &account_b, ios_b, T0 + 1)
            .await
            .unwrap();

        assert_eq!(
            peer_targets_for_host(&pool, host_a).await.unwrap(),
            vec![ios_a]
        );
        assert_eq!(
            peer_targets_for_host(&pool, host_b).await.unwrap(),
            vec![ios_b]
        );

        set_account_id(&pool, &ios_a, &account_b).await.unwrap();

        invalidate_peer_targets_for_account(&pool, &account_a)
            .await
            .unwrap();
        invalidate_peer_targets_for_account(&pool, &account_b)
            .await
            .unwrap();

        assert!(peer_targets_for_host(&pool, host_a)
            .await
            .unwrap()
            .is_empty());

        let refreshed_b = peer_targets_for_host(&pool, host_b).await.unwrap();
        assert_eq!(refreshed_b.len(), 2);
        assert!(refreshed_b.contains(&ios_a));
        assert!(refreshed_b.contains(&ios_b));
    }
}
