use std::sync::Arc;

use async_trait::async_trait;
use minos_protocol::UserSummary;

use crate::error::BackendError;
use crate::store::{social, StoreHandle};

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile_not_found")]
    NotFound,
    #[error("minos_id_taken")]
    MinosIdTaken,
    #[error("validation_format: {0}")]
    ValidationFormat(&'static str),
    #[error(transparent)]
    Internal(#[from] BackendError),
}

#[async_trait]
pub trait ProfileService: Send + Sync {
    async fn get_my_profile(&self, account_id: &str) -> Result<ProfileDto, ProfileError>;

    async fn set_minos_id(
        &self,
        account_id: &str,
        minos_id: &str,
    ) -> Result<ProfileDto, ProfileError>;

    async fn set_display_name(
        &self,
        account_id: &str,
        display_name: Option<&str>,
    ) -> Result<ProfileDto, ProfileError>;

    async fn search_users(
        &self,
        query: &str,
        exclude_account_id: &str,
    ) -> Result<Vec<UserSummary>, ProfileError>;
}

#[derive(Debug, Clone)]
pub struct ProfileDto {
    pub account_id: String,
    pub email: String,
    pub minos_id: String,
    pub display_name: Option<String>,
}

pub struct DefaultProfileService {
    store: StoreHandle,
}

impl DefaultProfileService {
    #[must_use]
    pub fn new(store: StoreHandle) -> Arc<Self> {
        Arc::new(Self { store })
    }
}

pub fn display_name(profile: &social::ProfileRow) -> String {
    if let Some(name) = profile.display_name.as_deref() {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let email = profile.email.trim();
    match email.split('@').next() {
        Some(head) if !head.is_empty() => head.to_string(),
        _ => profile.minos_id.clone(),
    }
}

pub(crate) fn to_user_summary(profile: &social::ProfileRow) -> UserSummary {
    UserSummary {
        account_id: profile.account_id.clone(),
        minos_id: profile.minos_id.clone(),
        display_name: display_name(profile),
    }
}

fn to_profile_dto(profile: &social::ProfileRow) -> ProfileDto {
    ProfileDto {
        account_id: profile.account_id.clone(),
        email: profile.email.clone(),
        minos_id: profile.minos_id.clone(),
        display_name: profile.display_name.clone(),
    }
}

#[async_trait]
impl ProfileService for DefaultProfileService {
    async fn get_my_profile(&self, account_id: &str) -> Result<ProfileDto, ProfileError> {
        let profile = social::profile_by_account(&self.store, account_id)
            .await?
            .ok_or(ProfileError::NotFound)?;
        Ok(to_profile_dto(&profile))
    }

    async fn set_minos_id(
        &self,
        account_id: &str,
        minos_id: &str,
    ) -> Result<ProfileDto, ProfileError> {
        if !validate_minos_id(minos_id) {
            return Err(ProfileError::ValidationFormat(
                "minos_id must be 6-24 ASCII letters or digits",
            ));
        }
        social::set_minos_id(&self.store, account_id, minos_id)
            .await
            .map_err(|e| match &e {
                BackendError::StoreQuery { operation, message }
                    if operation == "social::set_minos_id" && message == "minos_id_taken" =>
                {
                    ProfileError::MinosIdTaken
                }
                _ => ProfileError::Internal(e),
            })?;
        self.get_my_profile(account_id).await
    }

    async fn set_display_name(
        &self,
        account_id: &str,
        display_name: Option<&str>,
    ) -> Result<ProfileDto, ProfileError> {
        let next_display_name = display_name
            .map(|raw| raw.trim().to_string())
            .filter(|trimmed| !trimmed.is_empty());
        if let Some(name) = next_display_name.as_ref() {
            let char_count = name.chars().count();
            if !(1..=48).contains(&char_count) {
                return Err(ProfileError::ValidationFormat(
                    "display_name must be 1-48 characters after trimming",
                ));
            }
        }
        social::set_display_name(&self.store, account_id, next_display_name.as_deref()).await?;
        self.get_my_profile(account_id).await
    }

    async fn search_users(
        &self,
        query: &str,
        exclude_account_id: &str,
    ) -> Result<Vec<UserSummary>, ProfileError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let users = social::search_by_minos_id_prefix(&self.store, query)
            .await?
            .into_iter()
            .filter(|user| user.account_id != exclude_account_id)
            .map(|user| to_user_summary(&user))
            .collect();
        Ok(users)
    }
}

fn validate_minos_id(minos_id: &str) -> bool {
    let len = minos_id.len();
    (6..=24).contains(&len) && minos_id.bytes().all(|b| b.is_ascii_alphanumeric())
}
