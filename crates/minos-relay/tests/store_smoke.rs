use tempfile::tempdir;

#[tokio::test]
async fn connect_creates_tables_and_migrates() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("smoke.db");
    let url = format!("sqlite://{}", db.display());

    let pool = minos_relay::store::connect(&url).await.unwrap();

    for table in ["devices", "pairings", "pairing_tokens"] {
        let row: Option<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert!(row.is_some(), "table {table} missing after migrate");
    }

    // Spot-check an index and STRICT mode (best-effort: STRICT has no reflection
    // API, but the CHECK constraints embedded in STRICT rejections will be
    // exercised by later store tests in step 5).
    let idx: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_pairings_a'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(idx.is_some(), "idx_pairings_a missing after migrate");
}
