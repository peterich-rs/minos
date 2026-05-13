//! Account-scoped project CRUD.

use sqlx::SqlitePool;

use crate::error::BackendError;

pub async fn create(
    pool: &SqlitePool,
    project_id: &str,
    account_id: &str,
    name: &str,
    workspace_slug: &str,
    now_ms: i64,
) -> Result<minos_protocol::ProjectSummary, BackendError> {
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
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Vec<minos_protocol::ProjectSummary>, BackendError> {
    let rows = sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(
        r"SELECT
              p.project_id,
              p.name,
              p.workspace_slug,
              p.created_at_ms,
              p.updated_at_ms,
              COUNT(pt.thread_id) AS thread_count
          FROM projects p
          LEFT JOIN project_threads pt
            ON pt.project_id = p.project_id
           AND pt.account_id = p.account_id
          WHERE p.account_id = ?1
          GROUP BY p.project_id
          ORDER BY p.updated_at_ms DESC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
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

pub async fn update_name(
    pool: &SqlitePool,
    account_id: &str,
    project_id: &str,
    name: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
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
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.update_name".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

pub async fn delete(
    pool: &SqlitePool,
    account_id: &str,
    project_id: &str,
) -> Result<(), BackendError> {
    sqlx::query(r"DELETE FROM projects WHERE project_id = ?1 AND account_id = ?2")
        .bind(project_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "projects.delete".into(),
            message: e.to_string(),
        })?;
    Ok(())
}

pub async fn assign_thread(
    pool: &SqlitePool,
    account_id: &str,
    project_id: &str,
    thread_id: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        r"INSERT INTO project_threads (project_id, thread_id, account_id, linked_at_ms)
          SELECT project_id, ?3, account_id, ?4
          FROM projects
          WHERE project_id = ?1 AND account_id = ?2
          ON CONFLICT(project_id, thread_id) DO UPDATE SET
            account_id = excluded.account_id,
            linked_at_ms = excluded.linked_at_ms",
    )
    .bind(project_id)
    .bind(account_id)
    .bind(thread_id)
    .bind(now_ms)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "projects.assign_thread".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

pub async fn list_threads(
    pool: &SqlitePool,
    account_id: &str,
    project_id: &str,
    before_ts_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<minos_protocol::ThreadSummary>, BackendError> {
    crate::store::threads::list_by_project(pool, account_id, project_id, before_ts_ms, limit).await
}

#[cfg(test)]
mod tests {
    use minos_domain::AgentName;

    use super::*;
    use crate::store::test_support::{insert_account, memory_pool};

    async fn insert_host_device(pool: &SqlitePool, account_id: &str, device_id: &str) {
        sqlx::query(
            r"INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at, account_id)
              VALUES (?1, 'Mac', 'agent-host', 0, 0, ?2)",
        )
        .bind(device_id)
        .bind(account_id)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_empty_projects_when_account_has_no_rows() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "empty-projects@example.com").await;

        let projects = list(&pool, &account_id).await.unwrap();

        assert!(projects.is_empty());
    }

    #[tokio::test]
    async fn list_is_account_scoped_and_counts_assigned_threads() {
        let pool = memory_pool().await;
        let account_a = insert_account(&pool, "project-a@example.com").await;
        let account_b = insert_account(&pool, "project-b@example.com").await;
        insert_host_device(&pool, &account_a, "host-a").await;
        insert_host_device(&pool, &account_b, "host-b").await;

        create(&pool, "proj-a", &account_a, "Project A", "proj-a", 1000)
            .await
            .unwrap();
        create(&pool, "proj-b", &account_b, "Project B", "proj-b", 2000)
            .await
            .unwrap();
        crate::store::threads::upsert(&pool, "thr-a", AgentName::Codex, "host-a", 3000)
            .await
            .unwrap();
        crate::store::threads::upsert(&pool, "thr-b", AgentName::Codex, "host-b", 4000)
            .await
            .unwrap();
        assign_thread(&pool, &account_a, "proj-a", "thr-a", 5000)
            .await
            .unwrap();
        assign_thread(&pool, &account_b, "proj-b", "thr-b", 6000)
            .await
            .unwrap();

        let projects = list(&pool, &account_a).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].project_id, "proj-a");
        assert_eq!(projects[0].thread_count, 1);

        let threads = list_threads(&pool, &account_a, "proj-a", None, 50)
            .await
            .unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, "thr-a");
    }
}
