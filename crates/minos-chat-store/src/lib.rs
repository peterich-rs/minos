use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use minos_domain::AgentName;
use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

pub mod mcp_handler;
pub mod mcp_server;
pub mod mcp_socket;
pub mod teamwork_mcp;

const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 500;
const OPEN_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(25),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamworkDelegationStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamworkDelegation {
    pub delegation_id: String,
    pub room_id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub status: TeamworkDelegationStatus,
    pub source_agent: Option<AgentName>,
    pub target_agent: AgentName,
    pub prompt: String,
    pub thread_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserFeedbackStatus {
    Pending,
    Answered,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserFeedbackRequest {
    pub feedback_id: String,
    pub room_id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub status: UserFeedbackStatus,
    pub source_agent: Option<AgentName>,
    pub question: String,
    pub question_message_seq: u64,
    pub answer_message_seq: Option<u64>,
    pub answer_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageReactionAction {
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReactionResult {
    pub room_id: String,
    pub message_id: String,
    pub message_seq: u64,
    pub emoji: String,
    pub reactor: String,
    pub action: MessageReactionAction,
    pub active: bool,
}

impl ChatStore {
    pub async fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create chat db parent dir {}", parent.display())
            })?;
        }

        for (attempt, delay) in OPEN_RETRY_DELAYS.iter().enumerate() {
            match Self::open_once(db_path).await {
                Ok(store) => return Ok(store),
                Err(error) if is_sqlite_busy_error(&error) => {
                    if attempt == OPEN_RETRY_DELAYS.len() - 1 {
                        return Err(error);
                    }
                    tokio::time::sleep(*delay).await;
                }
                Err(error) => return Err(error),
            }
        }

        Self::open_once(db_path).await
    }

    async fn open_once(db_path: &Path) -> Result<Self> {
        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let opts = SqliteConnectOptions::from_str(&url)?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10));
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

    pub async fn upsert_message_by_id(
        &self,
        room_id: &str,
        input: NewChatMessage,
    ) -> Result<ChatMessage> {
        let Some(message_id) = input.message_id.clone().filter(|id| !id.trim().is_empty()) else {
            return self.append_message(room_id, input).await;
        };

        let mut tx = self.pool.begin().await?;
        let now = chrono::Utc::now().timestamp_millis();
        let update_at_ms = if input.created_at_ms == 0 {
            now
        } else {
            input.created_at_ms
        };
        let existing = sqlx::query(
            "SELECT message_seq, created_at_ms FROM chat_messages \
             WHERE room_id = ? AND message_id = ?",
        )
        .bind(room_id)
        .bind(&message_id)
        .fetch_optional(&mut *tx)
        .await?;
        let sender_role = input.event_type.sender_role();

        let (seq_i64, created_at_ms) = if let Some(row) = existing {
            let seq_i64: i64 = row.get("message_seq");
            let created_at_ms: i64 = row.get("created_at_ms");
            sqlx::query(
                "UPDATE chat_messages SET \
                    sender_role = ?, event_type = ?, body = ?, agent = ?, thread_id = ?, \
                    thread_short_id = ?, workspace_root = ? \
                 WHERE room_id = ? AND message_id = ?",
            )
            .bind(sender_role.as_db())
            .bind(input.event_type.as_db())
            .bind(&input.text)
            .bind(input.agent.map(|agent| agent.bin_name().to_owned()))
            .bind(input.thread_id.as_deref())
            .bind(input.thread_short_id.as_deref())
            .bind(input.workspace_root.as_deref())
            .bind(room_id)
            .bind(&message_id)
            .execute(&mut *tx)
            .await?;
            (seq_i64, created_at_ms)
        } else {
            let seq_i64: i64 =
                sqlx::query_scalar("SELECT COALESCE(MAX(message_seq), 0) + 1 FROM chat_messages")
                    .fetch_one(&mut *tx)
                    .await?;
            sqlx::query(
                "INSERT INTO chat_messages( \
                    message_seq, message_id, room_id, created_at_ms, sender_role, event_type, \
                    body, agent, thread_id, thread_short_id, workspace_root \
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(seq_i64)
            .bind(&message_id)
            .bind(room_id)
            .bind(update_at_ms)
            .bind(sender_role.as_db())
            .bind(input.event_type.as_db())
            .bind(&input.text)
            .bind(input.agent.map(|agent| agent.bin_name().to_owned()))
            .bind(input.thread_id.as_deref())
            .bind(input.thread_short_id.as_deref())
            .bind(input.workspace_root.as_deref())
            .execute(&mut *tx)
            .await?;
            (seq_i64, update_at_ms)
        };

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
            .bind(update_at_ms)
            .bind(update_at_ms)
            .bind(seq_i64)
            .bind(seq_i64)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query("UPDATE chat_rooms SET updated_at_ms = ? WHERE room_id = ?")
            .bind(update_at_ms)
            .bind(room_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        let seq = u64::try_from(seq_i64).context("chat message seq overflow")?;

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

    pub async fn create_delegation(
        &self,
        room_id: &str,
        source_agent: Option<AgentName>,
        target_agent: AgentName,
        prompt: String,
        thread_id: Option<String>,
    ) -> Result<TeamworkDelegation> {
        let prompt = prompt.trim().to_owned();
        anyhow::ensure!(!prompt.is_empty(), "delegation prompt must not be empty");
        let now = chrono::Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await?;
        let seq_i64: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(delegation_seq), 0) + 1 FROM teamwork_delegations",
        )
        .fetch_one(&mut *tx)
        .await?;
        let delegation_id = format!("delegation-{seq_i64}");
        let status = TeamworkDelegationStatus::Running;
        sqlx::query(
            "INSERT INTO teamwork_delegations( \
                delegation_seq, delegation_id, room_id, created_at_ms, updated_at_ms, \
                status, source_agent, target_agent, prompt, thread_id, error \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(seq_i64)
        .bind(&delegation_id)
        .bind(room_id)
        .bind(now)
        .bind(now)
        .bind(status.as_db())
        .bind(source_agent.map(|agent| agent.bin_name().to_owned()))
        .bind(target_agent.bin_name())
        .bind(&prompt)
        .bind(thread_id.as_deref())
        .bind(Option::<String>::None)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(TeamworkDelegation {
            delegation_id,
            room_id: room_id.to_owned(),
            created_at_ms: now,
            updated_at_ms: now,
            status,
            source_agent,
            target_agent,
            prompt,
            thread_id,
            error: None,
        })
    }

    pub async fn get_delegation(
        &self,
        room_id: &str,
        delegation_id: &str,
    ) -> Result<Option<TeamworkDelegation>> {
        let row = sqlx::query(
            "SELECT * FROM teamwork_delegations \
             WHERE room_id = ? AND delegation_id = ?",
        )
        .bind(room_id)
        .bind(delegation_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(delegation_from_row).transpose()
    }

    pub async fn cancel_delegation(
        &self,
        room_id: &str,
        delegation_id: &str,
        reason: Option<String>,
    ) -> Result<TeamworkDelegation> {
        let current = self
            .get_delegation(room_id, delegation_id)
            .await?
            .with_context(|| format!("delegation not found: {delegation_id}"))?;
        anyhow::ensure!(
            !current.status.is_terminal(),
            "delegation {delegation_id} is already {:?}",
            current.status
        );
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE teamwork_delegations \
             SET status = ?, updated_at_ms = ?, error = ? \
             WHERE room_id = ? AND delegation_id = ?",
        )
        .bind(TeamworkDelegationStatus::Cancelled.as_db())
        .bind(now)
        .bind(reason.as_deref())
        .bind(room_id)
        .bind(delegation_id)
        .execute(&self.pool)
        .await?;
        self.get_delegation(room_id, delegation_id)
            .await?
            .with_context(|| format!("delegation disappeared after cancel: {delegation_id}"))
    }

    pub async fn create_user_feedback(
        &self,
        room_id: &str,
        source_agent: Option<AgentName>,
        question: String,
        question_message_seq: u64,
    ) -> Result<UserFeedbackRequest> {
        let question = question.trim().to_owned();
        anyhow::ensure!(!question.is_empty(), "feedback question must not be empty");
        let now = chrono::Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await?;
        let seq_i64: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(feedback_seq), 0) + 1 FROM teamwork_user_feedback",
        )
        .fetch_one(&mut *tx)
        .await?;
        let feedback_id = format!("feedback-{seq_i64}");
        let status = UserFeedbackStatus::Pending;
        sqlx::query(
            "INSERT INTO teamwork_user_feedback( \
                feedback_seq, feedback_id, room_id, created_at_ms, updated_at_ms, \
                status, source_agent, question, question_message_seq, \
                answer_message_seq, answer_text \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(seq_i64)
        .bind(&feedback_id)
        .bind(room_id)
        .bind(now)
        .bind(now)
        .bind(status.as_db())
        .bind(source_agent.map(|agent| agent.bin_name().to_owned()))
        .bind(&question)
        .bind(question_message_seq as i64)
        .bind(Option::<i64>::None)
        .bind(Option::<String>::None)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(UserFeedbackRequest {
            feedback_id,
            room_id: room_id.to_owned(),
            created_at_ms: now,
            updated_at_ms: now,
            status,
            source_agent,
            question,
            question_message_seq,
            answer_message_seq: None,
            answer_text: None,
        })
    }

    pub async fn check_user_feedback(
        &self,
        room_id: &str,
        feedback_id: &str,
    ) -> Result<UserFeedbackRequest> {
        let current = self
            .get_user_feedback(room_id, feedback_id)
            .await?
            .with_context(|| format!("feedback request not found: {feedback_id}"))?;
        if current.status != UserFeedbackStatus::Pending {
            return Ok(current);
        }

        let answer = sqlx::query(
            "SELECT * FROM chat_messages \
             WHERE room_id = ? AND message_seq > ? AND sender_role = 'user' \
             ORDER BY message_seq ASC LIMIT 1",
        )
        .bind(room_id)
        .bind(current.question_message_seq as i64)
        .fetch_optional(&self.pool)
        .await?;

        let Some(answer) = answer else {
            return Ok(current);
        };
        let answer = chat_message_from_row(answer)?;
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE teamwork_user_feedback \
             SET status = ?, updated_at_ms = ?, answer_message_seq = ?, answer_text = ? \
             WHERE room_id = ? AND feedback_id = ?",
        )
        .bind(UserFeedbackStatus::Answered.as_db())
        .bind(now)
        .bind(answer.seq as i64)
        .bind(&answer.text)
        .bind(room_id)
        .bind(feedback_id)
        .execute(&self.pool)
        .await?;

        self.get_user_feedback(room_id, feedback_id)
            .await?
            .with_context(|| format!("feedback request disappeared after update: {feedback_id}"))
    }

    pub async fn react_to_message(
        &self,
        room_id: &str,
        source_agent: Option<AgentName>,
        message_id: Option<String>,
        message_seq: Option<u64>,
        emoji: String,
        action: MessageReactionAction,
    ) -> Result<MessageReactionResult> {
        let emoji = emoji.trim().to_owned();
        anyhow::ensure!(!emoji.is_empty(), "emoji must not be empty");
        let message = self
            .resolve_message_ref(room_id, message_id.as_deref(), message_seq)
            .await?;
        let reactor = source_agent
            .map(|agent| agent.bin_name().to_owned())
            .unwrap_or_else(|| "agent".to_owned());
        match action {
            MessageReactionAction::Add => {
                sqlx::query(
                    "INSERT INTO teamwork_message_reactions( \
                        room_id, message_id, message_seq, emoji, reactor, created_at_ms \
                     ) VALUES (?, ?, ?, ?, ?, ?) \
                     ON CONFLICT(room_id, message_id, emoji, reactor) DO UPDATE SET \
                        message_seq = excluded.message_seq",
                )
                .bind(room_id)
                .bind(&message.message_id)
                .bind(message.seq as i64)
                .bind(&emoji)
                .bind(&reactor)
                .bind(chrono::Utc::now().timestamp_millis())
                .execute(&self.pool)
                .await?;
            }
            MessageReactionAction::Remove => {
                sqlx::query(
                    "DELETE FROM teamwork_message_reactions \
                     WHERE room_id = ? AND message_id = ? AND emoji = ? AND reactor = ?",
                )
                .bind(room_id)
                .bind(&message.message_id)
                .bind(&emoji)
                .bind(&reactor)
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(MessageReactionResult {
            room_id: room_id.to_owned(),
            message_id: message.message_id,
            message_seq: message.seq,
            emoji,
            reactor,
            action,
            active: action == MessageReactionAction::Add,
        })
    }

    async fn get_user_feedback(
        &self,
        room_id: &str,
        feedback_id: &str,
    ) -> Result<Option<UserFeedbackRequest>> {
        let row = sqlx::query(
            "SELECT * FROM teamwork_user_feedback \
             WHERE room_id = ? AND feedback_id = ?",
        )
        .bind(room_id)
        .bind(feedback_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(user_feedback_from_row).transpose()
    }

    async fn resolve_message_ref(
        &self,
        room_id: &str,
        message_id: Option<&str>,
        message_seq: Option<u64>,
    ) -> Result<ChatMessage> {
        anyhow::ensure!(
            message_id.is_some() || message_seq.is_some(),
            "message_id or message_seq is required"
        );
        let row = match (message_id, message_seq) {
            (Some(message_id), Some(message_seq)) => {
                sqlx::query(
                    "SELECT * FROM chat_messages \
                     WHERE room_id = ? AND message_id = ? AND message_seq = ?",
                )
                .bind(room_id)
                .bind(message_id)
                .bind(message_seq as i64)
                .fetch_optional(&self.pool)
                .await?
            }
            (Some(message_id), None) => {
                sqlx::query("SELECT * FROM chat_messages WHERE room_id = ? AND message_id = ?")
                    .bind(room_id)
                    .bind(message_id)
                    .fetch_optional(&self.pool)
                    .await?
            }
            (None, Some(message_seq)) => {
                sqlx::query("SELECT * FROM chat_messages WHERE room_id = ? AND message_seq = ?")
                    .bind(room_id)
                    .bind(message_seq as i64)
                    .fetch_optional(&self.pool)
                    .await?
            }
            (None, None) => unreachable!("validated above"),
        };
        row.map(chat_message_from_row)
            .transpose()?
            .with_context(|| "message not found for reaction".to_owned())
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
            "CREATE TABLE IF NOT EXISTS teamwork_delegations ( \
                delegation_seq INTEGER PRIMARY KEY, \
                delegation_id TEXT NOT NULL UNIQUE, \
                room_id TEXT NOT NULL REFERENCES chat_rooms(room_id), \
                created_at_ms INTEGER NOT NULL, \
                updated_at_ms INTEGER NOT NULL, \
                status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'cancelled', 'failed')), \
                source_agent TEXT, \
                target_agent TEXT NOT NULL, \
                prompt TEXT NOT NULL, \
                thread_id TEXT, \
                error TEXT \
             )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS teamwork_delegations_by_room_status \
             ON teamwork_delegations(room_id, status, delegation_seq DESC)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS teamwork_user_feedback ( \
                feedback_seq INTEGER PRIMARY KEY, \
                feedback_id TEXT NOT NULL UNIQUE, \
                room_id TEXT NOT NULL REFERENCES chat_rooms(room_id), \
                created_at_ms INTEGER NOT NULL, \
                updated_at_ms INTEGER NOT NULL, \
                status TEXT NOT NULL CHECK(status IN ('pending', 'answered', 'cancelled')), \
                source_agent TEXT, \
                question TEXT NOT NULL, \
                question_message_seq INTEGER NOT NULL, \
                answer_message_seq INTEGER, \
                answer_text TEXT \
             )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS teamwork_user_feedback_by_room_status \
             ON teamwork_user_feedback(room_id, status, feedback_seq DESC)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS teamwork_message_reactions ( \
                room_id TEXT NOT NULL REFERENCES chat_rooms(room_id), \
                message_id TEXT NOT NULL, \
                message_seq INTEGER NOT NULL, \
                emoji TEXT NOT NULL, \
                reactor TEXT NOT NULL, \
                created_at_ms INTEGER NOT NULL, \
                PRIMARY KEY(room_id, message_id, emoji, reactor) \
             )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS teamwork_message_reactions_by_message \
             ON teamwork_message_reactions(room_id, message_id)",
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

impl TeamworkDelegationStatus {
    const fn as_db(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    fn from_db(value: &str) -> Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown teamwork delegation status: {other}"),
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

impl UserFeedbackStatus {
    const fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Answered => "answered",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_db(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "answered" => Ok(Self::Answered),
            "cancelled" => Ok(Self::Cancelled),
            other => anyhow::bail!("unknown user feedback status: {other}"),
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

fn is_sqlite_busy_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string();
        message.contains("database is locked")
            || message.contains("database table is locked")
            || message.contains("SQLITE_BUSY")
            || message.contains("SQLITE_LOCKED")
            || message.contains("(code: 5)")
            || message.contains("(code: 6)")
    })
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

fn delegation_from_row(row: sqlx::sqlite::SqliteRow) -> Result<TeamworkDelegation> {
    let status: String = row.try_get("status")?;
    let target_agent: String = row.try_get("target_agent")?;
    Ok(TeamworkDelegation {
        delegation_id: row.try_get("delegation_id")?,
        room_id: row.try_get("room_id")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
        status: TeamworkDelegationStatus::from_db(&status)?,
        source_agent: parse_agent(row.try_get::<Option<String>, _>("source_agent")?)?,
        target_agent: parse_agent(Some(target_agent))?
            .context("teamwork_delegations.target_agent is NULL")?,
        prompt: row.try_get("prompt")?,
        thread_id: row.try_get("thread_id")?,
        error: row.try_get("error")?,
    })
}

fn user_feedback_from_row(row: sqlx::sqlite::SqliteRow) -> Result<UserFeedbackRequest> {
    let status: String = row.try_get("status")?;
    let question_message_seq: i64 = row.try_get("question_message_seq")?;
    let answer_message_seq: Option<i64> = row.try_get("answer_message_seq")?;
    Ok(UserFeedbackRequest {
        feedback_id: row.try_get("feedback_id")?,
        room_id: row.try_get("room_id")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
        status: UserFeedbackStatus::from_db(&status)?,
        source_agent: parse_agent(row.try_get::<Option<String>, _>("source_agent")?)?,
        question: row.try_get("question")?,
        question_message_seq: u64::try_from(question_message_seq)
            .context("negative feedback question_message_seq")?,
        answer_message_seq: answer_message_seq
            .map(|seq| u64::try_from(seq).context("negative feedback answer_message_seq"))
            .transpose()?,
        answer_text: row.try_get("answer_text")?,
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
    async fn upsert_message_by_id_updates_existing_row_without_advancing_seq() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();

        let first = store
            .upsert_message_by_id(
                "room-main",
                NewChatMessage {
                    message_id: Some("agent-result:thread-1:msg-1".into()),
                    created_at_ms: 10,
                    event_type: ChatMessageType::AgentResult,
                    text: "Hel".into(),
                    agent: Some(AgentName::Codex),
                    thread_id: Some("thread-1".into()),
                    thread_short_id: Some("thread-1".into()),
                    workspace_root: Some("/tmp/ws".into()),
                },
            )
            .await
            .unwrap();
        let second = store
            .upsert_message_by_id(
                "room-main",
                NewChatMessage {
                    message_id: Some("agent-result:thread-1:msg-1".into()),
                    created_at_ms: 20,
                    event_type: ChatMessageType::AgentResult,
                    text: "Hello".into(),
                    agent: Some(AgentName::Codex),
                    thread_id: Some("thread-1".into()),
                    thread_short_id: Some("thread-1".into()),
                    workspace_root: Some("/tmp/ws".into()),
                },
            )
            .await
            .unwrap();

        assert_eq!(second.seq, first.seq);
        assert_eq!(second.created_at_ms, first.created_at_ms);
        assert_eq!(second.text, "Hello");
        assert_eq!(store.count_messages("room-main").await.unwrap(), 1);
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
    async fn delegation_can_be_created_read_and_cancelled() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();

        let delegation = store
            .create_delegation(
                "room-main",
                Some(AgentName::Codex),
                AgentName::Gemini,
                "check the failing test".into(),
                Some("thread-gemini".into()),
            )
            .await
            .unwrap();

        assert_eq!(delegation.delegation_id, "delegation-1");
        assert_eq!(delegation.status, TeamworkDelegationStatus::Running);
        assert_eq!(
            store
                .get_delegation("room-main", &delegation.delegation_id)
                .await
                .unwrap()
                .unwrap()
                .target_agent,
            AgentName::Gemini
        );

        let cancelled = store
            .cancel_delegation(
                "room-main",
                &delegation.delegation_id,
                Some("no longer needed".into()),
            )
            .await
            .unwrap();
        assert_eq!(cancelled.status, TeamworkDelegationStatus::Cancelled);
        assert_eq!(cancelled.error.as_deref(), Some("no longer needed"));
    }

    #[tokio::test]
    async fn user_feedback_check_records_first_user_reply_after_question() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();

        let question = store
            .append_message(
                "room-main",
                NewChatMessage {
                    message_id: None,
                    created_at_ms: 10,
                    event_type: ChatMessageType::AgentResult,
                    text: "@user Which option?".into(),
                    agent: Some(AgentName::Codex),
                    thread_id: None,
                    thread_short_id: None,
                    workspace_root: Some("/tmp/ws".into()),
                },
            )
            .await
            .unwrap();
        let feedback = store
            .create_user_feedback(
                "room-main",
                Some(AgentName::Codex),
                "Which option?".into(),
                question.seq,
            )
            .await
            .unwrap();
        assert_eq!(feedback.status, UserFeedbackStatus::Pending);

        let pending = store
            .check_user_feedback("room-main", &feedback.feedback_id)
            .await
            .unwrap();
        assert_eq!(pending.status, UserFeedbackStatus::Pending);

        store
            .append_message(
                "room-main",
                NewChatMessage {
                    message_id: None,
                    created_at_ms: 20,
                    event_type: ChatMessageType::UserMessage,
                    text: "Option B".into(),
                    agent: None,
                    thread_id: None,
                    thread_short_id: None,
                    workspace_root: Some("/tmp/ws".into()),
                },
            )
            .await
            .unwrap();

        let answered = store
            .check_user_feedback("room-main", &feedback.feedback_id)
            .await
            .unwrap();
        assert_eq!(answered.status, UserFeedbackStatus::Answered);
        assert_eq!(answered.answer_text.as_deref(), Some("Option B"));
    }

    #[tokio::test]
    async fn reactions_can_be_added_and_removed_by_message_seq() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();
        let message = store
            .append_message(
                "room-main",
                NewChatMessage {
                    message_id: None,
                    created_at_ms: 10,
                    event_type: ChatMessageType::AgentResult,
                    text: "done".into(),
                    agent: Some(AgentName::Gemini),
                    thread_id: None,
                    thread_short_id: None,
                    workspace_root: Some("/tmp/ws".into()),
                },
            )
            .await
            .unwrap();

        let added = store
            .react_to_message(
                "room-main",
                Some(AgentName::Codex),
                None,
                Some(message.seq),
                "+1".into(),
                MessageReactionAction::Add,
            )
            .await
            .unwrap();
        assert!(added.active);
        assert_eq!(added.message_id, message.message_id);

        let removed = store
            .react_to_message(
                "room-main",
                Some(AgentName::Codex),
                None,
                Some(message.seq),
                "+1".into(),
                MessageReactionAction::Remove,
            )
            .await
            .unwrap();
        assert!(!removed.active);
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
