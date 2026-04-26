//! Pure history-read helper backing HTTP `GET /v1/threads/{id}/events`.
//!
//! Originally extracted from the now-deleted WS `LocalRpcMethod::ReadThread`
//! handler during the Phase-C → Phase-D migration; the HTTP route is now
//! the single canonical entry point.

use minos_protocol::{ReadThreadParams, ReadThreadResponse};

use crate::http::BackendState;

/// Errors surfaced by [`read_thread`]. Mapped to HTTP status codes by the
/// caller (`http/v1/threads.rs`).
#[derive(Debug)]
pub enum HistoryError {
    /// `thread_id` does not exist OR is owned by a device other than
    /// `owner_id`. Both collapse to NotFound so we don't leak ownership
    /// information across pairings.
    NotFound,
    /// Underlying store / serialisation failure. Message is operator-facing.
    Internal(String),
}

/// Read a window of translated UI events for one thread. A fresh
/// [`minos_ui_protocol::CodexTranslatorState`] is instantiated per call,
/// so history replays never share state with the live-ingest translator
/// cache — guaranteeing deterministic output on repeated reads.
#[allow(clippy::too_many_lines)] // Single-site reader: ownership probe + read_range + translation + title/end-reason probes share a pagination cursor.
pub async fn read_thread(
    state: &BackendState,
    owner_id: minos_domain::DeviceId,
    params: ReadThreadParams,
) -> Result<ReadThreadResponse, HistoryError> {
    let owner_device_id: Option<String> =
        match sqlx::query_scalar("SELECT owner_device_id FROM threads WHERE thread_id = ?1")
            .bind(&params.thread_id)
            .fetch_optional(&state.store)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "minos_backend::ingest::history",
                    error = %e,
                    thread_id = %params.thread_id,
                    "read_thread.owner_probe failed"
                );
                return Err(HistoryError::Internal("read_thread failed".into()));
            }
        };
    let Some(owner_device_id) = owner_device_id else {
        return Err(HistoryError::NotFound);
    };
    if owner_device_id != owner_id.to_string() {
        return Err(HistoryError::NotFound);
    }

    let from_seq = params.from_seq.unwrap_or(0);
    let limit = params.limit.min(2000);
    let rows = match crate::store::raw_events::read_range(
        &state.store,
        &params.thread_id,
        from_seq,
        limit,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::ingest::history",
                error = ?e,
                thread_id = %params.thread_id,
                "read_thread.read_range failed"
            );
            return Err(HistoryError::Internal("read_thread failed".into()));
        }
    };

    // Fresh translator state per call so history stays deterministic.
    let mut translator_state =
        minos_ui_protocol::CodexTranslatorState::new(params.thread_id.clone());
    let mut ui_events: Vec<minos_ui_protocol::UiEventMessage> = Vec::new();
    let mut last_seq_read = from_seq;
    for row in &rows {
        last_seq_read = u64::try_from(row.seq).unwrap_or(last_seq_read);
        match row.agent {
            minos_domain::AgentName::Codex => {
                match minos_ui_protocol::translate_codex(&mut translator_state, &row.payload) {
                    Ok(v) => ui_events.extend(v),
                    Err(e) => ui_events.push(minos_ui_protocol::UiEventMessage::Error {
                        code: "translation_failed".into(),
                        message: format!("{e}"),
                        message_id: None,
                    }),
                }
            }
            minos_domain::AgentName::Claude => {
                match minos_ui_protocol::translate_claude(&row.payload) {
                    Ok(v) => ui_events.extend(v),
                    Err(e) => ui_events.push(minos_ui_protocol::UiEventMessage::Error {
                        code: "translation_failed".into(),
                        message: format!("{e}"),
                        message_id: None,
                    }),
                }
            }
            minos_domain::AgentName::Gemini => {
                match minos_ui_protocol::translate_gemini(&row.payload) {
                    Ok(v) => ui_events.extend(v),
                    Err(e) => ui_events.push(minos_ui_protocol::UiEventMessage::Error {
                        code: "translation_failed".into(),
                        message: format!("{e}"),
                        message_id: None,
                    }),
                }
            }
        }
    }

    if from_seq == 0
        && !ui_events.iter().any(|ui| {
            matches!(
                ui,
                minos_ui_protocol::UiEventMessage::ThreadTitleUpdated { .. }
            )
        })
    {
        let stored_title: Option<Option<String>> =
            match sqlx::query_scalar("SELECT title FROM threads WHERE thread_id = ?1")
                .bind(&params.thread_id)
                .fetch_optional(&state.store)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        target: "minos_backend::ingest::history",
                        error = %e,
                        thread_id = %params.thread_id,
                        "read_thread.title_probe failed"
                    );
                    return Err(HistoryError::Internal("read_thread failed".into()));
                }
            };
        if let Some(Some(title)) = stored_title {
            ui_events.insert(
                0,
                minos_ui_protocol::UiEventMessage::ThreadTitleUpdated {
                    thread_id: params.thread_id.clone(),
                    title,
                },
            );
        }
    }

    // Pagination cursor: if we filled the page, hand the caller a `next_seq`
    // to continue from. Otherwise the cursor is None (no more rows).
    let next_seq = if u32::try_from(rows.len()).unwrap_or(u32::MAX) == limit {
        Some(last_seq_read + 1)
    } else {
        None
    };

    // Look up end_reason (may be present even if rows are empty).
    let end_reason_json: Option<Option<String>> =
        match sqlx::query_scalar("SELECT end_reason FROM threads WHERE thread_id = ?1")
            .bind(&params.thread_id)
            .fetch_optional(&state.store)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "minos_backend::ingest::history",
                    error = %e,
                    thread_id = %params.thread_id,
                    "read_thread.end_reason_probe failed"
                );
                return Err(HistoryError::Internal("read_thread failed".into()));
            }
        };
    let thread_end_reason = end_reason_json
        .flatten()
        .as_ref()
        .and_then(|s| serde_json::from_str::<minos_ui_protocol::ThreadEndReason>(s).ok());

    Ok(ReadThreadResponse {
        ui_events,
        next_seq,
        thread_end_reason,
    })
}
