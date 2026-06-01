use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Sqlite, SqlitePool, Transaction};

use crate::error::BackendError;
use crate::store::StoreHandle;

pub enum DbTx<'a> {
    Postgres(Transaction<'a, Postgres>),
    Sqlite(Transaction<'a, Sqlite>),
}

impl<'a> DbTx<'a> {
    pub async fn commit(self) -> Result<(), BackendError> {
        match self {
            Self::Postgres(tx) => tx.commit().await.map_err(|error| BackendError::StoreQuery {
                operation: "db_tx.commit_postgres".into(),
                message: error.to_string(),
            }),
            Self::Sqlite(tx) => tx.commit().await.map_err(|error| BackendError::StoreQuery {
                operation: "db_tx.commit_sqlite".into(),
                message: error.to_string(),
            }),
        }
    }

    pub async fn rollback(self) -> Result<(), BackendError> {
        match self {
            Self::Postgres(tx) => tx.rollback().await.map_err(|error| BackendError::StoreQuery {
                operation: "db_tx.rollback_postgres".into(),
                message: error.to_string(),
            }),
            Self::Sqlite(tx) => tx.rollback().await.map_err(|error| BackendError::StoreQuery {
                operation: "db_tx.rollback_sqlite".into(),
                message: error.to_string(),
            }),
        }
    }
}

#[async_trait]
pub trait Storage: Send + Sync {
    async fn begin(&self) -> Result<DbTx<'_>, BackendError>;
}

#[async_trait]
impl Storage for StoreHandle {
    async fn begin(&self) -> Result<DbTx<'_>, BackendError> {
        match self {
            Self::Postgres(pool) => pool.begin().await.map(DbTx::Postgres).map_err(|error| {
                BackendError::StoreQuery {
                    operation: "storage.begin_postgres".into(),
                    message: error.to_string(),
                }
            }),
            Self::Sqlite(pool) => pool.begin().await.map(DbTx::Sqlite).map_err(|error| {
                BackendError::StoreQuery {
                    operation: "storage.begin_sqlite".into(),
                    message: error.to_string(),
                }
            }),
        }
    }
}

#[async_trait]
impl Storage for SqlitePool {
    async fn begin(&self) -> Result<DbTx<'_>, BackendError> {
        self.begin().await.map(DbTx::Sqlite).map_err(|error| BackendError::StoreQuery {
            operation: "storage.begin_sqlite".into(),
            message: error.to_string(),
        })
    }
}

#[async_trait]
impl Storage for PgPool {
    async fn begin(&self) -> Result<DbTx<'_>, BackendError> {
        self.begin().await.map(DbTx::Postgres).map_err(|error| BackendError::StoreQuery {
            operation: "storage.begin_postgres".into(),
            message: error.to_string(),
        })
    }
}