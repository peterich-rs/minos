use tempfile::tempdir;

#[tokio::test]
async fn connect_creates_tables_and_migrates() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("smoke.db");
    let url = format!("sqlite://{}", db.display());

    let pool = minos_backend::store::connect(&url).await.unwrap();

    // ADR-0020 / Phase F+H1: legacy device-keyed `pairings` table was
    // dropped in migration 0011 and replaced by `account_mac_pairings`
    // (migration 0012). Migration 0013 renamed that table (and its
    // device column) to the host-prefixed names. Smoke-test the
    // post-rename schema.
    for table in [
        "devices",
        "accounts",
        "account_host_pairings",
        "agent_sessions",
        "agent_turns",
        "agent_turn_events",
        "pairing_tokens",
        "pairing_codes",
        "host_installation_tokens",
        "host_commands",
        "durable_event_log",
        "outbox_events",
    ] {
        let row: Option<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert!(row.is_some(), "table {table} missing after migrate");
    }

    // Spot-check an index from the renamed pair table to confirm migrations
    // 0012 + 0013 ran cleanly. STRICT mode has no reflection API, but the
    // CHECK constraints embedded in STRICT rejections are exercised by
    // store submodule tests.
    for index in [
        "idx_account_host_pairings_account",
        "idx_agent_sessions_conversation_status",
        "idx_agent_turns_session_seq",
        "idx_host_commands_host_status_deadline",
        "idx_durable_event_log_topic_seq",
        "idx_outbox_events_status_available",
    ] {
        let idx: Option<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='index' AND name=?")
                .bind(index)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert!(idx.is_some(), "index {index} missing after migrate");
    }
}

#[tokio::test]
async fn connect_enables_sqlite_write_contention_pragmas() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("pragmas.db");
    let url = format!("sqlite://{}", db.display());

    let pool = minos_backend::store::connect(&url).await.unwrap();

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

    let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(busy_timeout, 5_000);

    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(synchronous, 1, "NORMAL synchronous expected");

    let temp_store: i64 = sqlx::query_scalar("PRAGMA temp_store")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(temp_store, 2, "MEMORY temp_store expected");
}
