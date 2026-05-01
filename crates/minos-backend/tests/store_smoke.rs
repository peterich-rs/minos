use tempfile::tempdir;

#[tokio::test]
async fn connect_creates_tables_and_migrates() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("smoke.db");
    let url = format!("sqlite://{}", db.display());

    let pool = minos_backend::store::connect(&url).await.unwrap();

    // ADR-0020 / Phase F+H1: legacy device-keyed `pairings` table was
    // dropped in migration 0011 and replaced by `account_mac_pairings`
    // (migration 0012). Smoke-test the post-ADR schema.
    for table in [
        "devices",
        "accounts",
        "account_mac_pairings",
        "pairing_tokens",
    ] {
        let row: Option<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert!(row.is_some(), "table {table} missing after migrate");
    }

    // Spot-check an index from the new pair table to confirm migration 0012
    // ran cleanly. STRICT mode has no reflection API, but the CHECK
    // constraints embedded in STRICT rejections are exercised by store
    // submodule tests.
    let idx: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_amp_mobile_account'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(
        idx.is_some(),
        "idx_amp_mobile_account missing after migrate"
    );
}
