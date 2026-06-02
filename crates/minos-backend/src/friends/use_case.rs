use std::sync::Arc;

use async_trait::async_trait;
use minos_protocol::{FriendRequestStatus, FriendRequestSummary, FriendSummary};

use crate::error::BackendError;
use crate::profiles::use_case::to_user_summary;
use crate::store::{social, StoreHandle};

#[derive(Debug, thiserror::Error)]
pub enum FriendError {
    #[error("target_user_not_found")]
    TargetNotFound,
    #[error("cannot_add_yourself")]
    CannotAddSelf,
    #[error("already_friends")]
    AlreadyFriends,
    #[error("friend_request_already_pending")]
    RequestAlreadyPending,
    #[error("friend_request_not_found")]
    RequestNotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("already_resolved")]
    AlreadyResolved,
    #[error(transparent)]
    Internal(#[from] BackendError),
}

#[derive(Debug, Clone)]
pub struct FriendRequestsResult {
    pub incoming: Vec<FriendRequestSummary>,
    pub outgoing: Vec<FriendRequestSummary>,
}

#[async_trait]
pub trait FriendService: Send + Sync {
    async fn create_request(
        &self,
        from_account_id: &str,
        target_minos_id: &str,
    ) -> Result<FriendRequestSummary, FriendError>;

    async fn list_requests(&self, account_id: &str) -> Result<FriendRequestsResult, FriendError>;

    async fn resolve_request(
        &self,
        acting_account_id: &str,
        request_id: &str,
        status: FriendRequestStatus,
    ) -> Result<FriendRequestSummary, FriendError>;

    async fn list_friends(&self, account_id: &str) -> Result<Vec<FriendSummary>, FriendError>;
}

pub struct DefaultFriendService {
    store: StoreHandle,
}

impl DefaultFriendService {
    #[must_use]
    pub fn new(store: StoreHandle) -> Arc<Self> {
        Arc::new(Self { store })
    }
}

#[async_trait]
impl FriendService for DefaultFriendService {
    async fn create_request(
        &self,
        from_account_id: &str,
        target_minos_id: &str,
    ) -> Result<FriendRequestSummary, FriendError> {
        let me = social::profile_by_account(&self.store, from_account_id)
            .await?
            .ok_or(FriendError::Internal(BackendError::StoreQuery {
                operation: "friends.create_request".into(),
                message: "caller profile not found".into(),
            }))?;
        let target = social::find_by_minos_id(&self.store, target_minos_id)
            .await?
            .ok_or(FriendError::TargetNotFound)?;
        if target.account_id == from_account_id {
            return Err(FriendError::CannotAddSelf);
        }
        if social::are_friends(&self.store, from_account_id, &target.account_id).await? {
            return Err(FriendError::AlreadyFriends);
        }
        if social::has_pending_friend_request_between(
            &self.store,
            from_account_id,
            &target.account_id,
        )
        .await?
        {
            return Err(FriendError::RequestAlreadyPending);
        }
        let created_at_ms = chrono::Utc::now().timestamp_millis();
        let request_id = social::create_friend_request(
            &self.store,
            from_account_id,
            &target.account_id,
            created_at_ms,
        )
        .await?;
        Ok(FriendRequestSummary {
            request_id,
            from: to_user_summary(&me),
            to: to_user_summary(&target),
            status: FriendRequestStatus::Pending,
            created_at_ms,
            resolved_at_ms: None,
        })
    }

    async fn list_requests(&self, account_id: &str) -> Result<FriendRequestsResult, FriendError> {
        let incoming_rows = social::list_incoming_friend_requests(&self.store, account_id).await?;
        let outgoing_rows = social::list_outgoing_friend_requests(&self.store, account_id).await?;
        let incoming = hydrate_friend_requests(&self.store, incoming_rows).await?;
        let outgoing = hydrate_friend_requests(&self.store, outgoing_rows).await?;
        Ok(FriendRequestsResult { incoming, outgoing })
    }

    async fn resolve_request(
        &self,
        acting_account_id: &str,
        request_id: &str,
        status: FriendRequestStatus,
    ) -> Result<FriendRequestSummary, FriendError> {
        let resolved_at_ms = chrono::Utc::now().timestamp_millis();
        match social::resolve_friend_request_transactional(
            &self.store,
            acting_account_id,
            request_id,
            status,
            resolved_at_ms,
        )
        .await?
        {
            social::ResolveFriendRequestTxResult::Resolved(row) => {
                let mut hydrated = hydrate_friend_requests(&self.store, vec![row]).await?;
                Ok(hydrated.remove(0))
            }
            social::ResolveFriendRequestTxResult::NotFound => Err(FriendError::RequestNotFound),
            social::ResolveFriendRequestTxResult::Unauthorized => Err(FriendError::Unauthorized),
            social::ResolveFriendRequestTxResult::AlreadyResolved => {
                Err(FriendError::AlreadyResolved)
            }
        }
    }

    async fn list_friends(&self, account_id: &str) -> Result<Vec<FriendSummary>, FriendError> {
        let friendships = social::list_friendships_for(&self.store, account_id).await?;

        let friend_ids: Vec<String> = friendships
            .iter()
            .map(|f| {
                if f.account_low_id == account_id {
                    f.account_high_id.clone()
                } else {
                    f.account_low_id.clone()
                }
            })
            .collect();
        let profiles = social::profiles_by_accounts(&self.store, &friend_ids).await?;

        let mut friends = Vec::with_capacity(friendships.len());
        for friendship in friendships {
            let other_id = if friendship.account_low_id == account_id {
                &friendship.account_high_id
            } else {
                &friendship.account_low_id
            };
            let profile = profiles.get(other_id).ok_or_else(|| {
                FriendError::Internal(BackendError::StoreQuery {
                    operation: "friends.list_friends".into(),
                    message: format!("profile not found: {other_id}"),
                })
            })?;
            friends.push(FriendSummary {
                account_id: profile.account_id.clone(),
                minos_id: profile.minos_id.clone(),
                display_name: crate::profiles::use_case::display_name(profile),
                created_at_ms: friendship.created_at_ms,
            });
        }
        Ok(friends)
    }
}

async fn hydrate_friend_requests(
    store: &StoreHandle,
    rows: Vec<social::FriendRequestRow>,
) -> Result<Vec<FriendRequestSummary>, BackendError> {
    let mut account_ids: Vec<String> = rows
        .iter()
        .flat_map(|r| [r.from_account_id.clone(), r.to_account_id.clone()])
        .collect();
    account_ids.sort();
    account_ids.dedup();
    let profiles = social::profiles_by_accounts(store, &account_ids).await?;

    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        let from = profiles
            .get(&row.from_account_id)
            .ok_or_else(|| BackendError::StoreQuery {
                operation: "friends.hydrate_friend_requests".into(),
                message: format!("profile not found: {}", row.from_account_id),
            })?;
        let to = profiles
            .get(&row.to_account_id)
            .ok_or_else(|| BackendError::StoreQuery {
                operation: "friends.hydrate_friend_requests".into(),
                message: format!("profile not found: {}", row.to_account_id),
            })?;
        output.push(FriendRequestSummary {
            request_id: row.request_id,
            from: to_user_summary(from),
            to: to_user_summary(to),
            status: parse_request_status(&row.status)?,
            created_at_ms: row.created_at_ms,
            resolved_at_ms: row.resolved_at_ms,
        });
    }
    Ok(output)
}

fn parse_request_status(status: &str) -> Result<FriendRequestStatus, BackendError> {
    match status {
        "pending" => Ok(FriendRequestStatus::Pending),
        "accepted" => Ok(FriendRequestStatus::Accepted),
        "rejected" => Ok(FriendRequestStatus::Rejected),
        "canceled" => Ok(FriendRequestStatus::Canceled),
        _ => Err(BackendError::StoreQuery {
            operation: "friends.parse_request_status".into(),
            message: format!("unknown friend request status: {status}"),
        }),
    }
}
