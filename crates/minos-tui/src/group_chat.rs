use std::path::{Path, PathBuf};

use minos_chat_store::{ChatAgentSession, ChatStore, NewChatMessage};
use minos_protocol::LocalGroupChatMessage;

#[derive(Clone)]
pub struct GroupChatStore {
    db_path: Option<PathBuf>,
    legacy_jsonl_path: Option<PathBuf>,
    room_id: String,
    room_title: String,
    workspace_root: String,
}

impl GroupChatStore {
    #[cfg(not(test))]
    pub fn default_for_runtime(workspace: &Path) -> anyhow::Result<Self> {
        let room_id = minos_chat_store::room_id_for_workspace(workspace);
        let room_title = minos_chat_store::room_title_for_workspace(workspace);
        Ok(Self {
            db_path: Some(minos_chat_store::default_db_path()?),
            legacy_jsonl_path: minos_chat_store::legacy_jsonl_path().ok(),
            room_id,
            room_title,
            workspace_root: workspace.display().to_string(),
        })
    }

    #[cfg(test)]
    pub fn at_path(path: PathBuf) -> Self {
        Self {
            db_path: Some(path),
            legacy_jsonl_path: None,
            room_id: "room-test".into(),
            room_title: "test".into(),
            workspace_root: "/tmp/test".into(),
        }
    }

    pub fn disabled() -> Self {
        Self {
            db_path: None,
            legacy_jsonl_path: None,
            room_id: "room-disabled".into(),
            room_title: "disabled".into(),
            workspace_root: String::new(),
        }
    }

    pub async fn load_recent(&self, limit: usize) -> anyhow::Result<Vec<LocalGroupChatMessage>> {
        let Some(store) = self.open().await? else {
            return Ok(Vec::new());
        };
        self.migrate_legacy_jsonl_if_needed(&store).await?;
        let limit = u32::try_from(limit).unwrap_or(u32::MAX);
        let messages = store
            .list_recent_messages_asc(&self.room_id, Some(limit))
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(messages)
    }

    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    pub async fn list_agent_sessions(&self) -> anyhow::Result<Vec<ChatAgentSession>> {
        let Some(store) = self.open().await? else {
            return Ok(Vec::new());
        };
        self.migrate_legacy_jsonl_if_needed(&store).await?;
        store.list_agent_sessions(&self.room_id).await
    }

    pub async fn append(
        &self,
        message: LocalGroupChatMessage,
    ) -> anyhow::Result<LocalGroupChatMessage> {
        let Some(store) = self.open().await? else {
            let mut message = message;
            if message.message_id.is_empty() {
                message.message_id = "volatile-group-message".into();
            }
            return Ok(message);
        };
        self.migrate_legacy_jsonl_if_needed(&store).await?;
        let message = store
            .append_message(&self.room_id, NewChatMessage::from(message))
            .await?;
        Ok(message.into())
    }

    async fn open(&self) -> anyhow::Result<Option<ChatStore>> {
        let Some(path) = &self.db_path else {
            return Ok(None);
        };
        let store = ChatStore::open(path).await?;
        store
            .ensure_room(&self.room_id, &self.room_title, &self.workspace_root)
            .await?;
        Ok(Some(store))
    }

    async fn migrate_legacy_jsonl_if_needed(&self, store: &ChatStore) -> anyhow::Result<()> {
        let Some(path) = &self.legacy_jsonl_path else {
            return Ok(());
        };
        if store.count_messages(&self.room_id).await? > 0 {
            return Ok(());
        }
        let messages = read_legacy_jsonl(path)?;
        for message in messages {
            store
                .append_message(&self.room_id, NewChatMessage::from(message))
                .await?;
        }
        Ok(())
    }
}

fn read_legacy_jsonl(path: &Path) -> anyhow::Result<Vec<LocalGroupChatMessage>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut messages = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(message) => messages.push(message),
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui::group_chat",
                    path = %path.display(),
                    error = %error,
                    "skipping malformed legacy group chat log line"
                );
            }
        }
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;
    use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind};

    #[tokio::test]
    async fn append_assigns_sequence_and_load_recent_returns_ascending() {
        let temp = tempfile::tempdir().unwrap();
        let store = GroupChatStore::at_path(temp.path().join("group.sqlite"));
        store
            .append(LocalGroupChatMessage {
                seq: 0,
                message_id: String::new(),
                created_at_ms: 10,
                kind: LocalGroupChatMessageKind::User,
                text: "@codex hello".into(),
                agent: Some(AgentName::Codex),
                thread_id: Some("thread-1".into()),
                thread_short_id: Some("thread-1".into()),
                workspace: Some("/tmp/ws".into()),
            })
            .await
            .unwrap();
        store
            .append(LocalGroupChatMessage {
                seq: 0,
                message_id: String::new(),
                created_at_ms: 20,
                kind: LocalGroupChatMessageKind::AgentResult,
                text: "done".into(),
                agent: Some(AgentName::Codex),
                thread_id: Some("thread-1".into()),
                thread_short_id: Some("thread-1".into()),
                workspace: Some("/tmp/ws".into()),
            })
            .await
            .unwrap();

        let messages = store.load_recent(10).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].seq, 1);
        assert_eq!(messages[1].kind, LocalGroupChatMessageKind::AgentResult);

        let sessions = store.list_agent_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent, AgentName::Codex);
    }
}
