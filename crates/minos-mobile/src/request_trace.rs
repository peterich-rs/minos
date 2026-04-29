//! In-process request tracing for the mobile-side Rust core.
//!
//! The capture model mirrors `log_capture`: keep a bounded in-memory ring so
//! a freshly opened inspector can replay recent activity, and broadcast every
//! update so Flutter can tail the stream live.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use tokio::sync::broadcast;

const RING_CAPACITY: usize = 200;
const BROADCAST_CAPACITY: usize = 256;
const SUMMARY_LIMIT: usize = 160;
const ERROR_LIMIT: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestTransport {
    Http,
    Rpc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestTraceStatus {
    Pending,
    Success,
    Failure,
}

#[derive(Debug, Clone)]
pub struct RequestTraceRecord {
    pub id: u64,
    pub transport: RequestTransport,
    pub method: String,
    pub target: String,
    pub thread_id: Option<String>,
    pub request_summary: Option<String>,
    pub response_summary: Option<String>,
    pub error_detail: Option<String>,
    pub status: RequestTraceStatus,
    pub status_code: Option<u16>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub duration_ms: Option<u32>,
}

struct State {
    ring: Mutex<VecDeque<RequestTraceRecord>>,
    sender: broadcast::Sender<RequestTraceRecord>,
    next_id: AtomicU64,
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| {
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        State {
            ring: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
            sender,
            next_id: AtomicU64::new(1),
        }
    })
}

#[must_use]
pub fn recent() -> Vec<RequestTraceRecord> {
    state()
        .ring
        .lock()
        .map(|guard| guard.iter().cloned().collect())
        .unwrap_or_default()
}

#[must_use]
pub fn subscribe() -> broadcast::Receiver<RequestTraceRecord> {
    state().sender.subscribe()
}

pub fn clear() {
    if let Ok(mut ring) = state().ring.lock() {
        ring.clear();
    }
}

#[must_use]
pub fn start(
    transport: RequestTransport,
    method: impl Into<String>,
    target: impl Into<String>,
    thread_id: Option<String>,
    request_summary: Option<String>,
) -> u64 {
    let trace = RequestTraceRecord {
        id: state().next_id.fetch_add(1, Ordering::Relaxed),
        transport,
        method: method.into(),
        target: target.into(),
        thread_id,
        request_summary: request_summary.map(|s| trim_summary(&s, SUMMARY_LIMIT)),
        response_summary: None,
        error_detail: None,
        status: RequestTraceStatus::Pending,
        status_code: None,
        started_at_ms: Utc::now().timestamp_millis(),
        completed_at_ms: None,
        duration_ms: None,
    };
    let id = trace.id;
    update_ring(id, |_| trace.clone());
    id
}

pub fn finish_success(
    id: u64,
    status_code: Option<u16>,
    response_summary: Option<String>,
    thread_id: Option<String>,
) {
    let now = Utc::now().timestamp_millis();
    let _ = update_ring(id, |record| {
        record.status = RequestTraceStatus::Success;
        record.status_code = status_code;
        record.response_summary = response_summary.map(|s| trim_summary(&s, SUMMARY_LIMIT));
        if record.thread_id.is_none() {
            record.thread_id = thread_id;
        }
        record.completed_at_ms = Some(now);
        record.duration_ms = Some(duration_ms(record.started_at_ms, now));
        record.clone()
    });
}

pub fn finish_failure(id: u64, status_code: Option<u16>, error_detail: impl Into<String>) {
    let now = Utc::now().timestamp_millis();
    let error_detail = trim_summary(&error_detail.into(), ERROR_LIMIT);
    let _ = update_ring(id, |record| {
        record.status = RequestTraceStatus::Failure;
        record.status_code = status_code;
        record.error_detail = Some(error_detail.clone());
        record.completed_at_ms = Some(now);
        record.duration_ms = Some(duration_ms(record.started_at_ms, now));
        record.clone()
    });
}

fn update_ring(
    id: u64,
    update: impl FnOnce(&mut RequestTraceRecord) -> RequestTraceRecord,
) -> Option<RequestTraceRecord> {
    let st = state();
    let snapshot = st.ring.lock().ok().map(|mut ring| {
        if let Some(record) = ring.iter_mut().find(|record| record.id == id) {
            update(record)
        } else {
            let mut record = RequestTraceRecord {
                id,
                transport: RequestTransport::Http,
                method: String::new(),
                target: String::new(),
                thread_id: None,
                request_summary: None,
                response_summary: None,
                error_detail: None,
                status: RequestTraceStatus::Pending,
                status_code: None,
                started_at_ms: Utc::now().timestamp_millis(),
                completed_at_ms: None,
                duration_ms: None,
            };
            let snapshot = update(&mut record);
            record = snapshot.clone();
            if ring.len() == RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(record);
            snapshot
        }
    });
    if let Some(snapshot) = snapshot {
        let _ = st.sender.send(snapshot.clone());
        Some(snapshot)
    } else {
        None
    }
}

fn duration_ms(started_at_ms: i64, completed_at_ms: i64) -> u32 {
    u32::try_from((completed_at_ms - started_at_ms).max(0)).unwrap_or(u32::MAX)
}

fn trim_summary(value: &str, max_len: usize) -> String {
    let mut out = value.trim().replace('\n', " ");
    if out.len() > max_len {
        out.truncate(max_len.saturating_sub(1));
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_returns_updated_snapshot() {
        clear();
        let id = start(
            RequestTransport::Http,
            "GET",
            "/v1/threads",
            None,
            Some("limit=50".into()),
        );
        finish_success(id, Some(200), Some("threads=2".into()), None);

        let recent = recent();
        let record = recent
            .iter()
            .find(|record| record.id == id)
            .expect("updated trace record should be present");
        assert_eq!(record.status, RequestTraceStatus::Success);
        assert_eq!(record.status_code, Some(200));
        assert_eq!(record.response_summary.as_deref(), Some("threads=2"));
    }

    #[test]
    fn ring_is_bounded() {
        clear();
        for i in 0..(RING_CAPACITY + 5) {
            let id = start(
                RequestTransport::Rpc,
                "minos_send_user_message",
                "rpc:minos_send_user_message",
                Some(format!("thr_{i}")),
                Some(format!("message={i}")),
            );
            finish_success(id, None, Some("ok".into()), None);
        }
        let recent = recent();
        assert_eq!(recent.len(), RING_CAPACITY);
        assert_eq!(recent[0].thread_id.as_deref(), Some("thr_5"));
    }
}
