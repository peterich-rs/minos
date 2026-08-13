//! Postgres contract smoke. Skipped unless `MINOS_PG_TESTS=1`.

use minos_backend::store::{self, StoreHandle};
use minos_domain::{DeviceId, DeviceRole};

fn pg_tests_enabled() -> bool {
    std::env::var("MINOS_PG_TESTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn database_url() -> Option<String> {
    std::env::var("MINOS_DATABASE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
}

async fn connect() -> Option<StoreHandle> {
    if !pg_tests_enabled() {
        eprintln!("skipping: set MINOS_PG_TESTS=1 and MINOS_DATABASE_URL");
        return None;
    }
    let url = database_url().expect("MINOS_DATABASE_URL required when MINOS_PG_TESTS=1");
    let pool = store::connect_postgres_with_options(&url, 4)
        .await
        .expect("connect postgres");
    Some(StoreHandle::from(pool))
}

#[tokio::test]
async fn pg_strict_installation_check_and_projects_archive() {
    let Some(store) = connect().await else {
        return;
    };
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let email = format!("pg-smoke-{suffix}@example.com");
    let account = store::accounts::create(&store, &email).await.unwrap();
    let account_id = account.account_id;

    // Strict CHECK: host without key must fail.
    let bad_host = DeviceId::new();
    let host_fail = sqlx::query(
        r#"INSERT INTO devices
            (device_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id)
           VALUES ($1, 'host', 'bad', NULL, 1, 1, NULL)"#,
    )
    .bind(bad_host.to_string())
    .execute(store.postgres_pool().unwrap())
    .await;
    assert!(
        host_fail.is_err(),
        "host without public_key must fail CHECK"
    );

    // Strict CHECK: client without account_id must fail.
    let bad_client = DeviceId::new();
    let client_fail = sqlx::query(
        r#"INSERT INTO devices
            (device_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id)
           VALUES ($1, 'mobile', 'bad', NULL, 1, 1, NULL)"#,
    )
    .bind(bad_client.to_string())
    .execute(store.postgres_pool().unwrap())
    .await;
    assert!(
        client_fail.is_err(),
        "client without account_id must fail CHECK"
    );

    // Strict host + client.
    let host = DeviceId::new();
    store::devices::insert_host_with_public_key(
        &store,
        host,
        "pg-host",
        store::test_support::TEST_HOST_PUBLIC_KEY,
        1000,
    )
    .await
    .unwrap();
    let client = DeviceId::new();
    store::devices::insert_client_for_account(
        &store,
        client,
        "pg-phone",
        DeviceRole::MobileClient,
        &account_id,
        1001,
    )
    .await
    .unwrap();

    // Project create → owner membership + archive filter.
    let project_id = format!("proj-{suffix}");
    store::projects::create(
        &store,
        &project_id,
        &account_id,
        "PG Project",
        &format!("slug-{suffix}"),
        None,
        2000,
    )
    .await
    .unwrap();
    let members: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM project_members WHERE project_id = $1 AND role = 'owner'",
    )
    .bind(&project_id)
    .fetch_one(store.postgres_pool().unwrap())
    .await
    .unwrap();
    assert_eq!(members, 1);

    assert!(
        store::projects::archive(&store, &account_id, &project_id, 3000)
            .await
            .unwrap()
    );
    let listed = store::projects::list(&store, &account_id).await.unwrap();
    assert!(
        listed.iter().all(|p| p.project_id != project_id),
        "archived project hidden from list"
    );

    // Refresh rotate writes rotated_to_hash.
    let plain = store::refresh_tokens::generate_plaintext();
    store::refresh_tokens::insert(&store, &plain, &account_id, &client.to_string())
        .await
        .unwrap();
    let plain2 = store::refresh_tokens::generate_plaintext();
    let rotated =
        store::refresh_tokens::rotate(&store, &plain, &plain2, &account_id, &client.to_string())
            .await
            .unwrap();
    assert!(rotated.is_some());
    let old_hash = store::refresh_tokens::hash_plaintext(&plain);
    let rotated_to: Option<String> =
        sqlx::query_scalar("SELECT rotated_to_hash FROM refresh_tokens WHERE token_hash = $1")
            .bind(&old_hash)
            .fetch_one(store.postgres_pool().unwrap())
            .await
            .unwrap();
    assert_eq!(
        rotated_to.as_deref(),
        Some(store::refresh_tokens::hash_plaintext(&plain2).as_str())
    );

    // thread_sync running bool
    store::thread_sync_state::upsert_manifest(
        &store,
        &minos_protocol::realtime::HostGapManifest {
            manifest_id: format!("m-{suffix}"),
            host_id: host,
            sessions: vec![minos_protocol::realtime::SessionGapManifest {
                session_id: format!("sess-{suffix}"),
                backend_acked_seq: 0,
                local_from_seq: 1,
                local_to_seq: 1,
                missing_ranges: vec![],
                bytes: 0,
                event_count: 0,
                first_ts_ms: 1,
                last_ts_ms: 1,
                running: true,
            }],
        },
        4000,
    )
    .await
    .unwrap();

    // Durable log legal topic kinds (partitions).
    for kind in [
        "account",
        "conversation",
        "project",
        "agent_session",
        "host",
    ] {
        store::durable_event_log::append(
            &store,
            &format!("evt-{kind}-{suffix}"),
            &format!("{kind}:x"),
            kind,
            1,
            "pk",
            &serde_json::json!({"k": kind}),
            5000,
        )
        .await
        .unwrap_or_else(|e| panic!("append topic_kind={kind}: {e}"));
    }

    // agent_sessions.agent_id FK: invalid id fails; valid host_runtime agent ok.
    let conversation =
        store::social::create_group_conversation(&store, &account_id, "pg-smoke-group", &[], 6000)
            .await
            .unwrap();
    let bad_session = store::agent_sessions::create(
        &store,
        &format!("sess-bad-{suffix}"),
        &conversation.conversation_id,
        Some(&project_id),
        Some(&host.to_string()),
        Some("agent_does_not_exist"),
        "pending",
        6100,
        None,
    )
    .await;
    assert!(
        bad_session.is_err(),
        "invalid agent_id must fail FK on agent_sessions"
    );

    let agent = store::social::ensure_host_runtime_agent(
        &store,
        &account_id,
        "codex",
        "codex",
        "",
        None,
        6200,
    )
    .await
    .unwrap();
    store::agent_sessions::create(
        &store,
        &format!("sess-ok-{suffix}"),
        &conversation.conversation_id,
        Some(&project_id),
        Some(&host.to_string()),
        Some(agent.agent_id.as_str()),
        "pending",
        6300,
        None,
    )
    .await
    .unwrap();

    store.close().await;
}
