use crossterm::event::{KeyEvent, MouseEvent};
use minos_agent_runtime::ManagerEvent;
use minos_domain::AgentName;
use minos_protocol::LocalIngestFrame;
use std::path::PathBuf;

use crate::action::InputTarget;
use crate::backend::BackendSessionSnapshot;
use crate::path_complete::PathCandidate;

pub enum AppEvent {
    Ingest(LocalIngestFrame),
    ManagerEvent(ManagerEvent),
    /// Background tick listed daemon sessions; apply on the main loop without
    /// blocking input/draw on the network round-trip.
    DaemonThreadsListed {
        sessions: Vec<BackendSessionSnapshot>,
    },
    AgentStartedForPrompt {
        agent: AgentName,
        session_id: String,
        cwd: PathBuf,
        text: String,
    },
    SendMessageFailed {
        session_id: String,
        error: String,
    },
    ProjectCreated(crate::backend::ProjectEntry),
    PathCandidatesResolved {
        target: InputTarget,
        sequence: u64,
        candidates: Vec<PathCandidate>,
    },
    ConversationsLoaded {
        project_id: String,
        conversations: Vec<crate::backend::ConversationEntry>,
    },
    ConversationOpened {
        project_id: String,
        conversation_id: String,
        messages: Vec<crate::backend::ConversationMessageEntry>,
        sessions: Vec<crate::backend::SessionSummaryEntry>,
    },
    ConversationAgentStarted {
        conversation_id: String,
        agent: AgentName,
        session_id: String,
        cwd: PathBuf,
        text: String,
    },
    ConversationMessageAppended {
        conversation_id: String,
        message_seq: i64,
    },
    ProjectFailed(String),
    Key(KeyEvent),
    Paste(String),
    Mouse(MouseEvent),
    #[allow(dead_code)]
    Resize(u16, u16),
    Tick,
}

pub fn spawn_ingest_pump(
    mut rx: tokio::sync::broadcast::Receiver<LocalIngestFrame>,
    tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ingest) => {
                    if tx.send(AppEvent::Ingest(ingest)).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

pub fn spawn_manager_event_pump(
    mut rx: tokio::sync::broadcast::Receiver<ManagerEvent>,
    tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if tx.send(AppEvent::ManagerEvent(event)).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

pub fn spawn_conversation_message_event_pump(
    mut rx: tokio::sync::broadcast::Receiver<crate::backend::ConversationMessageEvent>,
    tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if tx
                        .send(AppEvent::ConversationMessageAppended {
                            conversation_id: event.conversation_id,
                            message_seq: event.message_seq,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

pub fn spawn_terminal_pump(tx: tokio::sync::mpsc::UnboundedSender<AppEvent>) {
    std::thread::Builder::new()
        .name("minos-tui-terminal".into())
        .spawn(move || {
            let poll_timeout = std::time::Duration::from_millis(250);

            loop {
                match crossterm::event::poll(poll_timeout) {
                    Ok(true) => match crossterm::event::read() {
                        Ok(crossterm::event::Event::Key(key)) => {
                            if tx.send(AppEvent::Key(key)).is_err() {
                                break;
                            }
                        }
                        Ok(crossterm::event::Event::Paste(text)) => {
                            if tx.send(AppEvent::Paste(text)).is_err() {
                                break;
                            }
                        }
                        Ok(crossterm::event::Event::Mouse(mouse)) => {
                            if tx.send(AppEvent::Mouse(mouse)).is_err() {
                                break;
                            }
                        }
                        Ok(crossterm::event::Event::Resize(w, h)) => {
                            if tx.send(AppEvent::Resize(w, h)).is_err() {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    },
                    Ok(false) => continue,
                    Err(_) => break,
                }
            }
        })
        .expect("terminal event pump thread should spawn");
}

pub fn spawn_tick_pump(tx: tokio::sync::mpsc::UnboundedSender<AppEvent>, interval_ms: u64) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        loop {
            interval.tick().await;
            if tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });
}
