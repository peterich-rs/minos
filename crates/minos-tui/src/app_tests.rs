use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use crate::backend::{BackendConnectionState, BackendThreadSnapshot};
use crate::translation::ChatItem;
use anyhow::Result;
use async_trait::async_trait;
use crossterm::event::{KeyEventState, KeyModifiers, MouseEvent, MouseEventKind};
use minos_agent_runtime::StartAgentOutcome;
use minos_domain::{AgentDescriptor, AgentName, AgentStatus};
use minos_protocol::local_rpc::ReadThreadRawHistoryResponse;
use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind, LocalIngestFrame};
use minos_ui_protocol::{MessageRole, UiEventMessage};
use ratatui::layout::Rect;
use tokio::sync::broadcast;

use super::*;

struct TestBackend {
    detected_agents: Vec<AgentDescriptor>,
    started: Mutex<Vec<AgentName>>,
    sent_messages: Mutex<Vec<(String, String)>>,
    approval_decisions: Mutex<Vec<(String, String, serde_json::Value)>>,
    opencode_permission_responses: Mutex<Vec<(String, String, String)>>,
    opencode_question_responses: Mutex<Vec<(String, String, Vec<Vec<String>>)>>,
    group_chat_pages: Mutex<VecDeque<Vec<LocalGroupChatMessage>>>,
    next_thread: Mutex<usize>,
    interrupted: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
    deleted: Mutex<Vec<String>>,
    listed_threads: Mutex<Vec<BackendThreadSnapshot>>,
    history_pages: Mutex<HashMap<String, VecDeque<ReadThreadRawHistoryResponse>>>,
    history_calls: Mutex<Vec<(String, Option<u64>, u32)>>,
    projects: Mutex<Vec<crate::backend::ProjectEntry>>,
    created_projects: Mutex<Vec<(String, std::path::PathBuf)>>,
    project_thread_lists: Mutex<Vec<(String, Vec<crate::backend::ThreadSummaryEntry>)>>,
    connection_state: BackendConnectionState,
    block_starts: bool,
    block_sends: bool,
    ingest_tx: broadcast::Sender<LocalIngestFrame>,
    manager_tx: broadcast::Sender<ManagerEvent>,
}

impl TestBackend {
    fn new() -> Self {
        Self::with_agents(Vec::new())
    }

    fn with_agents(detected_agents: Vec<AgentDescriptor>) -> Self {
        let (ingest_tx, _) = broadcast::channel(8);
        let (manager_tx, _) = broadcast::channel(8);
        Self {
            detected_agents,
            started: Mutex::new(Vec::new()),
            sent_messages: Mutex::new(Vec::new()),
            approval_decisions: Mutex::new(Vec::new()),
            opencode_permission_responses: Mutex::new(Vec::new()),
            opencode_question_responses: Mutex::new(Vec::new()),
            group_chat_pages: Mutex::new(VecDeque::new()),
            next_thread: Mutex::new(0),
            interrupted: Mutex::new(Vec::new()),
            closed: Mutex::new(Vec::new()),
            deleted: Mutex::new(Vec::new()),
            listed_threads: Mutex::new(Vec::new()),
            history_pages: Mutex::new(HashMap::new()),
            history_calls: Mutex::new(Vec::new()),
            projects: Mutex::new(Vec::new()),
            created_projects: Mutex::new(Vec::new()),
            project_thread_lists: Mutex::new(Vec::new()),
            connection_state: BackendConnectionState::Embedded,
            block_starts: false,
            block_sends: false,
            ingest_tx,
            manager_tx,
        }
    }

    fn with_connection_state(mut self, connection_state: BackendConnectionState) -> Self {
        self.connection_state = connection_state;
        self
    }

    fn with_blocked_starts(mut self) -> Self {
        self.block_starts = true;
        self
    }

    fn with_blocked_sends(mut self) -> Self {
        self.block_sends = true;
        self
    }

    fn with_listed_threads(self, listed_threads: Vec<BackendThreadSnapshot>) -> Self {
        *self.listed_threads.lock().expect("listed threads lock") = listed_threads;
        self
    }

    fn with_history_pages(self, thread_id: &str, pages: Vec<ReadThreadRawHistoryResponse>) -> Self {
        self.history_pages
            .lock()
            .expect("history pages lock")
            .insert(thread_id.to_owned(), VecDeque::from(pages));
        self
    }

    fn with_group_chat_pages(self, pages: Vec<Vec<LocalGroupChatMessage>>) -> Self {
        *self.group_chat_pages.lock().expect("group chat pages lock") = VecDeque::from(pages);
        self
    }

    fn with_projects(self, projects: Vec<crate::backend::ProjectEntry>) -> Self {
        *self.projects.lock().expect("projects lock") = projects;
        self
    }
}

#[async_trait]
impl AgentBackend for TestBackend {
    async fn detect_clis(&self) -> Result<Vec<AgentDescriptor>> {
        Ok(self.detected_agents.clone())
    }

    async fn start_agent(&self, agent: AgentName, workspace: PathBuf) -> Result<StartAgentOutcome> {
        if self.block_starts {
            std::future::pending::<()>().await;
        }
        self.started.lock().expect("started list lock").push(agent);
        let mut next_thread = self.next_thread.lock().expect("next_thread lock");
        *next_thread += 1;
        Ok(StartAgentOutcome {
            thread_id: format!("thread-{}", *next_thread),
            cwd: workspace,
            provider_session_id: None,
        })
    }

    async fn send_message(&self, thread_id: &str, text: &str) -> Result<()> {
        if self.block_sends {
            std::future::pending::<()>().await;
        }
        self.sent_messages
            .lock()
            .expect("sent messages lock")
            .push((thread_id.to_owned(), text.to_owned()));
        Ok(())
    }

    async fn send_approval_decision(
        &self,
        request_id: &str,
        thread_id: &str,
        decision: serde_json::Value,
    ) -> Result<()> {
        self.approval_decisions
            .lock()
            .expect("approval decisions lock")
            .push((request_id.to_owned(), thread_id.to_owned(), decision));
        Ok(())
    }

    async fn respond_opencode_permission(
        &self,
        thread_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<()> {
        self.opencode_permission_responses
            .lock()
            .expect("opencode permission responses lock")
            .push((
                thread_id.to_owned(),
                permission_id.to_owned(),
                response.to_owned(),
            ));
        Ok(())
    }

    async fn respond_opencode_question(
        &self,
        thread_id: &str,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> Result<()> {
        self.opencode_question_responses
            .lock()
            .expect("opencode question responses lock")
            .push((thread_id.to_owned(), question_id.to_owned(), answers));
        Ok(())
    }

    async fn interrupt_thread(&self, thread_id: &str) -> Result<()> {
        self.interrupted
            .lock()
            .expect("interrupt list lock")
            .push(thread_id.to_owned());
        Ok(())
    }

    async fn close_thread(&self, thread_id: &str) -> Result<()> {
        self.closed
            .lock()
            .expect("close list lock")
            .push(thread_id.to_owned());
        Ok(())
    }

    async fn delete_thread(&self, thread_id: &str) -> Result<()> {
        self.deleted
            .lock()
            .expect("delete list lock")
            .push(thread_id.to_owned());
        Ok(())
    }

    async fn list_threads(&self) -> Result<Vec<BackendThreadSnapshot>> {
        Ok(self
            .listed_threads
            .lock()
            .expect("listed threads lock")
            .clone())
    }

    async fn list_projects(&self) -> Result<Vec<crate::backend::ProjectEntry>> {
        Ok(self.projects.lock().expect("projects lock").clone())
    }

    async fn create_project(
        &self,
        name: &str,
        workspace_path: &std::path::Path,
    ) -> Result<crate::backend::ProjectEntry> {
        self.created_projects
            .lock()
            .expect("created projects lock")
            .push((name.to_owned(), workspace_path.to_path_buf()));
        let entry = crate::backend::ProjectEntry {
            project_id: format!("test-project-{}", name),
            name: name.to_owned(),
            workspace_path: workspace_path.to_path_buf(),
            thread_count: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        self.projects
            .lock()
            .expect("projects lock")
            .push(entry.clone());
        Ok(entry)
    }

    async fn list_project_threads(
        &self,
        project_id: &str,
    ) -> Result<Vec<crate::backend::ThreadSummaryEntry>> {
        let lists = self
            .project_thread_lists
            .lock()
            .expect("project threads lock");
        Ok(lists
            .iter()
            .find(|(pid, _)| pid == project_id)
            .map(|(_, threads)| threads.clone())
            .unwrap_or_default())
    }

    async fn start_agent_in_project(
        &self,
        _project_id: &str,
        agent: AgentName,
        workspace: PathBuf,
    ) -> Result<StartAgentOutcome> {
        self.start_agent(agent, workspace).await
    }

    async fn resume_thread(&self, _thread_id: &str) -> Result<StartAgentOutcome> {
        Ok(StartAgentOutcome {
            thread_id: String::new(),
            cwd: PathBuf::new(),
            provider_session_id: None,
        })
    }

    async fn read_thread_raw_history(
        &self,
        thread_id: &str,
        from_seq: Option<u64>,
        limit: u32,
    ) -> Result<ReadThreadRawHistoryResponse> {
        self.history_calls
            .lock()
            .expect("history calls lock")
            .push((thread_id.to_owned(), from_seq, limit));
        let mut pages = self.history_pages.lock().expect("history pages lock");
        Ok(pages
            .get_mut(thread_id)
            .and_then(VecDeque::pop_front)
            .unwrap_or(ReadThreadRawHistoryResponse {
                events: Vec::new(),
                next_seq: None,
            }))
    }

    async fn read_group_chat(
        &self,
        _room_id: &str,
        _after_seq: Option<u64>,
        _before_seq: Option<u64>,
        _limit: u32,
    ) -> Result<Vec<LocalGroupChatMessage>> {
        Ok(self
            .group_chat_pages
            .lock()
            .expect("group chat pages lock")
            .pop_front()
            .unwrap_or_default())
    }

    async fn subscribe_ingest(&self) -> broadcast::Receiver<LocalIngestFrame> {
        self.ingest_tx.subscribe()
    }

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager_tx.subscribe()
    }

    fn connection_state(&self) -> BackendConnectionState {
        self.connection_state.clone()
    }
}

fn ok_agent(agent: AgentName) -> AgentDescriptor {
    AgentDescriptor {
        name: agent,
        path: Some(format!("/usr/local/bin/{}", agent.bin_name())),
        version: Some("1.0.0".into()),
        status: AgentStatus::Ok,
    }
}

fn press(code: KeyCode) -> KeyEvent {
    press_with_modifiers(code, KeyModifiers::NONE)
}

fn press_with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn scroll(kind: MouseEventKind) -> MouseEvent {
    MouseEvent {
        kind,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    }
}

fn projected_frame(
    thread_id: &str,
    seq: u64,
    agent: AgentName,
    ui_events: Vec<UiEventMessage>,
) -> LocalIngestFrame {
    LocalIngestFrame {
        thread_id: thread_id.to_string(),
        seq,
        agent,
        ui_events,
        ts_ms: i64::try_from(seq).unwrap_or(0),
    }
}

#[path = "app_tests/group_and_agent.rs"]
mod group_and_agent;
#[path = "app_tests/ingest.rs"]
mod ingest;
#[path = "app_tests/input_and_routing.rs"]
mod input_and_routing;
#[path = "app_tests/navigation_and_lifecycle.rs"]
mod navigation_and_lifecycle;
#[path = "app_tests/nav_integration.rs"]
mod nav_integration;
