//! Ingest deduplication helpers.

use minos_agent_runtime::ThreadState;
use minos_protocol::LocalIngestFrame;

use super::AppState;

pub(crate) fn mark_ingest_applied(state: &mut AppState, frame: &LocalIngestFrame) -> bool {
    let thread_id = frame.thread_id.as_str();
    let seq = frame.seq;
    let fingerprint = ingest_fingerprint(frame);
    if state.applied_ingest_fingerprints.contains(&fingerprint) {
        if seq > 0 {
            let watermark = state
                .thread_watermarks
                .entry(thread_id.to_owned())
                .or_insert(0);
            *watermark = (*watermark).max(seq);
        }
        return false;
    }

    if seq > 0 {
        let watermark = state
            .thread_watermarks
            .entry(thread_id.to_owned())
            .or_insert(0);
        if seq <= *watermark {
            return false;
        }
        *watermark = seq;
    }

    state.applied_ingest_fingerprints.insert(fingerprint);
    true
}

pub(crate) fn thread_is_done(state: &ThreadState) -> bool {
    matches!(state, ThreadState::Idle | ThreadState::Closed { .. })
}

pub(crate) fn frame_marks_agent_result_done(frame: &LocalIngestFrame) -> bool {
    frame.ui_events.iter().any(|event| {
        matches!(
            event,
            minos_ui_protocol::UiEventMessage::MessageCompleted { .. }
                | minos_ui_protocol::UiEventMessage::ThreadClosed { .. }
        )
    })
}

fn ingest_fingerprint(frame: &LocalIngestFrame) -> String {
    if frame.seq > 0 {
        return format!("{}:seq:{}", frame.thread_id, frame.seq);
    }
    let payload = serde_json::to_string(&frame.ui_events).unwrap_or_default();
    format!("{}:{payload}", frame.thread_id)
}
