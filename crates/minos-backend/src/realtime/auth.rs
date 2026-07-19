use crate::error::BackendError;
use crate::store::{self, AsStorePool};

use minos_protocol::realtime::{ConnectionPrincipal, RealtimeTopic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubscriptionDenied {
    #[error("realtime_subscription_denied")]
    Forbidden,
    #[error("realtime_subscription_limit_exceeded")]
    LimitExceeded,
    #[error("realtime_subscription_invalid_topic")]
    InvalidTopic,
}

impl SubscriptionDenied {
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::LimitExceeded => "limit_exceeded",
            Self::InvalidTopic => "invalid_topic",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubscriptionAuthError {
    #[error(transparent)]
    Denied(#[from] SubscriptionDenied),
    #[error(transparent)]
    Internal(#[from] BackendError),
}

pub async fn authorize_subscription(
    store: &impl AsStorePool,
    principal: &ConnectionPrincipal,
    topic: &RealtimeTopic,
) -> Result<(), SubscriptionAuthError> {
    match (principal, topic) {
        (
            ConnectionPrincipal::Account { account_id },
            RealtimeTopic::Account(target_account_id),
        ) => {
            if target_account_id == account_id {
                Ok(())
            } else {
                Err(SubscriptionDenied::Forbidden.into())
            }
        }
        (
            ConnectionPrincipal::Account { account_id },
            RealtimeTopic::Conversation(conversation_id),
        ) => {
            if store::social::is_conversation_member(store, conversation_id, account_id).await? {
                Ok(())
            } else {
                Err(SubscriptionDenied::Forbidden.into())
            }
        }
        (ConnectionPrincipal::Account { account_id }, RealtimeTopic::Project(project_id)) => {
            if store::projects::exists(store, account_id, project_id).await? {
                Ok(())
            } else {
                Err(SubscriptionDenied::Forbidden.into())
            }
        }
        (ConnectionPrincipal::Account { account_id }, RealtimeTopic::AgentSession(session_id)) => {
            if store::agent_sessions::get_for_account(store, session_id, account_id)
                .await?
                .is_some()
            {
                Ok(())
            } else {
                Err(SubscriptionDenied::Forbidden.into())
            }
        }
        (
            ConnectionPrincipal::Host {
                host_installation_id,
            },
            RealtimeTopic::Host(target_host_id),
        ) => {
            if target_host_id == host_installation_id {
                Ok(())
            } else {
                Err(SubscriptionDenied::Forbidden.into())
            }
        }
        (
            ConnectionPrincipal::Host {
                host_installation_id,
            },
            RealtimeTopic::AgentSession(session_id),
        ) => {
            let session = store::agent_sessions::get(store, session_id).await?;
            if session.and_then(|row| row.host_device_id).as_deref()
                == Some(host_installation_id.as_str())
            {
                Ok(())
            } else {
                Err(SubscriptionDenied::Forbidden.into())
            }
        }
        _ => Err(SubscriptionDenied::Forbidden.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_account, insert_ios_device, memory_pool};
    use crate::store::{account_host_pairings, agent_sessions, devices, social};
    use minos_domain::{DeviceId, DeviceRole};

    #[tokio::test]
    async fn account_subscription_authorizer_allows_owned_scopes_and_denies_host_topic() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "realtime-auth@example.com").await;
        let members = vec![account_id.clone()];
        let conversation =
            social::create_group_conversation(&pool, &account_id, "Realtime Auth", &members, 100)
                .await
                .unwrap();
        agent_sessions::create(
            &pool,
            "sess-auth",
            &conversation.conversation_id,
            None,
            None,
            None,
            "running",
            101,
            None,
        )
        .await
        .unwrap();

        let principal = ConnectionPrincipal::Account {
            account_id: account_id.clone(),
        };

        authorize_subscription(
            &pool,
            &principal,
            &RealtimeTopic::Account(account_id.clone()),
        )
        .await
        .unwrap();
        authorize_subscription(
            &pool,
            &principal,
            &RealtimeTopic::Conversation(conversation.conversation_id.clone()),
        )
        .await
        .unwrap();
        authorize_subscription(
            &pool,
            &principal,
            &RealtimeTopic::AgentSession("sess-auth".into()),
        )
        .await
        .unwrap();

        let err = authorize_subscription(
            &pool,
            &principal,
            &RealtimeTopic::Host(DeviceId::new().to_string()),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            SubscriptionAuthError::Denied(SubscriptionDenied::Forbidden)
        ));
    }

    #[tokio::test]
    async fn host_subscription_authorizer_requires_matching_host_scope() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "realtime-host-auth@example.com").await;
        let phone = insert_ios_device(&pool, &account_id).await;
        let host_id = DeviceId::new();
        devices::insert_device(&pool, host_id, "Mac", DeviceRole::AgentHost, 0)
            .await
            .unwrap();
        let members = vec![account_id.clone()];
        let conversation =
            social::create_group_conversation(&pool, &account_id, "Host Scope", &members, 100)
                .await
                .unwrap();
        account_host_pairings::insert_pair(&pool, host_id, &account_id, phone, 0)
            .await
            .unwrap();
        agent_sessions::create(
            &pool,
            "sess-host-auth",
            &conversation.conversation_id,
            None,
            Some(&host_id.to_string()),
            None,
            "running",
            101,
            None,
        )
        .await
        .unwrap();

        let principal = ConnectionPrincipal::Host {
            host_installation_id: host_id.to_string(),
        };

        authorize_subscription(&pool, &principal, &RealtimeTopic::Host(host_id.to_string()))
            .await
            .unwrap();
        authorize_subscription(
            &pool,
            &principal,
            &RealtimeTopic::AgentSession("sess-host-auth".into()),
        )
        .await
        .unwrap();

        let err = authorize_subscription(
            &pool,
            &principal,
            &RealtimeTopic::Conversation(conversation.conversation_id),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            SubscriptionAuthError::Denied(SubscriptionDenied::Forbidden)
        ));
    }
}
