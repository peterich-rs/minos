use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use minos_agent_runtime::ManagerEvent;

use crate::backend::AgentBackend;
use crate::event::AppEvent;
use crate::translation::ChatState;
use crate::ui::{Focus, ThreadEntry, UiState};

pub struct App<B: AgentBackend> {
    backend: Arc<B>,
    ui: UiState,
    should_quit: bool,
}

impl<B: AgentBackend> App<B> {
    pub fn new(backend: Arc<B>, readonly: bool) -> Self {
        Self {
            backend,
            ui: UiState::new(readonly),
            should_quit: false,
        }
    }

    pub async fn init(&mut self) -> anyhow::Result<()> {
        let agents = self.backend.detect_clis().await?;
        self.ui.status.update_agents(agents);
        Ok(())
    }

    pub async fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Ingest(ingest) => {
                if let Some(chat) = self.ui.chat_states.get_mut(&ingest.thread_id) {
                    let events = chat.translation_state.translate(&ingest.payload);
                    chat.apply_ui_events(events);
                }
            }
            AppEvent::ManagerEvent(event) => self.handle_manager_event(event).await,
            AppEvent::Key(key) => self.handle_key(key).await,
            AppEvent::Tick => {
                if let Some((_, instant)) = self.ui.error_flash {
                    if instant.elapsed() > Duration::from_secs(3) {
                        self.ui.error_flash = None;
                    }
                }
            }
            AppEvent::Resize(_, _) => {}
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub async fn shutdown(&self) {
        let _ = self.backend.close_thread("").await;
    }

    pub fn ui(&mut self) -> &mut UiState {
        &mut self.ui
    }

    async fn handle_manager_event(&mut self, event: ManagerEvent) {
        match event {
            ManagerEvent::ThreadAdded {
                thread_id,
                workspace,
                agent,
            } => {
                let entry = ThreadEntry {
                    thread_id: thread_id.clone(),
                    agent,
                    workspace,
                    state: minos_agent_runtime::ThreadState::Starting,
                };
                self.ui.threads.push(entry);
                let chat = ChatState::new(thread_id.clone(), agent);
                self.ui.chat_states.insert(thread_id, chat);
                if self.ui.selected_thread.is_none() {
                    self.ui.selected_thread = Some(self.ui.threads.len() - 1);
                    self.ui.thread_list_state.select(Some(self.ui.threads.len() - 1));
                }
            }
            ManagerEvent::ThreadStateChanged {
                thread_id,
                new,
                ..
            } => {
                if let Some(entry) = self
                    .ui
                    .threads
                    .iter_mut()
                    .find(|t| t.thread_id == thread_id)
                {
                    entry.state = new;
                }
            }
            ManagerEvent::ThreadClosed { thread_id, reason: _ } => {
                if let Some(entry) = self
                    .ui
                    .threads
                    .iter_mut()
                    .find(|t| t.thread_id == thread_id)
                {
                    entry.state = minos_agent_runtime::ThreadState::Closed {
                        reason: minos_agent_runtime::CloseReason::UserClose,
                    };
                }
            }
            ManagerEvent::InstanceCrashed {
                affected_threads,
                ..
            } => {
                for tid in affected_threads {
                    if let Some(entry) = self
                        .ui
                        .threads
                        .iter_mut()
                        .find(|t| t.thread_id == tid)
                    {
                        entry.state = minos_agent_runtime::ThreadState::Suspended {
                            reason: minos_agent_runtime::PauseReason::InstanceReaped,
                        };
                    }
                }
            }
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('c') => {
                    if let Some(thread_id) = self.ui.current_thread_id().map(String::from) {
                        let _ = self.backend.interrupt_thread(&thread_id).await;
                    }
                    return;
                }
                KeyCode::Char('d') => {
                    if let Some(thread_id) = self.ui.current_thread_id().map(String::from) {
                        let _ = self.backend.close_thread(&thread_id).await;
                    }
                    return;
                }
                _ => {}
            }
        }

        match self.ui.focus {
            Focus::Input => self.handle_input_key(key).await,
            Focus::ThreadList => self.handle_thread_list_key(key).await,
            Focus::Chat => self.handle_chat_key(key).await,
        }
    }

    async fn handle_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.ui.input.insert_char('\n');
                } else {
                    let text = self.ui.input.take_input();
                    if !text.is_empty() {
                        if let Some(thread_id) = self.ui.current_thread_id().map(String::from) {
                            let _ = self.backend.send_message(&thread_id, &text).await;
                        }
                    }
                }
            }
            KeyCode::Char(c) => {
                self.ui.input.insert_char(c);
            }
            KeyCode::Backspace => {
                self.ui.input.backspace();
            }
            KeyCode::Tab => {
                self.ui.focus = Focus::ThreadList;
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    async fn handle_thread_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                if let Some(selected) = self.ui.selected_thread {
                    if selected > 0 {
                        self.ui.selected_thread = Some(selected - 1);
                        self.ui.thread_list_state.select(Some(selected - 1));
                    }
                }
            }
            KeyCode::Down => {
                if let Some(selected) = self.ui.selected_thread {
                    if selected + 1 < self.ui.threads.len() {
                        self.ui.selected_thread = Some(selected + 1);
                        self.ui.thread_list_state.select(Some(selected + 1));
                    }
                }
            }
            KeyCode::Enter => {
                self.ui.focus = Focus::Input;
            }
            KeyCode::Char('n') => {
                self.start_new_agent().await;
            }
            KeyCode::Tab => {
                self.ui.focus = Focus::Input;
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    async fn handle_chat_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::PageUp => {
                if let Some(chat) = self.ui.current_chat_mut() {
                    chat.scroll_offset = chat.scroll_offset.saturating_add(5);
                }
            }
            KeyCode::PageDown => {
                if let Some(chat) = self.ui.current_chat_mut() {
                    chat.scroll_offset = chat.scroll_offset.saturating_sub(5);
                }
            }
            KeyCode::Char('e') => {
                if let Some(chat) = self.ui.current_chat_mut() {
                    for msg in &mut chat.messages {
                        for tc in &mut msg.tool_calls {
                            tc.is_expanded = !tc.is_expanded;
                        }
                    }
                }
            }
            KeyCode::Tab => {
                self.ui.focus = Focus::Input;
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    async fn start_new_agent(&mut self) {
        let installed = self.ui.status.installed_agents();
        let agent = match installed.first() {
            Some(a) => *a,
            None => {
                self.ui.set_error("No installed agent found".into());
                return;
            }
        };
        let workspace = std::env::current_dir().unwrap_or_default();
        match self.backend.start_agent(agent, workspace).await {
            Ok(_) => {}
            Err(e) => {
                self.ui.set_error(format!("Failed to start agent: {e}"));
            }
        }
    }
}
