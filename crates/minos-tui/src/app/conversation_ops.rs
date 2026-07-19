//! Shared conversation RPC sequences used by effect execution.

use std::path::PathBuf;
use std::sync::Arc;

use minos_domain::AgentName;

use crate::backend::{AgentBackend, ConversationMessageEntry, ThreadSummaryEntry};

pub(super) struct OpenedConversation {
    pub project_id: String,
    pub conversation_id: String,
    pub messages: Vec<ConversationMessageEntry>,
    pub sessions: Vec<ThreadSummaryEntry>,
}

pub(super) struct StartedAgent {
    pub conversation_id: String,
    pub agent: AgentName,
    pub thread_id: String,
    pub cwd: PathBuf,
    pub text: String,
}

pub(super) fn conversation_title_from_prompt(prompt: &str) -> String {
    prompt
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("Untitled conversation")
        .chars()
        .take(80)
        .collect()
}

pub(super) async fn append_user_message_and_load(
    backend: &dyn AgentBackend,
    conversation_id: &str,
    message_body: &str,
) -> Result<(Vec<ConversationMessageEntry>, Vec<ThreadSummaryEntry>), String> {
    if !message_body.trim().is_empty() {
        backend
            .append_conversation_message(conversation_id, None, None, "user", None, message_body)
            .await
            .map_err(|error| format!("Failed to save conversation message: {error}"))?;
    }

    let messages = backend
        .list_conversation_messages(conversation_id)
        .await
        .map_err(|error| format!("Failed to load conversation messages: {error}"))?;
    let sessions = backend
        .list_conversation_agent_sessions(conversation_id)
        .await
        .map_err(|error| format!("Failed to load agent sessions: {error}"))?;
    // At most one top-level interrupted session auto-continues on open.
    if let Some(session) = pick_auto_continue_session(&sessions) {
        if let Err(error) = backend.resume_thread(&session.thread_id, true).await {
            tracing::warn!(
                target: "minos_tui::app",
                error = %error,
                thread_id = %session.thread_id,
                "auto-continue resume_thread failed"
            );
        }
    }
    Ok((messages, sessions))
}

/// Prefer most recently active top-level session with `needs_continue`.
pub(super) fn pick_auto_continue_session(
    sessions: &[ThreadSummaryEntry],
) -> Option<&ThreadSummaryEntry> {
    sessions
        .iter()
        .filter(|s| s.parent_thread_id.is_none() && s.needs_continue && s.ended_at_ms.is_none())
        .max_by_key(|s| s.last_ts_ms)
}

pub(super) async fn create_conversation_and_start_agent(
    backend: Arc<dyn AgentBackend>,
    project_id: String,
    agent: AgentName,
    workspace: PathBuf,
    message_body: String,
    prompt: String,
) -> Result<(OpenedConversation, StartedAgent), String> {
    let title = conversation_title_from_prompt(&prompt);
    let conversation = backend
        .create_conversation(&project_id, &title)
        .await
        .map_err(|error| format!("Failed to create conversation: {error}"))?;
    let conversation_id = conversation.conversation_id;
    let (messages, sessions) =
        append_user_message_and_load(backend.as_ref(), &conversation_id, &message_body).await?;
    let outcome = backend
        .start_agent_in_conversation(&conversation_id, agent, workspace)
        .await
        .map_err(|error| format!("Failed to start {}: {error}", agent.bin_name()))?;
    Ok((
        OpenedConversation {
            project_id,
            conversation_id: conversation_id.clone(),
            messages,
            sessions,
        },
        StartedAgent {
            conversation_id,
            agent,
            thread_id: outcome.thread_id,
            cwd: outcome.cwd,
            text: prompt,
        },
    ))
}

pub(super) async fn start_agent_in_existing_conversation(
    backend: Arc<dyn AgentBackend>,
    project_id: String,
    conversation_id: String,
    agent: AgentName,
    workspace: PathBuf,
    message_body: String,
    prompt: String,
) -> Result<(OpenedConversation, StartedAgent), String> {
    let (messages, sessions) =
        append_user_message_and_load(backend.as_ref(), &conversation_id, &message_body).await?;
    let outcome = backend
        .start_agent_in_conversation(&conversation_id, agent, workspace)
        .await
        .map_err(|error| format!("Failed to start {}: {error}", agent.bin_name()))?;
    Ok((
        OpenedConversation {
            project_id,
            conversation_id: conversation_id.clone(),
            messages,
            sessions,
        },
        StartedAgent {
            conversation_id,
            agent,
            thread_id: outcome.thread_id,
            cwd: outcome.cwd,
            text: prompt,
        },
    ))
}
