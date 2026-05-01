//! Reconciliator: bridges the backend's `IngestCheckpoint` checkpoint and
//! the daemon's local SQLite event store.
//!
//! Phase D / spec §9. On every `/v1/devices/ws` reconnect the backend
//! emits `Event::IngestCheckpoint { last_seq_per_thread }` carrying its
//! per-thread `MAX(seq)` of durably-persisted `raw_events`. The daemon
//! compares each backend max against its own `threads.last_seq`:
//!
//! - `backend_seq >= local_seq` → nothing to do (backend is at or ahead).
//! - `backend_seq < local_seq` → the daemon has rows the backend never
//!   acknowledged. Replay `(backend_seq + 1)..=local_seq` from the local
//!   `events` table back onto the relay-out channel as `Envelope::Ingest`
//!   frames so they get re-uploaded.
//!
//! Replay is per-thread, prioritised so live work resumes first
//! (`running` > `idle/resuming/starting` > `suspended` > everything else).
//!
//! Whenever the local DB itself has a hole in the requested range
//! (read of `(from_seq..=to_seq)` returns fewer rows than expected), the
//! Reconciliator delegates to [`crate::jsonl_recover::recover`] to fill
//! the gap from the codex CLI's per-session JSONL on disk.
//!
//! ## Why writes go through `EventWriter`
//!
//! Recovery rows from JSONL are persisted via [`crate::store::event_writer::EventWriter`]
//! and never via direct SQLite writes — the writer owns the monotonic
//! `seq` invariant per thread (Phase C). Replay of EXISTING rows does
//! not go through the writer (no new SQLite row is created); it streams
//! the pre-existing payloads back to the relay verbatim.

use crate::store::event_writer::EventWriter;
use crate::store::LocalStore;
use anyhow::Result;
use minos_agent_runtime::AgentKind;
use minos_protocol::Envelope;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Map a `threads.agent` DB string back to `AgentKind`. The daemon
/// rejects unknown strings rather than coercing — a row with a fresh
/// agent value usually means a schema mismatch, and silently mapping it
/// to `Codex` would corrupt the relay payload.
fn parse_agent(s: &str) -> Result<AgentKind> {
    match s {
        "codex" => Ok(AgentKind::Codex),
        "claude" => Ok(AgentKind::Claude),
        "gemini" => Ok(AgentKind::Gemini),
        other => anyhow::bail!("unknown agent in threads.agent: {other}"),
    }
}

/// Priority ordinal for thread sort (lower = handled first).
fn status_priority(status: &str) -> u8 {
    match status {
        "running" => 0,
        "idle" | "resuming" | "starting" => 1,
        "suspended" => 2,
        _ => 3,
    }
}

pub struct Reconciliator {
    store: Arc<LocalStore>,
    writer: Arc<EventWriter>,
    relay_out: mpsc::Sender<Envelope>,
}

impl Reconciliator {
    pub fn new(
        store: Arc<LocalStore>,
        writer: Arc<EventWriter>,
        relay_out: mpsc::Sender<Envelope>,
    ) -> Self {
        Self {
            store,
            writer,
            relay_out,
        }
    }

    /// Process one `IngestCheckpoint` frame: for every local thread whose
    /// `last_seq` is ahead of the backend's max, replay the gap.
    ///
    /// Threads not present in the backend's map are treated as
    /// `backend_seq = 0` (the backend has no rows for them).
    pub async fn on_checkpoint(&self, backend_seqs: HashMap<String, u64>) -> Result<()> {
        let mut local = self.store.list_threads(None, Some(500)).await?;
        local.sort_by_key(|t| status_priority(&t.status));

        for thread in local {
            let backend_seq = backend_seqs.get(&thread.thread_id).copied().unwrap_or(0);
            let local_seq = u64::try_from(thread.last_seq).unwrap_or(0);
            if backend_seq >= local_seq {
                continue;
            }
            if let Err(e) = self
                .replay_thread(
                    &thread.thread_id,
                    backend_seq + 1,
                    local_seq,
                    &thread.agent,
                    thread.codex_session_id.as_deref(),
                )
                .await
            {
                tracing::warn!(
                    target: "minos_daemon::reconciliator",
                    thread_id = %thread.thread_id,
                    error = %e,
                    "reconciliation failed for thread; continuing with next"
                );
            }
        }
        Ok(())
    }

    /// Replay `from_seq..=to_seq` for one thread. Reads in 1000-row
    /// windows (matches `LocalStore::read_events`'s closed range bounds)
    /// to keep memory bounded for very long sessions.
    ///
    /// If the local DB returns fewer rows than requested, the missing
    /// seqs are fed to [`crate::jsonl_recover::recover`].
    async fn replay_thread(
        &self,
        thread_id: &str,
        from_seq: u64,
        to_seq: u64,
        agent_str: &str,
        codex_session_id: Option<&str>,
    ) -> Result<()> {
        let agent = parse_agent(agent_str)?;
        let mut next = from_seq;
        let mut all_seqs: Vec<u64> = Vec::new();
        while next <= to_seq {
            let upper = (next + 999).min(to_seq);
            let rows = self.store.read_events(thread_id, next, upper).await?;
            for row in rows {
                let seq = u64::try_from(row.seq).unwrap_or(0);
                all_seqs.push(seq);
                let payload: serde_json::Value = serde_json::from_slice(&row.payload)?;
                let env = Envelope::Ingest {
                    version: 1,
                    agent,
                    thread_id: row.thread_id.clone(),
                    seq,
                    payload,
                    ts_ms: row.ts_ms,
                };
                self.relay_out
                    .send(env)
                    .await
                    .map_err(|_| anyhow::anyhow!("relay_out channel closed"))?;
            }
            next = upper + 1;
        }
        // Detect gaps. `expected.contains(&seq)` would be O(n²); convert
        // present-seqs to a set lookup for the linear filter below.
        let present: std::collections::BTreeSet<u64> = all_seqs.iter().copied().collect();
        let missing: Vec<u64> = (from_seq..=to_seq).filter(|s| !present.contains(s)).collect();
        if !missing.is_empty() {
            tracing::warn!(
                target: "minos_daemon::reconciliator",
                thread_id,
                missing_count = missing.len(),
                from_seq,
                to_seq,
                "DB gap detected; attempting jsonl fallback"
            );
            crate::jsonl_recover::recover(thread_id, &missing, codex_session_id, &self.writer)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_accepts_known_strings() {
        assert_eq!(parse_agent("codex").unwrap(), AgentKind::Codex);
        assert_eq!(parse_agent("claude").unwrap(), AgentKind::Claude);
        assert_eq!(parse_agent("gemini").unwrap(), AgentKind::Gemini);
        assert!(parse_agent("rovo").is_err());
    }

    #[test]
    fn status_priority_orders_running_first() {
        assert!(status_priority("running") < status_priority("idle"));
        assert!(status_priority("idle") < status_priority("suspended"));
        assert!(status_priority("suspended") < status_priority("closed"));
        // Unknown statuses sort behind everything.
        assert!(status_priority("zonked") >= status_priority("closed"));
    }
}
