use super::{
    AgentBackend, BackendConnectionState, BackendThreadSnapshot, ConversationEntry,
    ConversationMessageEntry, ProjectEntry, ThreadSummaryEntry,
};
use anyhow::Result;
use async_trait::async_trait;
use minos_agent_runtime::{AgentManager, InstanceCaps, ManagerEvent, StartAgentOutcome};
use minos_cli_detect::{capture_user_shell_env, detect_all, RealCommandRunner};
use minos_domain::AgentName;
use minos_protocol::local_rpc::ReadThreadRawHistoryResponse;
use minos_protocol::LocalGroupChatMessage;
use minos_protocol::LocalIngestFrame;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::event::{AppEvent, McpToolEvent};
use crate::translation::AgentTranslationState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct EmbeddedBackend {
    manager: Arc<AgentManager>,
    mcp_socket_path: Option<PathBuf>,
    workspace: PathBuf,
    projects: std::sync::Mutex<Vec<ProjectEntry>>,
    conversations: std::sync::Mutex<Vec<ConversationEntry>>,
    conversation_messages: std::sync::Mutex<HashMap<String, Vec<ConversationMessageEntry>>>,
    conversation_sessions: std::sync::Mutex<HashMap<String, Vec<ThreadSummaryEntry>>>,
}

impl EmbeddedBackend {
    pub async fn new(
        workspace_root: PathBuf,
        max_instances: usize,
        idle_timeout: std::time::Duration,
        mcp_permissions: minos_chat_store::mcp_server::McpToolPermissions,
    ) -> Result<Self> {
        let workspace = workspace_root.clone();
        let shell_env = capture_user_shell_env().await;
        let mut config = minos_agent_runtime::AgentRuntimeConfig::new(workspace_root);
        let db_path = minos_chat_store::default_db_path()?;
        let minos_home = db_path.parent().expect("db_path parent").to_path_buf();
        let socket_path = {
            let id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            minos_home.join("run").join(format!("mcp-{id}.sock"))
        };
        let mcp_result = config.enable_default_mcp_with_socket_path(socket_path);
        if let Some(mcp) = config.mcp.as_mut() {
            mcp.permissions = mcp_permissions;
        }
        if let Err(error) = mcp_result {
            tracing::warn!(
                target: "minos_tui::backend::embedded",
                error = %error,
                "failed to enable default MCP"
            );
        }
        let mcp_socket_path = config.mcp.as_ref().map(|mcp| mcp.socket_path.clone());
        config.subprocess_env = Arc::new(shell_env);
        let caps = InstanceCaps {
            max_instances,
            idle_timeout,
        };
        let manager = AgentManager::new(config, caps);
        Ok(Self {
            manager: Arc::new(manager),
            mcp_socket_path,
            workspace,
            projects: std::sync::Mutex::new(Vec::new()),
            conversations: std::sync::Mutex::new(Vec::new()),
            conversation_messages: std::sync::Mutex::new(HashMap::new()),
            conversation_sessions: std::sync::Mutex::new(HashMap::new()),
        })
    }
}

#[async_trait]
impl AgentBackend for EmbeddedBackend {
    async fn detect_clis(&self) -> Result<Vec<minos_domain::AgentDescriptor>> {
        let env = capture_user_shell_env().await;
        let runner = Arc::new(RealCommandRunner::new(Arc::new(env)));
        Ok(detect_all(runner).await)
    }

    async fn start_agent(&self, agent: AgentName, workspace: PathBuf) -> Result<StartAgentOutcome> {
        self.manager
            .start_agent(agent, workspace)
            .await
            .map_err(Into::into)
    }

    async fn send_message(&self, thread_id: &str, text: &str) -> Result<()> {
        self.manager
            .send_user_message(thread_id, text.to_owned())
            .await
            .map_err(Into::into)
    }

    async fn send_approval_decision(
        &self,
        request_id: &str,
        thread_id: &str,
        decision: Value,
    ) -> Result<()> {
        self.manager
            .resolve_approval(request_id, thread_id, decision)
            .await
            .map_err(Into::into)
    }

    async fn respond_opencode_permission(
        &self,
        thread_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<()> {
        self.manager
            .respond_opencode_permission(thread_id, permission_id, response)
            .await
            .map_err(Into::into)
    }

    async fn respond_opencode_question(
        &self,
        thread_id: &str,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> Result<()> {
        self.manager
            .respond_opencode_question(thread_id, question_id, answers)
            .await
            .map_err(Into::into)
    }

    async fn interrupt_thread(&self, thread_id: &str) -> Result<()> {
        self.manager
            .interrupt_thread(thread_id)
            .await
            .map_err(Into::into)
    }

    async fn close_thread(&self, thread_id: &str) -> Result<()> {
        self.manager
            .close_thread(thread_id)
            .await
            .map_err(Into::into)
    }

    async fn delete_thread(&self, thread_id: &str) -> Result<()> {
        self.close_thread(thread_id).await
    }

    async fn list_threads(&self) -> Result<Vec<BackendThreadSnapshot>> {
        Ok(self
            .manager
            .list_threads()
            .await
            .into_iter()
            .map(|thread| BackendThreadSnapshot {
                thread_id: thread.thread_id,
                agent: None,
                workspace: thread.workspace,
                state: thread.state,
            })
            .collect())
    }

    async fn list_projects(&self) -> Result<Vec<ProjectEntry>> {
        let projects = self.projects.lock().expect("projects lock").clone();
        if projects.is_empty() {
            let cwd = self.workspace.clone();
            let name = cwd
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".to_owned());
            Ok(vec![ProjectEntry {
                project_id: format!("embedded-{}", name),
                name,
                workspace_path: cwd,
                thread_count: 0,
                created_at_ms: 0,
                updated_at_ms: 0,
            }])
        } else {
            Ok(projects)
        }
    }

    async fn create_project(&self, name: &str, workspace_path: &Path) -> Result<ProjectEntry> {
        let entry = ProjectEntry {
            project_id: format!("embedded-{}", name),
            name: name.to_owned(),
            workspace_path: workspace_path.to_path_buf(),
            thread_count: 0,
            created_at_ms: now_ms(),
            updated_at_ms: now_ms(),
        };
        self.projects
            .lock()
            .expect("projects lock")
            .push(entry.clone());
        Ok(entry)
    }

    async fn list_conversations(&self, project_id: &str) -> Result<Vec<ConversationEntry>> {
        Ok(self
            .conversations
            .lock()
            .expect("conversations lock")
            .iter()
            .filter(|conversation| conversation.project_id == project_id)
            .cloned()
            .collect())
    }

    async fn create_conversation(
        &self,
        project_id: &str,
        title: &str,
    ) -> Result<ConversationEntry> {
        let now = now_ms();
        let entry = ConversationEntry {
            conversation_id: format!("embedded-conversation-{now}"),
            project_id: project_id.to_owned(),
            title: title.to_owned(),
            last_message_preview: None,
            created_at_ms: now,
            updated_at_ms: now,
            message_count: 0,
            agent_session_count: 0,
            participating_agents: Vec::new(),
        };
        self.conversations
            .lock()
            .expect("conversations lock")
            .push(entry.clone());
        Ok(entry)
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationMessageEntry>> {
        Ok(self
            .conversation_messages
            .lock()
            .expect("conversation messages lock")
            .get(conversation_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn list_conversation_agent_sessions(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ThreadSummaryEntry>> {
        Ok(self
            .conversation_sessions
            .lock()
            .expect("conversation sessions lock")
            .get(conversation_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn start_agent_in_conversation(
        &self,
        conversation_id: &str,
        agent: AgentName,
        workspace: PathBuf,
    ) -> Result<StartAgentOutcome> {
        let outcome = self.start_agent(agent, workspace).await?;
        self.conversation_sessions
            .lock()
            .expect("conversation sessions lock")
            .entry(conversation_id.to_owned())
            .or_default()
            .push(ThreadSummaryEntry {
                thread_id: outcome.thread_id.clone(),
                agent,
                title: None,
                first_ts_ms: now_ms(),
                last_ts_ms: now_ms(),
                message_count: 0,
                ended_at_ms: None,
            });
        if let Some(conversation) = self
            .conversations
            .lock()
            .expect("conversations lock")
            .iter_mut()
            .find(|conversation| conversation.conversation_id == conversation_id)
        {
            conversation.agent_session_count = conversation.agent_session_count.saturating_add(1);
            conversation.updated_at_ms = now_ms();
            if !conversation.participating_agents.contains(&agent) {
                conversation.participating_agents.push(agent);
            }
        }
        Ok(outcome)
    }

    async fn append_conversation_message(
        &self,
        conversation_id: &str,
        thread_id: Option<&str>,
        sender_role: &str,
        agent: Option<AgentName>,
        body: &str,
    ) -> Result<()> {
        let mut messages = self
            .conversation_messages
            .lock()
            .expect("conversation messages lock");
        let conversation_messages = messages.entry(conversation_id.to_owned()).or_default();
        let now = now_ms();
        conversation_messages.push(ConversationMessageEntry {
            message_seq: i64::try_from(conversation_messages.len() + 1).unwrap_or(i64::MAX),
            message_id: format!("embedded-message-{now}"),
            conversation_id: conversation_id.to_owned(),
            thread_id: thread_id.map(str::to_owned),
            created_at_ms: now,
            sender_role: sender_role.to_owned(),
            agent,
            body: body.to_owned(),
        });
        if let Some(conversation) = self
            .conversations
            .lock()
            .expect("conversations lock")
            .iter_mut()
            .find(|conversation| conversation.conversation_id == conversation_id)
        {
            conversation.message_count = conversation.message_count.saturating_add(1);
            conversation.last_message_preview = Some(body.chars().take(120).collect());
            conversation.updated_at_ms = now;
        }
        Ok(())
    }

    async fn resume_thread(&self, _thread_id: &str) -> Result<StartAgentOutcome> {
        Err(anyhow::anyhow!(
            "embedded mode does not support thread resumption"
        ))
    }

    async fn read_thread_raw_history(
        &self,
        _thread_id: &str,
        _from_seq: Option<u64>,
        _limit: u32,
    ) -> Result<ReadThreadRawHistoryResponse> {
        Ok(ReadThreadRawHistoryResponse {
            events: Vec::new(),
            next_seq: None,
        })
    }

    async fn read_group_chat(
        &self,
        _room_id: &str,
        _after_seq: Option<u64>,
        _before_seq: Option<u64>,
        _limit: u32,
    ) -> Result<Vec<LocalGroupChatMessage>> {
        Err(anyhow::anyhow!(
            "embedded mode does not expose group chat RPC"
        ))
    }

    async fn subscribe_ingest(&self) -> broadcast::Receiver<LocalIngestFrame> {
        let mut raw_rx = self.manager.ingest_stream();
        let (tx, rx) = broadcast::channel(256);
        tokio::spawn(async move {
            let mut translators: HashMap<String, AgentTranslationState> = HashMap::new();
            loop {
                match raw_rx.recv().await {
                    Ok(ingest) => {
                        let Some(payload) = ingest.json_value() else {
                            continue;
                        };
                        let translator = translators
                            .entry(ingest.thread_id.clone())
                            .or_insert_with(|| {
                                AgentTranslationState::new(ingest.agent, ingest.thread_id.clone())
                            });
                        let ui_events = translator.translate(&payload);
                        let _ = tx.send(LocalIngestFrame {
                            thread_id: ingest.thread_id,
                            seq: 0,
                            agent: ingest.agent,
                            ui_events,
                            ts_ms: ingest.ts_ms,
                        });
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        rx
    }

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager.manager_event_stream()
    }

    fn start_mcp_socket_handler(
        &self,
        event_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    ) -> Result<()> {
        let Some(socket_path) = self.mcp_socket_path.clone() else {
            return Ok(());
        };
        let callback: minos_chat_store::mcp_handler::ToolCallback = Arc::new(move |request| {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                event_tx
                    .send(AppEvent::McpToolCall(McpToolEvent {
                        request,
                        response_tx,
                    }))
                    .map_err(|_| anyhow::anyhow!("TUI event loop is closed"))?;
                response_rx
                    .await
                    .map_err(|_| anyhow::anyhow!("TUI dropped MCP socket response"))?
            })
        });
        tokio::spawn(async move {
            let handler =
                minos_chat_store::mcp_handler::McpSocketHandler::new(socket_path, callback);
            if let Err(error) = handler.run().await {
                tracing::warn!(
                    target: "minos_tui::backend::embedded",
                    error = %error,
                    "MCP socket handler stopped"
                );
            }
        });
        Ok(())
    }

    fn connection_state(&self) -> BackendConnectionState {
        BackendConnectionState::Embedded
    }
}
