//! Phase D Task D3 stub. Full implementation lands in Task D4.
//!
//! Today the function logs and returns `Ok(())`; the Reconciliator can
//! call it without a build break. D4 replaces this with a real codex
//! JSONL parser that feeds missing events through [`crate::store::event_writer::EventWriter`]
//! tagged `source = 'jsonl_recovery'`.

use crate::store::event_writer::EventWriter;
use anyhow::Result;
use std::sync::Arc;

pub async fn recover(
    thread_id: &str,
    _missing_seqs: &[u64],
    codex_session_id: Option<&str>,
    _writer: &Arc<EventWriter>,
) -> Result<()> {
    tracing::warn!(
        target: "minos_daemon::jsonl_recover",
        thread_id,
        codex_session_id = ?codex_session_id,
        "jsonl_recover stub invoked; real implementation lands in Phase D Task D4"
    );
    Ok(())
}
