//! Ingest deduplication helpers.

use minos_protocol::LocalIngestFrame;

use super::AppState;

pub(crate) fn mark_ingest_applied(state: &mut AppState, frame: &LocalIngestFrame) -> bool {
    let session_id = frame.session_id.as_str();
    let seq = frame.seq;
    let fingerprint = ingest_fingerprint(frame);
    if state.applied_ingest_fingerprints.contains(&fingerprint) {
        if seq > 0 {
            let watermark = state
                .session_watermarks
                .entry(session_id.to_owned())
                .or_insert(0);
            *watermark = (*watermark).max(seq);
        }
        return false;
    }

    if seq > 0 {
        let watermark = state
            .session_watermarks
            .entry(session_id.to_owned())
            .or_insert(0);
        if seq <= *watermark {
            return false;
        }
        *watermark = seq;
    }

    state.applied_ingest_fingerprints.insert(fingerprint);
    true
}

pub(crate) fn frame_marks_agent_result_done(frame: &LocalIngestFrame) -> bool {
    frame.ui_events.iter().any(|event| {
        matches!(
            event,
            minos_ui_protocol::UiEventMessage::MessageCompleted { .. }
                | minos_ui_protocol::UiEventMessage::SessionClosed { .. }
        )
    })
}

fn ingest_fingerprint(frame: &LocalIngestFrame) -> String {
    if frame.seq > 0 {
        return format!("{}:seq:{}", frame.session_id, frame.seq);
    }
    let payload = serde_json::to_string(&frame.ui_events).unwrap_or_default();
    format!("{}:{payload}", frame.session_id)
}
