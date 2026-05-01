//! Phase D Task D4: integration coverage for `jsonl_recover`.
//!
//! Production reads from `$HOME/.codex/sessions/`; tests pass
//! `recover_with_root` a temp dir as the codex home so the developer's
//! real session files are never touched.

use minos_daemon::jsonl_recover::recover_with_root;
use minos_daemon::store::event_writer::EventWriter;
use minos_daemon::store::LocalStore;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Insert a thread row so `EventWriter` can write events for it.
async fn seed_thread(store: &LocalStore, thread_id: &str) {
    sqlx::query("INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w', 0, 0)")
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
         VALUES (?, '/w', 'codex', 'idle', 0, 0, 0)",
    )
    .bind(thread_id)
    .execute(store.pool())
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn recover_skips_when_no_codex_session_id() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap(),
    );
    seed_thread(&store, "thr-X").await;
    let (relay_tx, _relay_rx) = mpsc::channel(16);
    let writer = Arc::new(EventWriter::spawn(store.clone(), relay_tx));

    recover_with_root("thr-X", &[1, 2, 3], None, tmp.path(), &writer)
        .await
        .expect("recover with no codex_session_id is a noop");

    // Allow any (non-existent) writer batches to flush.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE thread_id = ?")
        .bind("thr-X")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0, "no events should be written");
}

#[tokio::test(flavor = "multi_thread")]
async fn recover_skips_when_file_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap(),
    );
    seed_thread(&store, "thr-Y").await;
    let (relay_tx, _relay_rx) = mpsc::channel(16);
    let writer = Arc::new(EventWriter::spawn(store.clone(), relay_tx));

    // codex_session_id points at a file that doesn't exist under our
    // fake codex home → recover logs and returns Ok.
    recover_with_root(
        "thr-Y",
        &[1, 2, 3],
        Some("does-not-exist"),
        tmp.path(),
        &writer,
    )
    .await
    .expect("missing jsonl is a noop, not an error");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE thread_id = ?")
        .bind("thr-Y")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn recover_parses_valid_lines_and_writes_with_jsonl_recovery_source() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap(),
    );
    seed_thread(&store, "thr-Z").await;
    let (relay_tx, _relay_rx) = mpsc::channel(64);
    let writer = Arc::new(EventWriter::spawn(store.clone(), relay_tx));

    // Stage a fake `.codex/sessions/sess-uuid-1.jsonl` with three valid
    // lines and one malformed line. The malformed line MUST NOT abort
    // recovery for the others.
    let codex_root = tmp.path();
    std::fs::create_dir_all(codex_root.join(".codex/sessions")).unwrap();
    let lines = [
        serde_json::json!({"recovered_seq": 1, "ts_ms": 100}).to_string(),
        "{not valid json".to_string(),
        serde_json::json!({"recovered_seq": 2, "ts_ms": 200}).to_string(),
        serde_json::json!({"recovered_seq": 3, "ts_ms": 300}).to_string(),
    ]
    .join("\n");
    std::fs::write(codex_root.join(".codex/sessions/sess-uuid-1.jsonl"), lines).unwrap();

    recover_with_root(
        "thr-Z",
        &[1, 2, 3],
        Some("sess-uuid-1"),
        codex_root,
        &writer,
    )
    .await
    .unwrap();

    // Allow EventWriter's 5ms batch window to flush.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE thread_id = ? AND source = 'jsonl_recovery'",
    )
    .bind("thr-Z")
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(count, 3, "3 valid lines should be recovered");

    // Sanity: nothing leaked into the 'live' source.
    let live: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE thread_id = ? AND source = 'live'")
            .bind("thr-Z")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(live, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn recover_skips_blank_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap(),
    );
    seed_thread(&store, "thr-blank").await;
    let (relay_tx, _relay_rx) = mpsc::channel(16);
    let writer = Arc::new(EventWriter::spawn(store.clone(), relay_tx));

    let codex_root = tmp.path();
    std::fs::create_dir_all(codex_root.join(".codex/sessions")).unwrap();
    std::fs::write(
        codex_root.join(".codex/sessions/sid.jsonl"),
        "\n\n   \n{\"k\":1}\n\n",
    )
    .unwrap();

    recover_with_root("thr-blank", &[], Some("sid"), codex_root, &writer)
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE thread_id = ?")
        .bind("thr-blank")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 1, "blank lines must be ignored");
}
