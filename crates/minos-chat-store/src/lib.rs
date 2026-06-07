use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use minos_domain::AgentName;
use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

pub mod mcp;

const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 500;

#[derive(Debug, Clone)]
pub struct ChatStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRoom {
    pub room_id: String,
    pub title: String,
    pub workspace_root: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub seq: u64,
    pub message_id: String,
    pub room_id: String,
    pub created_at_ms: i64,
    pub sender_role: ChatSenderRole,
    pub event_type: ChatMessageType,
    pub text: String,
    pub agent: Option<AgentName>,
    pub thread_id: Option<String>,
    pub thread_short_id: Option<String>,
    pub workspace_root: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatSenderRole {
    User,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatMessageType {
    UserMessage,
    AgentResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatAgentSession {
    pub room_id: String,
    pub agent: AgentName,
    pub thread_id: String,
    pub thread_short_id: String,
    pub workspace_root: String,
    pub first_seen_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub first_message_seq: u64,
    pub last_message_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessagePage {
    pub room_id: String,
    pub messages: Vec<ChatMessage>,
    pub next_before_seq: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMcpCommand {
    pub seq: u64,
    pub room_id: String,
    pub created_at_ms: i64,
    pub claimed_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub status: ChatMcpCommandStatus,
    pub kind: ChatMcpCommandKind,
    pub source_agent: Option<AgentName>,
    pub target_agent: Option<AgentName>,
    pub body: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewChatMcpCommand {
    pub kind: ChatMcpCommandKind,
    pub source_agent: Option<AgentName>,
    pub target_agent: Option<AgentName>,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatMcpCommandKind {
    MentionAgent,
    MentionUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatMcpCommandStatus {
    Pending,
    Claimed,
    Completed,
    Failed,
}

impl ChatStore {
    pub async fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create chat db parent dir {}", parent.display())
            })?;
        }

        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let opts = SqliteConnectOptions::from_str(&url)?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn ensure_room(
        &self,
        room_id: &str,
        title: &str,
        workspace_root: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO chat_rooms(room_id, title, workspace_root, created_at_ms, updated_at_ms) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(room_id) DO UPDATE SET \
                title = excluded.title, \
                workspace_root = excluded.workspace_root, \
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(room_id)
        .bind(title)
        .bind(workspace_root)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn append_message(
        &self,
        room_id: &str,
        input: NewChatMessage,
    ) -> Result<ChatMessage> {
        let mut tx = self.pool.begin().await?;

        let seq_i64: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(message_seq), 0) + 1 FROM chat_messages")
                .fetch_one(&mut *tx)
                .await?;
        let seq = u64::try_from(seq_i64).context("chat message seq overflow")?;
        let message_id = input
            .message_id
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("tui-group-{seq}"));
        let created_at_ms = if input.created_at_ms == 0 {
            chrono::Utc::now().timestamp_millis()
        } else {
            input.created_at_ms
        };

        let sender_role = input.event_type.sender_role();
        sqlx::query(
            "INSERT INTO chat_messages( \
                message_seq, message_id, room_id, created_at_ms, sender_role, event_type, \
                body, agent, thread_id, thread_short_id, workspace_root \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(seq_i64)
        .bind(&message_id)
        .bind(room_id)
        .bind(created_at_ms)
        .bind(sender_role.as_db())
        .bind(input.event_type.as_db())
        .bind(&input.text)
        .bind(input.agent.map(|agent| agent.bin_name().to_owned()))
        .bind(input.thread_id.as_deref())
        .bind(input.thread_short_id.as_deref())
        .bind(input.workspace_root.as_deref())
        .execute(&mut *tx)
        .await?;

        if let (Some(agent), Some(thread_id), Some(thread_short_id), Some(workspace_root)) = (
            input.agent,
            input.thread_id.as_deref(),
            input.thread_short_id.as_deref(),
            input.workspace_root.as_deref(),
        ) {
            sqlx::query(
                "INSERT INTO chat_agent_sessions( \
                    room_id, thread_id, agent, thread_short_id, workspace_root, \
                    first_seen_at_ms, last_seen_at_ms, first_message_seq, last_message_seq \
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(room_id, thread_id) DO UPDATE SET \
                    agent = excluded.agent, \
                    thread_short_id = excluded.thread_short_id, \
                    workspace_root = excluded.workspace_root, \
                    last_seen_at_ms = excluded.last_seen_at_ms, \
                    last_message_seq = excluded.last_message_seq",
            )
            .bind(room_id)
            .bind(thread_id)
            .bind(agent.bin_name())
            .bind(thread_short_id)
            .bind(workspace_root)
            .bind(created_at_ms)
            .bind(created_at_ms)
            .bind(seq_i64)
            .bind(seq_i64)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query("UPDATE chat_rooms SET updated_at_ms = ? WHERE room_id = ?")
            .bind(created_at_ms)
            .bind(room_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(ChatMessage {
            seq,
            message_id,
            room_id: room_id.to_owned(),
            created_at_ms,
            sender_role,
            event_type: input.event_type,
            text: input.text,
            agent: input.agent,
            thread_id: input.thread_id,
            thread_short_id: input.thread_short_id,
            workspace_root: input.workspace_root,
        })
    }

    pub async fn list_messages_desc(
        &self,
        room_id: &str,
        before_seq: Option<u64>,
        limit: Option<u32>,
    ) -> Result<ChatMessagePage> {
        let limit = normalize_limit(limit);
        let fetch_limit = i64::from(limit.saturating_add(1));
        let rows = match before_seq {
            Some(before_seq) => {
                sqlx::query(
                    "SELECT * FROM chat_messages \
                     WHERE room_id = ? AND message_seq < ? \
                     ORDER BY message_seq DESC LIMIT ?",
                )
                .bind(room_id)
                .bind(before_seq as i64)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT * FROM chat_messages \
                     WHERE room_id = ? \
                     ORDER BY message_seq DESC LIMIT ?",
                )
                .bind(room_id)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await?
            }
        };

        let mut messages = rows
            .into_iter()
            .map(chat_message_from_row)
            .collect::<Result<Vec<_>>>()?;
        let has_more = messages.len() > limit as usize;
        if has_more {
            messages.truncate(limit as usize);
        }
        let next_before_seq = has_more
            .then(|| messages.last().map(|message| message.seq))
            .flatten();
        Ok(ChatMessagePage {
            room_id: room_id.to_owned(),
            messages,
            next_before_seq,
            has_more,
        })
    }

    pub async fn list_recent_messages_asc(
        &self,
        room_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<ChatMessage>> {
        let mut page = self.list_messages_desc(room_id, None, limit).await?;
        page.messages.reverse();
        Ok(page.messages)
    }

    pub async fn list_messages_after_asc(
        &self,
        room_id: &str,
        after_seq: u64,
        limit: Option<u32>,
    ) -> Result<Vec<ChatMessage>> {
        let limit = normalize_limit(limit);
        let rows = sqlx::query(
            "SELECT * FROM chat_messages \
             WHERE room_id = ? AND message_seq > ? \
             ORDER BY message_seq ASC LIMIT ?",
        )
        .bind(room_id)
        .bind(after_seq as i64)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(chat_message_from_row).collect()
    }

    pub async fn count_messages(&self, room_id: &str) -> Result<u64> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages WHERE room_id = ?")
            .bind(room_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    pub async fn most_recent_non_empty_room_id(&self) -> Result<Option<String>> {
        let room_id = sqlx::query_scalar(
            "SELECT room_id FROM chat_rooms \
             WHERE EXISTS (SELECT 1 FROM chat_messages WHERE chat_messages.room_id = chat_rooms.room_id) \
             ORDER BY updated_at_ms DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(room_id)
    }

    pub async fn list_agent_sessions(&self, room_id: &str) -> Result<Vec<ChatAgentSession>> {
        let rows = sqlx::query(
            "SELECT * FROM chat_agent_sessions \
             WHERE room_id = ? \
             ORDER BY last_message_seq ASC",
        )
        .bind(room_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(agent_session_from_row).collect()
    }

    pub async fn enqueue_mcp_command(
        &self,
        room_id: &str,
        input: NewChatMcpCommand,
    ) -> Result<ChatMcpCommand> {
        let now = chrono::Utc::now().timestamp_millis();
        let result = sqlx::query(
            "INSERT INTO chat_mcp_commands( \
                room_id, created_at_ms, status, command_type, source_agent, target_agent, body \
             ) VALUES (?, ?, 'pending', ?, ?, ?, ?)",
        )
        .bind(room_id)
        .bind(now)
        .bind(input.kind.as_db())
        .bind(input.source_agent.map(|agent| agent.bin_name().to_owned()))
        .bind(input.target_agent.map(|agent| agent.bin_name().to_owned()))
        .bind(input.body)
        .execute(&self.pool)
        .await?;

        let seq = u64::try_from(result.last_insert_rowid()).context("mcp command seq overflow")?;
        self.get_mcp_command(seq)
            .await?
            .context("inserted MCP command is missing")
    }

    pub async fn claim_pending_mcp_commands(
        &self,
        room_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<ChatMcpCommand>> {
        let limit = i64::from(normalize_limit(limit));
        let now = chrono::Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT * FROM chat_mcp_commands \
             WHERE room_id = ? AND status = 'pending' \
             ORDER BY command_seq ASC LIMIT ?",
        )
        .bind(room_id)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;

        let mut commands = Vec::with_capacity(rows.len());
        for row in rows {
            let seq_i64: i64 = row.try_get("command_seq")?;
            let updated = sqlx::query(
                "UPDATE chat_mcp_commands \
                 SET status = 'claimed', claimed_at_ms = ? \
                 WHERE command_seq = ? AND status = 'pending'",
            )
            .bind(now)
            .bind(seq_i64)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() == 0 {
                continue;
            }
            let mut command = chat_mcp_command_from_row(row)?;
            command.status = ChatMcpCommandStatus::Claimed;
            command.claimed_at_ms = Some(now);
            commands.push(command);
        }
        tx.commit().await?;
        Ok(commands)
    }

    pub async fn complete_mcp_command(&self, seq: u64) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE chat_mcp_commands \
             SET status = 'completed', completed_at_ms = ?, error = NULL \
             WHERE command_seq = ?",
        )
        .bind(now)
        .bind(seq as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn fail_mcp_command(&self, seq: u64, error: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE chat_mcp_commands \
             SET status = 'failed', completed_at_ms = ?, error = ? \
             WHERE command_seq = ?",
        )
        .bind(now)
        .bind(error)
        .bind(seq as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_mcp_command(&self, seq: u64) -> Result<Option<ChatMcpCommand>> {
        let row = sqlx::query("SELECT * FROM chat_mcp_commands WHERE command_seq = ?")
            .bind(seq as i64)
            .fetch_optional(&self.pool)
            .await?;
        row.map(chat_mcp_command_from_row).transpose()
    }

    async fn migrate(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chat_rooms ( \
                room_id TEXT PRIMARY KEY, \
                title TEXT NOT NULL, \
                workspace_root TEXT NOT NULL, \
                created_at_ms INTEGER NOT NULL, \
                updated_at_ms INTEGER NOT NULL \
             )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chat_messages ( \
                message_seq INTEGER PRIMARY KEY, \
                message_id TEXT NOT NULL UNIQUE, \
                room_id TEXT NOT NULL REFERENCES chat_rooms(room_id), \
                created_at_ms INTEGER NOT NULL, \
                sender_role TEXT NOT NULL CHECK(sender_role IN ('user', 'agent')), \
                event_type TEXT NOT NULL CHECK(event_type IN ('user_message', 'agent_result')), \
                body TEXT NOT NULL, \
                agent TEXT, \
                thread_id TEXT, \
                thread_short_id TEXT, \
                workspace_root TEXT \
             )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS chat_messages_by_room_seq \
             ON chat_messages(room_id, message_seq DESC)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS chat_messages_by_thread \
             ON chat_messages(thread_id, message_seq DESC)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chat_agent_sessions ( \
                room_id TEXT NOT NULL, \
                thread_id TEXT NOT NULL, \
                agent TEXT NOT NULL, \
                thread_short_id TEXT NOT NULL, \
                workspace_root TEXT NOT NULL, \
                first_seen_at_ms INTEGER NOT NULL, \
                last_seen_at_ms INTEGER NOT NULL, \
                first_message_seq INTEGER NOT NULL, \
                last_message_seq INTEGER NOT NULL, \
                PRIMARY KEY(room_id, thread_id), \
                FOREIGN KEY(room_id) REFERENCES chat_rooms(room_id) \
             )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS chat_agent_sessions_by_room_last \
             ON chat_agent_sessions(room_id, last_message_seq DESC)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chat_mcp_commands ( \
                command_seq INTEGER PRIMARY KEY, \
                room_id TEXT NOT NULL REFERENCES chat_rooms(room_id), \
                created_at_ms INTEGER NOT NULL, \
                claimed_at_ms INTEGER, \
                completed_at_ms INTEGER, \
                status TEXT NOT NULL CHECK(status IN ('pending', 'claimed', 'completed', 'failed')), \
                command_type TEXT NOT NULL CHECK(command_type IN ('mention_agent', 'mention_user')), \
                source_agent TEXT, \
                target_agent TEXT, \
                body TEXT NOT NULL, \
                error TEXT \
             )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS chat_mcp_commands_by_room_status_seq \
             ON chat_mcp_commands(room_id, status, command_seq ASC)",
        )
        .execute(pool)
        .await?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct NewChatMessage {
    pub message_id: Option<String>,
    pub created_at_ms: i64,
    pub event_type: ChatMessageType,
    pub text: String,
    pub agent: Option<AgentName>,
    pub thread_id: Option<String>,
    pub thread_short_id: Option<String>,
    pub workspace_root: Option<String>,
}

impl From<LocalGroupChatMessage> for NewChatMessage {
    fn from(message: LocalGroupChatMessage) -> Self {
        Self {
            message_id: (!message.message_id.is_empty()).then_some(message.message_id),
            created_at_ms: message.created_at_ms,
            event_type: ChatMessageType::from(message.kind),
            text: message.text,
            agent: message.agent,
            thread_id: message.thread_id,
            thread_short_id: message.thread_short_id,
            workspace_root: message.workspace,
        }
    }
}

impl From<LocalGroupChatMessageKind> for ChatMessageType {
    fn from(kind: LocalGroupChatMessageKind) -> Self {
        match kind {
            LocalGroupChatMessageKind::User => Self::UserMessage,
            LocalGroupChatMessageKind::AgentResult => Self::AgentResult,
        }
    }
}

impl From<ChatMessageType> for LocalGroupChatMessageKind {
    fn from(kind: ChatMessageType) -> Self {
        match kind {
            ChatMessageType::UserMessage => Self::User,
            ChatMessageType::AgentResult => Self::AgentResult,
        }
    }
}

impl From<ChatMessage> for LocalGroupChatMessage {
    fn from(message: ChatMessage) -> Self {
        Self {
            seq: message.seq,
            message_id: message.message_id,
            created_at_ms: message.created_at_ms,
            kind: message.event_type.into(),
            text: message.text,
            agent: message.agent,
            thread_id: message.thread_id,
            thread_short_id: message.thread_short_id,
            workspace: message.workspace_root,
        }
    }
}

impl ChatMessageType {
    const fn as_db(self) -> &'static str {
        match self {
            Self::UserMessage => "user_message",
            Self::AgentResult => "agent_result",
        }
    }

    const fn sender_role(self) -> ChatSenderRole {
        match self {
            Self::UserMessage => ChatSenderRole::User,
            Self::AgentResult => ChatSenderRole::Agent,
        }
    }

    fn from_db(value: &str) -> Result<Self> {
        match value {
            "user_message" => Ok(Self::UserMessage),
            "agent_result" => Ok(Self::AgentResult),
            other => anyhow::bail!("unknown chat message event_type: {other}"),
        }
    }
}

impl ChatSenderRole {
    const fn as_db(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }

    fn from_db(value: &str) -> Result<Self> {
        match value {
            "user" => Ok(Self::User),
            "agent" => Ok(Self::Agent),
            other => anyhow::bail!("unknown chat message sender_role: {other}"),
        }
    }
}

impl ChatMcpCommandKind {
    const fn as_db(self) -> &'static str {
        match self {
            Self::MentionAgent => "mention_agent",
            Self::MentionUser => "mention_user",
        }
    }

    fn from_db(value: &str) -> Result<Self> {
        match value {
            "mention_agent" => Ok(Self::MentionAgent),
            "mention_user" => Ok(Self::MentionUser),
            other => anyhow::bail!("unknown MCP command type: {other}"),
        }
    }
}

impl ChatMcpCommandStatus {
    fn from_db(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "claimed" => Ok(Self::Claimed),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown MCP command status: {other}"),
        }
    }
}

pub fn default_db_path() -> Result<PathBuf> {
    Ok(resolve_minos_home()?.join("daemon.sqlite"))
}

pub fn legacy_jsonl_path() -> Result<PathBuf> {
    Ok(resolve_minos_home()?
        .join("state")
        .join("tui-group-chat.jsonl"))
}

pub fn room_id_for_workspace(workspace: &Path) -> String {
    format!("room-{}", room_title_for_workspace(workspace))
}

pub fn room_title_for_workspace(workspace: &Path) -> String {
    let resolved = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let title = resolved
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
        .trim()
        .to_owned();
    if title.is_empty() {
        "main".to_owned()
    } else {
        title
    }
}

fn resolve_minos_home() -> Result<PathBuf> {
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

fn chat_message_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ChatMessage> {
    let seq: i64 = row.try_get("message_seq")?;
    let sender_role: String = row.try_get("sender_role")?;
    let event_type: String = row.try_get("event_type")?;
    let agent = parse_agent(row.try_get::<Option<String>, _>("agent")?)?;
    Ok(ChatMessage {
        seq: u64::try_from(seq).context("negative chat message seq")?,
        message_id: row.try_get("message_id")?,
        room_id: row.try_get("room_id")?,
        created_at_ms: row.try_get("created_at_ms")?,
        sender_role: ChatSenderRole::from_db(&sender_role)?,
        event_type: ChatMessageType::from_db(&event_type)?,
        text: row.try_get("body")?,
        agent,
        thread_id: row.try_get("thread_id")?,
        thread_short_id: row.try_get("thread_short_id")?,
        workspace_root: row.try_get("workspace_root")?,
    })
}

fn agent_session_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ChatAgentSession> {
    let first_seq: i64 = row.try_get("first_message_seq")?;
    let last_seq: i64 = row.try_get("last_message_seq")?;
    let agent_label: String = row.try_get("agent")?;
    Ok(ChatAgentSession {
        room_id: row.try_get("room_id")?,
        agent: parse_agent(Some(agent_label))?.context("chat_agent_sessions.agent is NULL")?,
        thread_id: row.try_get("thread_id")?,
        thread_short_id: row.try_get("thread_short_id")?,
        workspace_root: row.try_get("workspace_root")?,
        first_seen_at_ms: row.try_get("first_seen_at_ms")?,
        last_seen_at_ms: row.try_get("last_seen_at_ms")?,
        first_message_seq: u64::try_from(first_seq).context("negative first_message_seq")?,
        last_message_seq: u64::try_from(last_seq).context("negative last_message_seq")?,
    })
}

fn chat_mcp_command_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ChatMcpCommand> {
    let seq: i64 = row.try_get("command_seq")?;
    let status: String = row.try_get("status")?;
    let kind: String = row.try_get("command_type")?;
    Ok(ChatMcpCommand {
        seq: u64::try_from(seq).context("negative MCP command seq")?,
        room_id: row.try_get("room_id")?,
        created_at_ms: row.try_get("created_at_ms")?,
        claimed_at_ms: row.try_get("claimed_at_ms")?,
        completed_at_ms: row.try_get("completed_at_ms")?,
        status: ChatMcpCommandStatus::from_db(&status)?,
        kind: ChatMcpCommandKind::from_db(&kind)?,
        source_agent: parse_agent(row.try_get::<Option<String>, _>("source_agent")?)?,
        target_agent: parse_agent(row.try_get::<Option<String>, _>("target_agent")?)?,
        body: row.try_get("body")?,
        error: row.try_get("error")?,
    })
}

fn parse_agent(value: Option<String>) -> Result<Option<AgentName>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value.as_str() {
        "codex" => Ok(Some(AgentName::Codex)),
        "claude" => Ok(Some(AgentName::Claude)),
        "gemini" => Ok(Some(AgentName::Gemini)),
        "opencode" => Ok(Some(AgentName::Opencode)),
        other => anyhow::bail!("unknown agent in chat store: {other}"),
    }
}

const fn normalize_limit(limit: Option<u32>) -> u32 {
    match limit {
        Some(0) => DEFAULT_LIMIT,
        Some(limit) if limit > MAX_LIMIT => MAX_LIMIT,
        Some(limit) => limit,
        None => DEFAULT_LIMIT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn appends_and_paginates_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();

        for index in 0..3 {
            store
                .append_message(
                    "room-main",
                    NewChatMessage {
                        message_id: None,
                        created_at_ms: 10 + index,
                        event_type: ChatMessageType::UserMessage,
                        text: format!("message {index}"),
                        agent: Some(AgentName::Codex),
                        thread_id: Some("thread-1".into()),
                        thread_short_id: Some("thread-1".into()),
                        workspace_root: Some("/tmp/ws".into()),
                    },
                )
                .await
                .unwrap();
        }

        let first = store
            .list_messages_desc("room-main", None, Some(2))
            .await
            .unwrap();
        assert_eq!(
            first
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert!(first.has_more);
        assert_eq!(first.next_before_seq, Some(2));

        let second = store
            .list_messages_desc("room-main", first.next_before_seq, Some(2))
            .await
            .unwrap();
        assert_eq!(second.messages[0].seq, 1);
        assert!(!second.has_more);
    }

    #[tokio::test]
    async fn tracks_agent_sessions_separately_from_recent_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();
        store
            .append_message(
                "room-main",
                NewChatMessage {
                    message_id: None,
                    created_at_ms: 10,
                    event_type: ChatMessageType::AgentResult,
                    text: "done".into(),
                    agent: Some(AgentName::Gemini),
                    thread_id: Some("thread-gemini".into()),
                    thread_short_id: Some("thread-g".into()),
                    workspace_root: Some("/tmp/ws".into()),
                },
            )
            .await
            .unwrap();

        let sessions = store.list_agent_sessions("room-main").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent, AgentName::Gemini);
        assert_eq!(sessions[0].thread_id, "thread-gemini");
    }

    #[tokio::test]
    async fn most_recent_non_empty_room_id_ignores_empty_rooms() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-empty", "empty", "/tmp/empty")
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();
        store
            .append_message(
                "room-main",
                NewChatMessage {
                    message_id: None,
                    created_at_ms: 10,
                    event_type: ChatMessageType::UserMessage,
                    text: "hello".into(),
                    agent: Some(AgentName::Codex),
                    thread_id: None,
                    thread_short_id: None,
                    workspace_root: Some("/tmp/ws".into()),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            store.most_recent_non_empty_room_id().await.unwrap(),
            Some("room-main".into())
        );
    }

    #[tokio::test]
    async fn mcp_commands_can_be_claimed_and_completed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();

        let queued = store
            .enqueue_mcp_command(
                "room-main",
                NewChatMcpCommand {
                    kind: ChatMcpCommandKind::MentionAgent,
                    source_agent: Some(AgentName::Codex),
                    target_agent: Some(AgentName::Gemini),
                    body: "review this".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(queued.status, ChatMcpCommandStatus::Pending);

        let claimed = store
            .claim_pending_mcp_commands("room-main", Some(10))
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].seq, queued.seq);
        assert_eq!(claimed[0].status, ChatMcpCommandStatus::Claimed);
        assert_eq!(claimed[0].source_agent, Some(AgentName::Codex));
        assert_eq!(claimed[0].target_agent, Some(AgentName::Gemini));

        assert!(store
            .claim_pending_mcp_commands("room-main", Some(10))
            .await
            .unwrap()
            .is_empty());

        store.complete_mcp_command(queued.seq).await.unwrap();
        let command = store.get_mcp_command(queued.seq).await.unwrap().unwrap();
        assert_eq!(command.status, ChatMcpCommandStatus::Completed);
        assert!(command.completed_at_ms.is_some());
    }

    #[test]
    fn room_id_for_workspace_canonicalizes_relative_paths() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            room_id_for_workspace(Path::new(".")),
            room_id_for_workspace(&cwd)
        );
    }
}
