//! In-process tracing capture for the mobile-side Rust core.
//!
//! Wears two hats so the Dart UI can both surface a live tail and replay
//! recent history when a debug panel opens after the fact:
//!
//! 1. A bounded ring buffer (`RING_CAPACITY` records) so a freshly-mounted
//!    log panel can show events that fired before subscription.
//! 2. A `tokio::sync::broadcast::Sender<LogRecord>` that any subscriber can
//!    receive on; FRB exposes this as a Dart stream.
//!
//! Composes with the existing `mars-xlog` layer instead of replacing it —
//! `logging::init` registers both, so on-disk logs keep their full fidelity
//! while the UI gets a free copy.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// Cap on records held in the in-memory ring. Sized for "useful tail of a
/// failed pairing attempt", not "full session history" — xlog owns the
/// long-term archive on disk.
const RING_CAPACITY: usize = 500;

/// Broadcast slack so a slow subscriber doesn't pin every emitter. Tail is
/// best-effort: lagged subscribers receive `Lagged(n)` and resync from
/// `recent()` if they care.
const BROADCAST_CAPACITY: usize = 256;

/// Severity tag mirrored across the FFI boundary. Kept narrow on purpose:
/// matches `tracing::Level` 1:1 so the conversion stays a `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<&tracing::Level> for LogLevel {
    fn from(level: &tracing::Level) -> Self {
        if *level == tracing::Level::TRACE {
            Self::Trace
        } else if *level == tracing::Level::DEBUG {
            Self::Debug
        } else if *level == tracing::Level::INFO {
            Self::Info
        } else if *level == tracing::Level::WARN {
            Self::Warn
        } else {
            Self::Error
        }
    }
}

/// One captured tracing event. `message` already includes structured fields
/// flattened into a `key=value` tail so the Dart UI doesn't need to know
/// about field types.
#[derive(Debug, Clone)]
pub struct LogRecord {
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    pub ts_ms: i64,
}

struct State {
    ring: Mutex<VecDeque<LogRecord>>,
    sender: broadcast::Sender<LogRecord>,
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| {
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        State {
            ring: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
            sender,
        }
    })
}

/// Snapshot of the records currently held in the ring buffer, oldest first.
#[must_use]
pub fn recent() -> Vec<LogRecord> {
    state()
        .ring
        .lock()
        .map(|guard| guard.iter().cloned().collect())
        .unwrap_or_default()
}

/// Subscribe to the live tail. Late subscribers miss anything older than
/// what `recent()` returns; pair the two when populating a fresh UI.
#[must_use]
pub fn subscribe() -> broadcast::Receiver<LogRecord> {
    state().sender.subscribe()
}

/// `tracing` Layer that records every event into both the ring buffer and
/// the broadcast channel. Cheap on the hot path: one `Mutex` lock + one
/// `broadcast::send` per event, plus the visitor allocations that any
/// formatting layer would also do.
pub struct CaptureLayer;

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let record = LogRecord {
            level: LogLevel::from(metadata.level()),
            target: metadata.target().to_string(),
            message: visitor.into_message(),
            ts_ms: Utc::now().timestamp_millis(),
        };

        let st = state();
        if let Ok(mut ring) = st.ring.lock() {
            if ring.len() == RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(record.clone());
        }
        let _ = st.sender.send(record);
    }
}

/// Field-flattening visitor: pulls the `message` field out as the prefix
/// and renders every other field as `name=debug` so the user can see the
/// structured context inline.
#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: String,
}

impl MessageVisitor {
    fn into_message(self) -> String {
        match (self.message, self.fields.is_empty()) {
            (Some(msg), true) => msg,
            (Some(msg), false) => format!("{msg} {fields}", fields = self.fields),
            (None, _) => self.fields,
        }
    }

    fn append_field(&mut self, name: &str, value: impl fmt::Display) {
        use fmt::Write;
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        let _ = write!(&mut self.fields, "{name}={value}");
    }
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.append_field(field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            self.append_field(field.name(), format!("{value:?}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;

    fn fresh_state() {
        if let Ok(mut ring) = state().ring.lock() {
            ring.clear();
        }
    }

    #[test]
    fn capture_layer_records_message_and_fields() {
        fresh_state();
        let _guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(CaptureLayer));

        tracing::info!(target: "minos_mobile::log_capture::tests", url = "wss://x", "ping");

        let records = recent();
        assert!(
            records
                .iter()
                .any(|r| r.message.contains("ping") && r.message.contains("url=wss://x")),
            "expected 'ping url=wss://x' in records, got {records:?}"
        );
    }

    #[test]
    fn ring_buffer_is_bounded_and_drops_oldest() {
        // The ring is process-global, so concurrent tests can push extra
        // records onto it. Filter by a unique target and assert that the
        // first surviving entry from *our* target has an index past the
        // eviction floor — foreign records only push that floor higher.
        const TARGET: &str = "minos_mobile::log_capture::ring_test";
        fresh_state();
        let _guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(CaptureLayer));

        for i in 0..(RING_CAPACITY + 50) {
            tracing::info!(target: TARGET, "event {i}");
        }

        let records = recent();
        assert_eq!(records.len(), RING_CAPACITY);
        let our_first = records
            .iter()
            .find(|r| r.target == TARGET)
            .expect("at least one of our records must survive eviction");
        let index: usize = our_first
            .message
            .strip_prefix("event ")
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("unexpected message shape: {:?}", our_first.message));
        assert!(
            index >= 50,
            "expected first surviving event index >= 50, got {index}"
        );
    }

    #[tokio::test]
    async fn subscribers_receive_live_records() {
        // The capture layer + broadcast channel are process-global, so any
        // other test running concurrently can also push records onto our
        // subscriber. Filter on a unique target so we ignore that crosstalk.
        const TARGET: &str = "minos_mobile::log_capture::live_tail_test";
        fresh_state();
        let mut rx = subscribe();
        let _guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(CaptureLayer));

        tracing::warn!(target: TARGET, "live tail");

        let deadline = std::time::Duration::from_millis(500);
        let received = tokio::time::timeout(deadline, async {
            loop {
                match rx.recv().await {
                    Ok(record) if record.target == TARGET => return record,
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        panic!("broadcast closed before our record arrived")
                    }
                }
            }
        })
        .await
        .expect("subscriber timed out");
        assert_eq!(received.level, LogLevel::Warn);
        assert!(received.message.contains("live tail"));
    }
}
