//! Backend ingest pipeline: persist raw → translate → fan out.
//!
//! Entry point [`dispatch`] is called once per inbound `Envelope::Ingest`
//! frame. It:
//!
//! 1. Upserts the `threads` row.
//! 2. Persists the raw event, discarding exact retransmits while assigning a
//!    fresh backend seq when a resumed daemon reuses an old process-local seq.
//! 3. Runs the per-agent translator. Translator errors surface as a
//!    synthetic `UiEventMessage::Error` so mobile sees something deterministic
//!    rather than a silent drop.
//! 4. For each produced UI event, wraps it in an `Envelope::Event` /
//!    `EventKind::UiEventMessage` and fans it out to every device paired
//!    with `owner_device_id` that has a live session.
//!
//! Fan-out is bounded: the SessionHandle's outbox is a fixed-size
//! `mpsc::channel(256)`; full channels drop the one frame with a warn log
//! rather than blocking the ingest path.

pub mod history;
pub mod translate;

use minos_domain::AgentName;
use minos_protocol::{Envelope, EventKind};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::error::BackendError;
use crate::ingest::translate::ThreadTranslators;
use crate::session::SessionRegistry;
use crate::store::{pairings, raw_events, threads};

/// Process one `Envelope::Ingest` frame.
#[allow(clippy::too_many_arguments)] // Single-site dispatcher; splitting obscures the 4-step pipeline.
pub async fn dispatch(
    pool: &SqlitePool,
    registry: &SessionRegistry,
    translators: &ThreadTranslators,
    agent: AgentName,
    thread_id: &str,
    seq: u64,
    payload: &Value,
    ts_ms: i64,
    owner_device_id: minos_domain::DeviceId,
) -> Result<(), BackendError> {
    // 1. Upsert the thread row (creates on first ingest, bumps last_ts_ms otherwise).
    threads::upsert(pool, thread_id, agent, &owner_device_id.to_string(), ts_ms).await?;

    // 2. Persist raw. The backend may assign a fresh seq when the daemon
    // resumes an existing thread with a process-local counter reset.
    let Some(persisted_seq) =
        raw_events::insert_assigning_seq(pool, thread_id, seq, agent, payload, ts_ms).await?
    else {
        tracing::debug!(
            target: "minos_backend::ingest",
            thread_id, seq, "ingest seq retransmit, dropping"
        );
        return Ok(());
    };

    // 3. Translate. Translator failures are non-fatal: we emit a synthetic
    // Error UI event so mobile sees a deterministic surface.
    let mut translated = match translators.translate(agent, thread_id, payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::ingest",
                ?e, thread_id, "translation failed; emitting synthetic Error"
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
            minos_ui_protocol::UiEventMessage::ThreadTitleUpdated { .. }
        )
    });
    if !has_explicit_title && thread_title_is_missing(pool, thread_id).await {
        if let Some(title) = derive_fallback_title(payload, &translated) {
            let _ = threads::update_title(pool, thread_id, &title).await;
            translated.insert(
                0,
                minos_ui_protocol::UiEventMessage::ThreadTitleUpdated {
                    thread_id: thread_id.to_string(),
                    title,
                },
            );
        }
    }

    // 4. Fan out each UI event to every live peer paired with owner_device_id.
    for ui in translated {
        // Side effects on DB when the UI event implies a thread mutation.
        match &ui {
            minos_ui_protocol::UiEventMessage::ThreadTitleUpdated { title, .. } => {
                let _ = threads::update_title(pool, thread_id, title).await;
            }
            minos_ui_protocol::UiEventMessage::MessageStarted { .. } => {
                let _ = threads::increment_message_count(pool, thread_id).await;
            }
            minos_ui_protocol::UiEventMessage::ThreadClosed { reason, .. } => {
                let _ = threads::mark_ended(pool, thread_id, reason, ts_ms).await;
                translators.drop_thread(thread_id);
            }
            _ => {}
        }

        let env = Envelope::Event {
            version: 1,
            event: EventKind::UiEventMessage {
                thread_id: thread_id.to_string(),
                seq: persisted_seq,
                ui,
                ts_ms,
            },
        };
        broadcast_to_peers_of(pool, registry, owner_device_id, &env).await;
    }

    Ok(())
}

async fn thread_title_is_missing(pool: &SqlitePool, thread_id: &str) -> bool {
    match sqlx::query_scalar::<_, Option<String>>("SELECT title FROM threads WHERE thread_id = ?1")
        .bind(thread_id)
        .fetch_optional(pool)
        .await
    {
        Ok(Some(None)) => true,
        Ok(Some(Some(_)) | None) => false,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::ingest",
                error = ?e,
                thread_id,
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
        minos_ui_protocol::UiEventMessage::TextDelta { text, .. } => sanitize_title(text),
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

/// Look up every peer for `device_id` in the DB, find each live session in
/// the registry (if any), and try-send `env` on its outbox. Misses
/// (unpaired device, peer offline, full outbox) are logged at debug/warn and
/// swallowed — ingest must stay crash-safe.
async fn broadcast_to_peers_of(
    pool: &SqlitePool,
    registry: &SessionRegistry,
    device_id: minos_domain::DeviceId,
    env: &Envelope,
) {
    let peers = match pairings::get_peers(pool, device_id).await {
        Ok(p) if !p.is_empty() => p,
        Ok(_) => {
            tracing::debug!(
                target: "minos_backend::ingest",
                device = %device_id,
                "no peers paired; dropping ui event"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::ingest",
                error = ?e,
                "failed to look up peers"
            );
            return;
        }
    };

    for peer in peers {
        let Some(handle) = registry.get(peer) else {
            tracing::debug!(
                target: "minos_backend::ingest",
                peer = %peer,
                "peer not live; dropping ui event"
            );
            continue;
        };

        // Route through `try_send_current` so a reconnect race (peer reconnects
        // between `get` and the send) cannot let a superseded socket consume
        // the live UI event. The replacement session will catch up via the
        // next ingest tick or via list/read_thread on its own (re)attach.
        if let Err(e) = registry.try_send_current(&handle, env.clone()) {
            tracing::warn!(
                target: "minos_backend::ingest",
                peer = %peer,
                error = ?e,
                "peer outbox full or superseded; dropping ui event"
            );
        }
    }
}
