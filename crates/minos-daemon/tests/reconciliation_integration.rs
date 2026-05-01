//! Phase D Task D6: end-to-end coverage of the Reconciliator.
//!
//! Two cases:
//!
//! 1. `reconciliation_replays_missing_seqs` — the local DB has every
//!    row from `1..=100` and the backend last persisted `seq = 50`.
//!    The Reconciliator must stream `51..=100` back onto the
//!    relay-out channel as `Envelope::Ingest`, in order.
//!
//! 2. `reconciliation_falls_back_to_jsonl_on_gap` — the local DB has
//!    `1..=100` minus `60..=70` and the backend max is `50`. The
//!    Reconciliator detects the gap during replay and delegates to
//!    `jsonl_recover`, which feeds 11 events through `EventWriter`
//!    tagged `source = 'jsonl_recovery'`.

// SQLite stores seq + ts_ms as i64 even where the Rust-side semantics
// use u64. The bind-site casts mirror the same allow used by the
// production store/event_writer code so the SQL surface stays readable
// without `try_from`-shaped clutter.
#![allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]

use minos_daemon::reconciliator::Reconciliator;
use minos_daemon::store::event_writer::EventWriter;
use minos_daemon::store::LocalStore;
use minos_protocol::Envelope;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Insert a workspace + thread row + every `seq` in `seqs` into the
/// local `events` table. `last_seq` on the thread row is set to the max
/// seq, mirroring what `EventWriter` would have done for live events.
async fn seed_thread_with_events(
    store: &LocalStore,
    thread_id: &str,
    seqs: &[u64],
    session_id: Option<&str>,
) {
    sqlx::query(
        "INSERT OR IGNORE INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w', 0, 0)",
    )
    .execute(store.pool())
    .await
    .unwrap();
    let max_seq = *seqs.iter().max().unwrap_or(&0) as i64;
    sqlx::query(
        "INSERT INTO threads(thread_id, workspace_root, agent, codex_session_id, status, last_seq, started_at, last_activity_at) \
         VALUES (?, '/w', 'codex', ?, 'idle', ?, 0, 0)",
    )
    .bind(thread_id)
    .bind(session_id)
    .bind(max_seq)
    .execute(store.pool())
    .await
    .unwrap();
    for s in seqs {
        sqlx::query(
            "INSERT INTO events(thread_id, seq, payload, ts_ms, source) VALUES (?, ?, ?, ?, 'live')",
        )
        .bind(thread_id)
        .bind(*s as i64)
        .bind(serde_json::to_vec(&serde_json::json!({"seq": s})).unwrap())
        .bind(0i64)
        .execute(store.pool())
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reconciliation_replays_missing_seqs() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap(),
    );
    let (relay_out_tx, mut relay_out_rx) = mpsc::channel::<Envelope>(256);
    let writer = Arc::new(EventWriter::spawn(store.clone(), relay_out_tx.clone()));

    let seqs: Vec<u64> = (1..=100).collect();
    seed_thread_with_events(&store, "thr-X", &seqs, None).await;

    let recon = Reconciliator::new(store.clone(), writer.clone(), relay_out_tx);

    let mut backend_seqs = HashMap::new();
    backend_seqs.insert("thr-X".to_string(), 50u64);
    recon.on_checkpoint(backend_seqs).await.unwrap();

    let mut got: Vec<u64> = Vec::new();
    while let Ok(Some(env)) =
        tokio::time::timeout(std::time::Duration::from_millis(500), relay_out_rx.recv()).await
    {
        if let Envelope::Ingest { thread_id, seq, .. } = env {
            assert_eq!(thread_id, "thr-X");
            got.push(seq);
        }
    }
    assert_eq!(
        got,
        (51..=100).collect::<Vec<_>>(),
        "should replay 51..=100 in order"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reconciliation_falls_back_to_jsonl_on_gap() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap(),
    );
    let (relay_out_tx, _relay_out_rx) = mpsc::channel::<Envelope>(1024);
    let writer = Arc::new(EventWriter::spawn(store.clone(), relay_out_tx.clone()));

    // Seed seqs 1..=100 minus 60..=70.
    let seqs: Vec<u64> = (1..=100).filter(|s| !(60..=70).contains(s)).collect();
    seed_thread_with_events(&store, "thr-Y", &seqs, Some("sess-uuid-1")).await;

    // Stage a fake codex jsonl under a temp HOME so the recovery path
    // fires without touching the developer's `~/.codex`.
    let fake_codex_root = tmp.path().join("fake-codex-home");
    std::fs::create_dir_all(fake_codex_root.join(".codex/sessions")).unwrap();
    let jsonl_path = fake_codex_root.join(".codex/sessions/sess-uuid-1.jsonl");
    let payload_lines = (60..=70u64)
        .map(|s| serde_json::json!({"recovered_seq": s, "ts_ms": s as i64}).to_string())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&jsonl_path, payload_lines).unwrap();

    // Inject the fake codex home directly so the test never mutates
    // the process-global `$HOME` and never touches the developer's
    // real `~/.codex`.
    let recon = Reconciliator::with_codex_home(
        store.clone(),
        writer.clone(),
        relay_out_tx,
        fake_codex_root.clone(),
    );
    let mut backend_seqs = HashMap::new();
    backend_seqs.insert("thr-Y".to_string(), 50u64);
    recon.on_checkpoint(backend_seqs).await.unwrap();

    // Allow EventWriter's batch window to flush.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let recovered: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE thread_id = ? AND source = 'jsonl_recovery'",
    )
    .bind("thr-Y")
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(
        recovered, 11,
        "11 missing events should be recovered (seq 60..=70)"
    );
}
