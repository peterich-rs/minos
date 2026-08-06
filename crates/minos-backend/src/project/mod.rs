use std::sync::Arc;

use async_trait::async_trait;

use crate::error::BackendError;
use crate::store::StoreHandle;

const PROJECT_REPO_METRIC_LABEL: &str = "project_repo";

#[derive(Debug)]
pub enum ProjectError {
    InvalidInput(&'static str),
    Internal(BackendError),
}

#[async_trait]
pub trait ProjectRepo: Send + Sync {
    async fn list(
        &self,
        account_id: &str,
    ) -> Result<Vec<minos_protocol::ProjectSummary>, BackendError>;

    async fn create(
        &self,
        account_id: &str,
        project_id: &str,
        name: &str,
        workspace_slug: &str,
        workspace_path: Option<&str>,
        now_ms: i64,
    ) -> Result<minos_protocol::ProjectSummary, BackendError>;

    async fn update_name(
        &self,
        account_id: &str,
        project_id: &str,
        name: &str,
        now_ms: i64,
    ) -> Result<(), BackendError>;

    async fn delete(&self, account_id: &str, project_id: &str) -> Result<(), BackendError>;

    async fn archive(
        &self,
        account_id: &str,
        project_id: &str,
        at_ms: i64,
    ) -> Result<bool, BackendError>;
}

struct SqlProjectRepo {
    store: StoreHandle,
}

impl SqlProjectRepo {
    fn new(store: StoreHandle) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ProjectRepo for SqlProjectRepo {
    async fn list(
        &self,
        account_id: &str,
    ) -> Result<Vec<minos_protocol::ProjectSummary>, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(PROJECT_REPO_METRIC_LABEL, "list");
        crate::store::projects::list(&self.store, account_id).await
    }

    async fn create(
        &self,
        account_id: &str,
        project_id: &str,
        name: &str,
        workspace_slug: &str,
        workspace_path: Option<&str>,
        now_ms: i64,
    ) -> Result<minos_protocol::ProjectSummary, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(PROJECT_REPO_METRIC_LABEL, "create");
        crate::store::projects::create(
            &self.store,
            project_id,
            account_id,
            name,
            workspace_slug,
            workspace_path,
            now_ms,
        )
        .await
    }

    async fn update_name(
        &self,
        account_id: &str,
        project_id: &str,
        name: &str,
        now_ms: i64,
    ) -> Result<(), BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(PROJECT_REPO_METRIC_LABEL, "update_name");
        crate::store::projects::update_name(&self.store, account_id, project_id, name, now_ms).await
    }

    async fn delete(&self, account_id: &str, project_id: &str) -> Result<(), BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(PROJECT_REPO_METRIC_LABEL, "delete");
        crate::store::projects::delete(&self.store, account_id, project_id).await
    }

    async fn archive(
        &self,
        account_id: &str,
        project_id: &str,
        at_ms: i64,
    ) -> Result<bool, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(PROJECT_REPO_METRIC_LABEL, "archive");
        crate::store::projects::archive(&self.store, account_id, project_id, at_ms).await
    }
}

pub struct ProjectService {
    repo: Arc<dyn ProjectRepo>,
}

impl ProjectService {
    #[must_use]
    pub fn new(store: impl Into<StoreHandle>) -> Arc<Self> {
        Self::with_repo(Arc::new(SqlProjectRepo::new(store.into())))
    }

    #[must_use]
    pub fn with_repo(repo: Arc<dyn ProjectRepo>) -> Arc<Self> {
        Arc::new(Self { repo })
    }

    pub async fn list(
        &self,
        account_id: &str,
    ) -> Result<Vec<minos_protocol::ProjectSummary>, ProjectError> {
        self.repo
            .list(account_id)
            .await
            .map_err(ProjectError::Internal)
    }

    pub async fn create(
        &self,
        account_id: &str,
        req: minos_protocol::CreateProjectRequest,
    ) -> Result<minos_protocol::ProjectSummary, ProjectError> {
        let name = req.name.trim();
        let workspace_slug = req.workspace_slug.trim();
        let workspace_path = normalize_workspace_path(req.workspace_path.as_deref());
        if name.is_empty() || !valid_workspace_slug(workspace_slug) {
            return Err(ProjectError::InvalidInput(
                "project name and a valid workspace_slug are required",
            ));
        }
        if let Some(path) = workspace_path.as_deref() {
            if !valid_workspace_path(path) {
                return Err(ProjectError::InvalidInput(
                    "workspace_path must be an absolute host path or ~/ path",
                ));
            }
        }

        self.repo
            .create(
                account_id,
                &uuid::Uuid::new_v4().to_string(),
                name,
                workspace_slug,
                workspace_path.as_deref(),
                chrono::Utc::now().timestamp_millis(),
            )
            .await
            .map_err(ProjectError::Internal)
    }

    pub async fn update_name(
        &self,
        account_id: &str,
        req: minos_protocol::UpdateProjectRequest,
    ) -> Result<(), ProjectError> {
        let name = req.name.trim();
        if name.is_empty() {
            return Err(ProjectError::InvalidInput("project name is required"));
        }

        self.repo
            .update_name(
                account_id,
                &req.project_id,
                name,
                chrono::Utc::now().timestamp_millis(),
            )
            .await
            .map_err(ProjectError::Internal)
    }

    pub async fn delete(&self, account_id: &str, project_id: &str) -> Result<(), ProjectError> {
        self.repo
            .delete(account_id, project_id)
            .await
            .map_err(ProjectError::Internal)
    }

    /// Soft-archive a project so default list filters hide it.
    pub async fn archive(
        &self,
        account_id: &str,
        project_id: &str,
    ) -> Result<bool, ProjectError> {
        self.repo
            .archive(
                account_id,
                project_id,
                chrono::Utc::now().timestamp_millis(),
            )
            .await
            .map_err(ProjectError::Internal)
    }
}

fn valid_workspace_slug(slug: &str) -> bool {
    !slug.is_empty() && slug != "." && slug != ".." && !slug.contains('/') && !slug.contains('\\')
}

fn normalize_workspace_path(path: Option<&str>) -> Option<String> {
    path.map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

fn valid_workspace_path(path: &str) -> bool {
    (path.starts_with('/') || path.starts_with("~/"))
        && !path.contains('\0')
        && !path.ends_with("/.")
        && !path.ends_with("/..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_account, memory_pool};

    #[tokio::test]
    async fn create_rejects_invalid_workspace_slug() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "project-service@example.com").await;
        let service = ProjectService::new(pool);

        let error = service
            .create(
                &account_id,
                minos_protocol::CreateProjectRequest {
                    name: "Workspace".to_string(),
                    workspace_slug: "../bad".to_string(),
                    workspace_path: None,
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(error, ProjectError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn list_records_project_repo_db_metric() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "project-service-metrics@example.com").await;
        let service = ProjectService::new(pool);

        let projects = service.list(&account_id).await.unwrap();
        assert!(projects.is_empty());

        let metrics = crate::telemetry::render();
        assert!(metrics.contains("minos_backend_db_query_duration_seconds"));
        assert!(metrics.contains("repo=\"project_repo\""));
        assert!(metrics.contains("op=\"list\""));
    }
}
