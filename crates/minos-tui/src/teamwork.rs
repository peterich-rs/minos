use std::path::PathBuf;

use minos_chat_store::{TeamworkDelegation, TeamworkStore as PersistentTeamworkStore};
use minos_domain::AgentName;

#[derive(Clone)]
pub struct TeamworkStore {
    db_path: Option<PathBuf>,
}

impl TeamworkStore {
    #[cfg(not(test))]
    pub fn default_for_runtime() -> anyhow::Result<Self> {
        Ok(Self {
            db_path: Some(minos_chat_store::default_db_path()?),
        })
    }

    pub fn disabled() -> Self {
        Self { db_path: None }
    }

    // Called from test-only in-process MCP handlers (daemon owns production MCP).
    #[allow(dead_code)]
    pub async fn create_delegation(
        &self,
        conversation_id: &str,
        source_agent: Option<AgentName>,
        source_thread_id: Option<String>,
        target_agent: AgentName,
        prompt: String,
        thread_id: Option<String>,
    ) -> anyhow::Result<TeamworkDelegation> {
        self.open_for_conversation(conversation_id)
            .await?
            .create_delegation(
                conversation_id,
                source_agent,
                source_thread_id,
                target_agent,
                prompt,
                thread_id,
            )
            .await
    }

    pub async fn running_delegation_for_thread(
        &self,
        conversation_id: &str,
        thread_id: &str,
    ) -> anyhow::Result<Option<TeamworkDelegation>> {
        self.open_for_conversation(conversation_id)
            .await?
            .running_delegation_for_thread(conversation_id, thread_id)
            .await
    }

    #[allow(dead_code)] // test-only MCP path
    pub async fn ensure_delegate_target_allowed(
        &self,
        conversation_id: &str,
        source_thread_id: Option<&str>,
        target_agent: AgentName,
    ) -> anyhow::Result<()> {
        self.open_for_conversation(conversation_id)
            .await?
            .ensure_delegate_target_allowed(conversation_id, source_thread_id, target_agent)
            .await
    }

    pub async fn complete_delegation_for_thread(
        &self,
        conversation_id: &str,
        thread_id: &str,
        result_message_id: Option<&str>,
        result_text: &str,
    ) -> anyhow::Result<Option<TeamworkDelegation>> {
        self.open_for_conversation(conversation_id)
            .await?
            .complete_delegation_for_thread(
                conversation_id,
                thread_id,
                result_message_id,
                result_text,
            )
            .await
    }

    #[cfg(test)]
    pub fn for_db_path(db_path: PathBuf) -> Self {
        Self {
            db_path: Some(db_path),
        }
    }

    #[allow(dead_code)] // test-only MCP path
    pub async fn get_delegation(
        &self,
        conversation_id: &str,
        delegation_id: &str,
    ) -> anyhow::Result<Option<TeamworkDelegation>> {
        self.open_for_conversation(conversation_id)
            .await?
            .get_delegation(conversation_id, delegation_id)
            .await
    }

    #[allow(dead_code)] // test-only MCP path
    pub async fn wait_delegation(
        &self,
        conversation_id: &str,
        delegation_id: &str,
        timeout: std::time::Duration,
        poll_interval: std::time::Duration,
    ) -> anyhow::Result<(TeamworkDelegation, bool)> {
        self.open_for_conversation(conversation_id)
            .await?
            .wait_delegation(conversation_id, delegation_id, timeout, poll_interval)
            .await
    }

    #[allow(dead_code)] // test-only MCP path
    pub async fn cancel_delegation(
        &self,
        conversation_id: &str,
        delegation_id: &str,
        reason: Option<String>,
    ) -> anyhow::Result<TeamworkDelegation> {
        self.open_for_conversation(conversation_id)
            .await?
            .cancel_delegation(conversation_id, delegation_id, reason)
            .await
    }

    async fn open_for_conversation(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<PersistentTeamworkStore> {
        let Some(path) = &self.db_path else {
            anyhow::bail!("teamwork storage is disabled");
        };
        let store = PersistentTeamworkStore::open(path).await?;
        store
            .ensure_conversation(conversation_id, conversation_id, "")
            .await?;
        Ok(store)
    }
}
