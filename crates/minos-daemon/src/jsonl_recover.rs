//! Codex JSONL recovery: parses `~/.codex/sessions/{codex_session_id}.jsonl`
//! and replays its events through [`EventWriter::write_recovery`] so the
//! local DB closes the gap a `Reconciliator` round detected.
//!
//! Phase D Task D4. Lives outside `agent-runtime/src/exec_jsonl.rs`
//! (which was the live JSONL exec driver and has been deleted): this is
//! a post-hoc parser, not a runtime driver.
//!
//! ## Why a separate writer entrypoint
//!
//! The codex JSONL on disk has its own per-line schema; we don't try to
//! match the original `seq` numbering from the live ingest. Instead each
//! recovered line gets a fresh monotonic `seq` from `EventWriter`, the
//! way it would for any other write — but tagged `source = 'jsonl_recovery'`
//! in the `events` table so observability / debugging can tell apart
//! "the daemon witnessed this live" from "the daemon synthesised it from
//! a CLI log on disk". See [`crate::store::event_writer::EventWriter::write_recovery`].
//!
//! ## Test-injection of the codex home
//!
//! Production reads from `$HOME/.codex/sessions/{sid}.jsonl`. Tests pass
//! [`recover_with_root`] a temp dir as `codex_home_root` to avoid
//! touching the developer's real codex sessions.

use crate::store::event_writer::EventWriter;
use anyhow::Result;
use minos_agent_runtime::{AgentKind, RawIngest};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Production entrypoint: derive `codex_home` from `$HOME`.
///
/// Returns `Ok(())` (logged) when:
/// - `codex_session_id` is `None` (the thread's runtime never recorded
///   one — e.g. an orphan from before app-server was wired),
/// - `$HOME` is unset,
/// - the JSONL file is missing or unreadable.
///
/// Recovery is best-effort: a partial replay is better than aborting
/// reconciliation entirely.
pub async fn recover(
    thread_id: &str,
    missing_seqs: &[u64],
    codex_session_id: Option<&str>,
    writer: &Arc<EventWriter>,
) -> Result<()> {
    let Ok(home) = std::env::var("HOME") else {
        tracing::warn!(
            target: "minos_daemon::jsonl_recover",
            thread_id,
            "$HOME unset; skipping recovery"
        );
        return Ok(());
    };
    recover_with_root(
        thread_id,
        missing_seqs,
        codex_session_id,
        Path::new(&home),
        writer,
    )
    .await
}

/// Test-injectable entrypoint: caller passes the directory that should
/// stand in for `$HOME` (i.e. the parent of `.codex/sessions/`).
pub async fn recover_with_root(
    thread_id: &str,
    _missing_seqs: &[u64],
    codex_session_id: Option<&str>,
    codex_home_root: &Path,
    writer: &Arc<EventWriter>,
) -> Result<()> {
    let Some(sid) = codex_session_id else {
        tracing::warn!(
            target: "minos_daemon::jsonl_recover",
            thread_id,
            "no codex_session_id on thread; skipping recovery"
        );
        return Ok(());
    };

    let path = jsonl_path(codex_home_root, sid);
    let file = match File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                target: "minos_daemon::jsonl_recover",
                thread_id,
                path = %path.display(),
                error = %e,
                "jsonl not readable; skipping recovery"
            );
            return Ok(());
        }
    };

    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut recovered: u64 = 0;
    let mut skipped_malformed: u64 = 0;
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let payload: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "minos_daemon::jsonl_recover",
                    thread_id,
                    error = %e,
                    "skipping malformed jsonl line"
                );
                skipped_malformed += 1;
                continue;
            }
        };
        // The codex JSONL line carries its own ts; if absent we fall
        // back to 0. The downstream events table has no NOT NULL
        // constraint that this would violate.
        let ts_ms = payload
            .get("ts_ms")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let ingest = RawIngest {
            agent: AgentKind::Codex,
            thread_id: thread_id.to_string(),
            payload,
            ts_ms,
        };
        if let Err(e) = writer.write_recovery(ingest).await {
            tracing::warn!(
                target: "minos_daemon::jsonl_recover",
                thread_id,
                error = %e,
                "write_recovery failed for one event; continuing"
            );
            continue;
        }
        recovered += 1;
    }
    tracing::info!(
        target: "minos_daemon::jsonl_recover",
        thread_id,
        recovered,
        skipped_malformed,
        "jsonl_recover completed"
    );
    Ok(())
}

fn jsonl_path(codex_home_root: &Path, codex_session_id: &str) -> PathBuf {
    codex_home_root
        .join(".codex")
        .join("sessions")
        .join(format!("{codex_session_id}.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_path_uses_dot_codex_sessions_layout() {
        let p = jsonl_path(Path::new("/users/x"), "sess-uuid-1");
        assert_eq!(
            p,
            PathBuf::from("/users/x/.codex/sessions/sess-uuid-1.jsonl")
        );
    }
}
