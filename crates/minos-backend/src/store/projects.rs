//! Account-scoped project CRUD.

use crate::store::{AsStorePool, StorePoolRef};

use crate::error::BackendError;

pub async fn create(
    store: &impl AsStorePool,
    project_id: &str,
    account_id: &str,
    name: &str,
    workspace_slug: &str,
    now_ms: i64,
) -> Result<minos_protocol::ProjectSummary, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                r"INSERT INTO projects (project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            )
            .bind(project_id)
            .bind(account_id)
            .bind(name)
            .bind(workspace_slug)
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                r"INSERT INTO projects (project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms)
                  VALUES ($1, $2, $3, $4, $5, $5)",
            )
            .bind(project_id)
            .bind(account_id)
            .bind(name)
            .bind(workspace_slug)
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.create".into(),
        message: e.to_string(),
    })?;

    Ok(minos_protocol::ProjectSummary {
        project_id: project_id.to_string(),
        name: name.to_string(),
        workspace_slug: workspace_slug.to_string(),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        thread_count: 0,
    })
}

pub async fn list(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<Vec<minos_protocol::ProjectSummary>, BackendError> {
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(
                r"SELECT
                      p.project_id,
                      p.name,
                      p.workspace_slug,
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
                  GROUP BY p.project_id
                  ORDER BY p.updated_at_ms DESC",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(
                r"SELECT
                      p.project_id,
                      p.name,
                      p.workspace_slug,
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
                  GROUP BY p.project_id
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
            |(project_id, name, workspace_slug, created_at_ms, updated_at_ms, thread_count)| {
                Ok(minos_protocol::ProjectSummary {
                    project_id,
                    name,
                    workspace_slug,
                    created_at_ms,
                    updated_at_ms,
                    thread_count: u32::try_from(thread_count).unwrap_or(u32::MAX),
                })
            },
        )
        .collect()
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

        create(&pool, "proj-a", &account_a, "Project A", "proj-a", 1000)
            .await
            .unwrap();
        create(&pool, "proj-b", &account_b, "Project B", "proj-b", 2000)
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
            Some("agent_codex"),
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
            Some("agent_codex"),
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
}
