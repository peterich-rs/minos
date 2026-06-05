use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use minos_protocol::LocalGroupChatMessage;

#[derive(Clone)]
pub struct GroupChatStore {
    path: Option<PathBuf>,
}

impl GroupChatStore {
    #[cfg(not(test))]
    pub fn default_for_runtime() -> anyhow::Result<Self> {
        Ok(Self {
            path: Some(default_group_chat_log_path()?),
        })
    }

    #[cfg(test)]
    pub fn at_path(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub fn disabled() -> Self {
        Self { path: None }
    }

    pub fn load_recent(&self, limit: usize) -> anyhow::Result<Vec<LocalGroupChatMessage>> {
        let Some(path) = &self.path else {
            return Ok(Vec::new());
        };

        let mut messages = read_messages(path)?;
        if messages.len() > limit {
            messages = messages.split_off(messages.len() - limit);
        }
        Ok(messages)
    }

    pub fn append(
        &self,
        mut message: LocalGroupChatMessage,
    ) -> anyhow::Result<LocalGroupChatMessage> {
        let Some(path) = &self.path else {
            if message.message_id.is_empty() {
                message.message_id = "volatile-group-message".into();
            }
            return Ok(message);
        };

        let next_seq = read_messages(path)?
            .iter()
            .map(|message| message.seq)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        message.seq = next_seq;
        if message.message_id.is_empty() {
            message.message_id = format!("tui-group-{next_seq}");
        }
        if message.created_at_ms == 0 {
            message.created_at_ms = chrono::Utc::now().timestamp_millis();
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        serde_json::to_writer(&mut file, &message)?;
        file.write_all(b"\n")?;
        Ok(message)
    }
}

fn read_messages(path: &PathBuf) -> anyhow::Result<Vec<LocalGroupChatMessage>> {
    let content = match fs::read_to_string(path) {
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
                    "skipping malformed group chat log line"
                );
            }
        }
    }
    Ok(messages)
}

#[cfg(not(test))]
fn default_group_chat_log_path() -> anyhow::Result<PathBuf> {
    Ok(resolve_minos_home()?
        .join("state")
        .join("tui-group-chat.jsonl"))
}

#[cfg(not(test))]
fn resolve_minos_home() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var("MINOS_HOME") {
        return Ok(path.into());
    }
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home).join(".minos"));
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(user_profile).join(".minos"));
    }
    let home_drive = std::env::var_os("HOMEDRIVE").filter(|value| !value.is_empty());
    let home_path = std::env::var_os("HOMEPATH").filter(|value| !value.is_empty());
    if let (Some(drive), Some(path)) = (home_drive, home_path) {
        return Ok(PathBuf::from(drive).join(path).join(".minos"));
    }
    anyhow::bail!("unable to resolve MINOS_HOME from environment")
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;
    use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind};

    #[test]
    fn append_and_load_recent_round_trips_jsonl_messages() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = GroupChatStore::at_path(temp.path().join("group.jsonl"));

        store
            .append(LocalGroupChatMessage {
                seq: 0,
                message_id: String::new(),
                created_at_ms: 10,
                kind: LocalGroupChatMessageKind::User,
                text: "@codex inspect src".into(),
                agent: Some(AgentName::Codex),
                thread_id: Some("thread-1".into()),
                thread_short_id: Some("thread-1".into()),
                workspace: Some("/tmp/ws".into()),
            })
            .expect("append first");
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
            .expect("append second");

        let messages = store.load_recent(10).expect("load");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].seq, 1);
        assert_eq!(messages[0].message_id, "tui-group-1");
        assert_eq!(messages[1].seq, 2);
        assert_eq!(messages[1].kind, LocalGroupChatMessageKind::AgentResult);
    }
}
