pub mod migrations_loader;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

#[derive(Clone)]
pub struct LocalStore {
    pool: SqlitePool,
}

impl LocalStore {
    pub async fn open(db_file: &Path) -> anyhow::Result<Self> {
        let url = format!("sqlite://{}?mode=rwc", db_file.display());
        let opts = SqliteConnectOptions::from_str(&url)?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("test.sqlite");
        let store = LocalStore::open(&p).await.unwrap();
        let row: (i64,) = sqlx::query_as("SELECT count(*) FROM events")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }
}
