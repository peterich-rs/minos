//! Postgres migration smoke. Skipped unless `MINOS_PG_TESTS=1`.
//!
//! CI should export MINOS_PG_TESTS=1 with a postgres:16 service and
//! `MINOS_DATABASE_URL=postgres://…`.

use minos_backend::store;

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

#[tokio::test]
async fn postgres_migration_applies_parity_tables() {
    if !pg_tests_enabled() {
        eprintln!("skipping: set MINOS_PG_TESTS=1 and MINOS_DATABASE_URL");
        return;
    }
    let url = database_url().expect("MINOS_DATABASE_URL required when MINOS_PG_TESTS=1");
    let pool = store::connect_postgres_with_options(&url, 4)
        .await
        .expect("connect postgres");

    for table in [
        "accounts",
        "devices",
        "projects",
        "project_members",
        "audit_events",
        "agent_sessions",
        "outbox_events",
        "durable_event_log",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                 WHERE table_schema = 'public' AND table_name = $1
            )",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "table {table} missing after migrate");
    }

    // Retired tables must not exist.
    for table in ["pending_approvals", "project_sessions"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                 WHERE table_schema = 'public' AND table_name = $1
            )",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!exists, "retired table {table} must not exist");
    }

    pool.close().await;
}
