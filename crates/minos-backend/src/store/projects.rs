//! Account-scoped project CRUD.

use crate::store::{AsStorePool, StorePoolRef};

use crate::error::BackendError;

pub async fn create(
    store: &impl AsStorePool,
    project_id: &str,
    account_id: &str,
    name: &str,
    workspace_slug: &str,
    workspace_path: Option<&str>,
    now_ms: i64,
) -> Result<minos_protocol::ProjectSummary, BackendError> {
    // Owner membership is mandatory with projects.account_id SSOT — same tx.
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut tx = pool.begin().await.map_err(|e| BackendError::StoreQuery {
                operation: "projects.create.begin".into(),
                message: e.to_string(),
            })?;
            sqlx::query(
                r"INSERT INTO projects (project_id, account_id, name, workspace_slug, workspace_path, created_at_ms, updated_at_ms)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            )
            .bind(project_id)
            .bind(account_id)
            .bind(name)
            .bind(workspace_slug)
            .bind(workspace_path)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| BackendError::StoreQuery {
                operation: "projects.create".into(),
                message: e.to_string(),
            })?;
            sqlx::query(
                r"INSERT INTO project_members (project_id, account_id, role, joined_at_ms)
                  VALUES (?1, ?2, 'owner', ?3)",
            )
            .bind(project_id)
            .bind(account_id)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| BackendError::StoreQuery {
                operation: "projects.create.member".into(),
                message: e.to_string(),
            })?;
            tx.commit().await.map_err(|e| BackendError::StoreQuery {
                operation: "projects.create.commit".into(),
                message: e.to_string(),
            })?;
        }
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool.begin().await.map_err(|e| BackendError::StoreQuery {
                operation: "projects.create.begin".into(),
                message: e.to_string(),
            })?;
            sqlx::query(
                r"INSERT INTO projects (project_id, account_id, name, workspace_slug, workspace_path, created_at_ms, updated_at_ms)
                  VALUES ($1, $2, $3, $4, $5, $6, $6)",
            )
            .bind(project_id)
            .bind(account_id)
            .bind(name)
            .bind(workspace_slug)
            .bind(workspace_path)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| BackendError::StoreQuery {
                operation: "projects.create".into(),
                message: e.to_string(),
            })?;
            sqlx::query(
                r"INSERT INTO project_members (project_id, account_id, role, joined_at_ms)
                  VALUES ($1, $2, 'owner', $3)",
            )
            .bind(project_id)
            .bind(account_id)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| BackendError::StoreQuery {
                operation: "projects.create.member".into(),
                message: e.to_string(),
            })?;
            tx.commit().await.map_err(|e| BackendError::StoreQuery {
                operation: "projects.create.commit".into(),
                message: e.to_string(),
            })?;
        }
    }

    Ok(minos_protocol::ProjectSummary {
        project_id: project_id.to_string(),
        name: name.to_string(),
        workspace_slug: workspace_slug.to_string(),
        workspace_path: workspace_path.map(str::to_string),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        thread_count: 0,
    })
}

/// Internal row shape for repository/admin reads (includes owner + archive).
#[derive(Debug, Clone)]
pub struct ProjectRecord {
    pub project_id: String,
    pub account_id: String,
    pub name: String,
    pub workspace_slug: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub archived_at_ms: Option<i64>,
}

pub async fn find_record(
    store: &impl AsStorePool,
    project_id: &str,
) -> Result<Option<ProjectRecord>, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, (String, String, String, String, i64, i64, Option<i64>)>(
                r"SELECT project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms, archived_at_ms
                     FROM projects WHERE project_id = ?",
            )
            .bind(project_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, (String, String, String, String, i64, i64, Option<i64>)>(
                r"SELECT project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms, archived_at_ms
                     FROM projects WHERE project_id = $1",
            )
            .bind(project_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.find_record".into(),
        message: e.to_string(),
    })?;

    Ok(row.map(
        |(
            project_id,
            account_id,
            name,
            workspace_slug,
            created_at_ms,
            updated_at_ms,
            archived_at_ms,
        )| ProjectRecord {
            project_id,
            account_id,
            name,
            workspace_slug,
            created_at_ms,
            updated_at_ms,
            archived_at_ms,
        },
    ))
}

/// Cursor-paginated active projects for an account (archived excluded).
pub async fn list_records_for_account(
    store: &impl AsStorePool,
    account_id: &str,
    limit: u32,
    cursor: Option<&str>,
) -> Result<Vec<ProjectRecord>, BackendError> {
    let effective_limit = i64::from(limit.min(200));
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => match cursor {
            Some(cursor_id) => {
                sqlx::query_as::<_, (String, String, String, String, i64, i64, Option<i64>)>(
                    r"SELECT project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms, archived_at_ms
                         FROM projects
                        WHERE account_id = ? AND project_id > ? AND archived_at_ms IS NULL
                        ORDER BY project_id ASC LIMIT ?",
                )
                .bind(account_id)
                .bind(cursor_id)
                .bind(effective_limit)
                .fetch_all(pool)
                .await
            }
            None => {
                sqlx::query_as::<_, (String, String, String, String, i64, i64, Option<i64>)>(
                    r"SELECT project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms, archived_at_ms
                         FROM projects
                        WHERE account_id = ? AND archived_at_ms IS NULL
                        ORDER BY project_id ASC LIMIT ?",
                )
                .bind(account_id)
                .bind(effective_limit)
                .fetch_all(pool)
                .await
            }
        },
        StorePoolRef::Postgres(pool) => match cursor {
            Some(cursor_id) => {
                sqlx::query_as::<_, (String, String, String, String, i64, i64, Option<i64>)>(
                    r"SELECT project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms, archived_at_ms
                         FROM projects
                        WHERE account_id = $1 AND project_id > $2 AND archived_at_ms IS NULL
                        ORDER BY project_id ASC LIMIT $3",
                )
                .bind(account_id)
                .bind(cursor_id)
                .bind(effective_limit)
                .fetch_all(pool)
                .await
            }
            None => {
                sqlx::query_as::<_, (String, String, String, String, i64, i64, Option<i64>)>(
                    r"SELECT project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms, archived_at_ms
                         FROM projects
                        WHERE account_id = $1 AND archived_at_ms IS NULL
                        ORDER BY project_id ASC LIMIT $2",
                )
                .bind(account_id)
                .bind(effective_limit)
                .fetch_all(pool)
                .await
            }
        },
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.list_records_for_account".into(),
        message: e.to_string(),
    })?;

    Ok(rows
        .into_iter()
        .map(
            |(
                project_id,
                account_id,
                name,
                workspace_slug,
                created_at_ms,
                updated_at_ms,
                archived_at_ms,
            )| ProjectRecord {
                project_id,
                account_id,
                name,
                workspace_slug,
                created_at_ms,
                updated_at_ms,
                archived_at_ms,
            },
        )
        .collect())
}

/// Update name and/or workspace_slug for an owned project.
pub async fn update_fields(
    store: &impl AsStorePool,
    account_id: &str,
    project_id: &str,
    name: &str,
    workspace_slug: &str,
    at_ms: i64,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            r"UPDATE projects SET name = ?, workspace_slug = ?, updated_at_ms = ?
                   WHERE project_id = ? AND account_id = ?",
        )
        .bind(name)
        .bind(workspace_slug)
        .bind(at_ms)
        .bind(project_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            r"UPDATE projects SET name = $1, workspace_slug = $2, updated_at_ms = $3
                   WHERE project_id = $4 AND account_id = $5",
        )
        .bind(name)
        .bind(workspace_slug)
        .bind(at_ms)
        .bind(project_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.update_fields".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

pub async fn list(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<Vec<minos_protocol::ProjectSummary>, BackendError> {
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64, i64)>(
                r"SELECT
                      p.project_id,
                      p.name,
                      p.workspace_slug,
                      p.workspace_path,
                      p.created_at_ms,
                      p.updated_at_ms,
                                    COUNT(DISTINCT CASE WHEN cm.account_id IS NOT NULL THEN s.session_id END) AS thread_count
                  FROM projects p
                            LEFT JOIN agent_sessions s
                                ON s.project_id = p.project_id
                            LEFT JOIN conversation_members cm
                                ON cm.conversation_id = s.conversation_id
                             AND cm.account_id = p.account_id
                  WHERE p.account_id = ?1
                    AND p.archived_at_ms IS NULL
                  GROUP BY p.project_id
                  ORDER BY p.updated_at_ms DESC",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64, i64)>(
                r"SELECT
                      p.project_id,
                      p.name,
                      p.workspace_slug,
                      p.workspace_path,
                      p.created_at_ms,
                      p.updated_at_ms,
                                    COUNT(DISTINCT CASE WHEN cm.account_id IS NOT NULL THEN s.session_id END) AS thread_count
                  FROM projects p
                            LEFT JOIN agent_sessions s
                                ON s.project_id = p.project_id
                            LEFT JOIN conversation_members cm
                                ON cm.conversation_id = s.conversation_id
                             AND cm.account_id = p.account_id
                  WHERE p.account_id = $1
                    AND p.archived_at_ms IS NULL
                  GROUP BY p.project_id, p.name, p.workspace_slug, p.workspace_path, p.created_at_ms, p.updated_at_ms
                  ORDER BY p.updated_at_ms DESC",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.list".into(),
        message: e.to_string(),
    })?;

    rows.into_iter()
        .map(
            |(
                project_id,
                name,
                workspace_slug,
                workspace_path,
                created_at_ms,
                updated_at_ms,
                thread_count,
            )| {
                Ok(minos_protocol::ProjectSummary {
                    project_id,
                    name,
                    workspace_slug,
                    workspace_path,
                    created_at_ms,
                    updated_at_ms,
                    thread_count: u32::try_from(thread_count).unwrap_or(u32::MAX),
                })
            },
        )
        .collect()
}

pub async fn get_for_account(
    store: &impl AsStorePool,
    account_id: &str,
    project_id: &str,
) -> Result<Option<minos_protocol::ProjectSummary>, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query_as::<
            _,
            (String, String, String, Option<String>, i64, i64),
        >(
            r"SELECT project_id, name, workspace_slug, workspace_path, created_at_ms, updated_at_ms
                    FROM projects
                   WHERE account_id = ?1
                     AND project_id = ?2
                     AND archived_at_ms IS NULL",
        )
        .bind(account_id)
        .bind(project_id)
        .fetch_optional(pool)
        .await,
        StorePoolRef::Postgres(pool) => sqlx::query_as::<
            _,
            (String, String, String, Option<String>, i64, i64),
        >(
            r"SELECT project_id, name, workspace_slug, workspace_path, created_at_ms, updated_at_ms
                    FROM projects
                   WHERE account_id = $1
                     AND project_id = $2
                     AND archived_at_ms IS NULL",
        )
        .bind(account_id)
        .bind(project_id)
        .fetch_optional(pool)
        .await,
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.get_for_account".into(),
        message: e.to_string(),
    })?;

    Ok(row.map(
        |(project_id, name, workspace_slug, workspace_path, created_at_ms, updated_at_ms)| {
            minos_protocol::ProjectSummary {
                project_id,
                name,
                workspace_slug,
                workspace_path,
                created_at_ms,
                updated_at_ms,
                thread_count: 0,
            }
        },
    ))
}

pub async fn exists(
    store: &impl AsStorePool,
    account_id: &str,
    project_id: &str,
) -> Result<bool, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(
                    SELECT 1
                      FROM projects
                     WHERE project_id = ?1
                       AND account_id = ?2
                )",
        )
        .bind(project_id)
        .bind(account_id)
        .fetch_one(pool)
        .await
        .map(|exists| exists != 0),
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(
                    SELECT 1
                      FROM projects
                     WHERE project_id = $1
                       AND account_id = $2
                )",
            )
            .bind(project_id)
            .bind(account_id)
            .fetch_one(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.exists".into(),
        message: e.to_string(),
    })
}

pub async fn update_name(
    store: &impl AsStorePool,
    account_id: &str,
    project_id: &str,
    name: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            r"UPDATE projects
                  SET name = ?1, updated_at_ms = ?2
                  WHERE project_id = ?3 AND account_id = ?4",
        )
        .bind(name)
        .bind(now_ms)
        .bind(project_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            r"UPDATE projects
                  SET name = $1, updated_at_ms = $2
                  WHERE project_id = $3 AND account_id = $4",
        )
        .bind(name)
        .bind(now_ms)
        .bind(project_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.update_name".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Soft-archive a project. Idempotent when already archived.
pub async fn archive(
    store: &impl AsStorePool,
    account_id: &str,
    project_id: &str,
    at_ms: i64,
) -> Result<bool, BackendError> {
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            r"UPDATE projects SET archived_at_ms = ?1, updated_at_ms = ?1
               WHERE project_id = ?2 AND account_id = ?3 AND archived_at_ms IS NULL",
        )
        .bind(at_ms)
        .bind(project_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            r"UPDATE projects SET archived_at_ms = $1, updated_at_ms = $1
               WHERE project_id = $2 AND account_id = $3 AND archived_at_ms IS NULL",
        )
        .bind(at_ms)
        .bind(project_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.archive".into(),
        message: e.to_string(),
    })?;
    Ok(rows > 0)
}

pub async fn delete(
    store: &impl AsStorePool,
    account_id: &str,
    project_id: &str,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(r"DELETE FROM projects WHERE project_id = ?1 AND account_id = ?2")
                .bind(project_id)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(r"DELETE FROM projects WHERE project_id = $1 AND account_id = $2")
                .bind(project_id)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.delete".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_account, memory_pool};

    #[tokio::test]
    async fn list_empty_projects_when_account_has_no_rows() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "empty-projects@example.com").await;

        let projects = list(&pool, &account_id).await.unwrap();

        assert!(projects.is_empty());
    }

    #[tokio::test]
    async fn list_is_account_scoped_and_counts_project_sessions() {
        let pool = memory_pool().await;
        let account_a = insert_account(&pool, "project-a@example.com").await;
        let account_b = insert_account(&pool, "project-b@example.com").await;

        create(
            &pool,
            "proj-a",
            &account_a,
            "Project A",
            "proj-a",
            Some("/Users/example/proj-a"),
            1000,
        )
        .await
        .unwrap();
        create(
            &pool,
            "proj-b",
            &account_b,
            "Project B",
            "proj-b",
            None,
            2000,
        )
        .await
        .unwrap();
        let convo_a = crate::store::social::create_group_conversation(
            &pool,
            &account_a,
            "Project A",
            &[],
            3000,
        )
        .await
        .unwrap();
        let convo_b = crate::store::social::create_group_conversation(
            &pool,
            &account_b,
            "Project B",
            &[],
            4000,
        )
        .await
        .unwrap();
        crate::store::agent_sessions::create(
            &pool,
            "sess-a",
            &convo_a.conversation_id,
            Some("proj-a"),
            None,
            None,
            "running",
            5000,
            None,
        )
        .await
        .unwrap();
        crate::store::agent_sessions::create(
            &pool,
            "sess-b",
            &convo_b.conversation_id,
            Some("proj-b"),
            None,
            None,
            "running",
            6000,
            None,
        )
        .await
        .unwrap();

        let projects = list(&pool, &account_a).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].project_id, "proj-a");
        assert_eq!(projects[0].thread_count, 1);
        assert!(exists(&pool, &account_a, "proj-a").await.unwrap());
        assert!(!exists(&pool, &account_a, "proj-b").await.unwrap());
    }

    #[tokio::test]
    async fn create_inserts_owner_project_member_and_archive_hides_from_list() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "archive-projects@example.com").await;
        create(
            &pool,
            "proj-arch",
            &account_id,
            "Archivable",
            "archivable",
            None,
            1000,
        )
        .await
        .unwrap();

        let members: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM project_members WHERE project_id = ? AND role = 'owner'",
        )
        .bind("proj-arch")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(members, 1);

        assert_eq!(list(&pool, &account_id).await.unwrap().len(), 1);
        assert!(archive(&pool, &account_id, "proj-arch", 2000)
            .await
            .unwrap());
        assert!(list(&pool, &account_id).await.unwrap().is_empty());
        assert!(get_for_account(&pool, &account_id, "proj-arch")
            .await
            .unwrap()
            .is_none());
        // Second archive is a no-op.
        assert!(!archive(&pool, &account_id, "proj-arch", 3000)
            .await
            .unwrap());
    }
}
