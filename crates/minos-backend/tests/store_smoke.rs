use tempfile::tempdir;

#[tokio::test]
async fn connect_creates_tables_and_migrates() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("smoke.db");
    let url = format!("sqlite://{}", db.display());

    let pool = minos_backend::store::connect(&url).await.unwrap();

    // Latest-only schema: host link tables without QR pairing_codes/tokens.
    for table in [
        "device_installations",
        "accounts",
        "host_links",
        "agent_sessions",
        "agent_turns",
        "agent_turn_events",
        "host_installation_tokens",
        "host_commands",
        "durable_event_log",
        "outbox_events",
        "audit_events",
        "project_members",
        "projects",
        "agents",
        "chat_message_mentions",
        "bot_message_deliveries",
        "bot_revisions",
        "bot_deployments",
    ] {
        let row: Option<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert!(row.is_some(), "table {table} missing after migrate");
    }

    // Spot-check named indexes (unique on agent_turns is table-level UNIQUE).
    for index in [
        "idx_host_links_account",
        "idx_agent_sessions_conversation_status",
        "idx_host_commands_host_status_deadline",
        "idx_durable_event_log_topic_seq",
        "idx_outbox_events_status_available",
        "idx_agents_owner_name_active",
        "idx_bot_message_deliveries_due",
        "idx_bot_revisions_agent_created",
        "idx_bot_deployments_host",
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
