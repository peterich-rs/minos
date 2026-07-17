//! Side-effect descriptions returned by update handlers.

use std::path::PathBuf;

use minos_domain::AgentName;

use crate::action::InputTarget;
use crate::translation::PendingAgentRequestKind;

pub enum Effect {
    Quit,
    InterruptOrQuit,
    CloseCurrentThread,
    HandleTick,
    AgentStartedForPrompt {
        agent: AgentName,
        thread_id: String,
        cwd: PathBuf,
        text: String,
    },
    DispatchPromptToAgent {
        agent: AgentName,
        text: String,
        message_body: String,
    },
    SendTextToThread {
        thread_id: String,
        text: String,
        message_body: Option<String>,
    },
    SubmitPendingAgentRequest {
        thread_id: String,
        pending: PendingAgentRequestKind,
        text: String,
    },
    ConfirmDeleteThread,
    CopyToClipboard(String),
    ResolvePathCandidates {
        target: InputTarget,
        sequence: u64,
        token: String,
        workspace_root: PathBuf,
    },
    CreateProject {
        name: String,
        workspace_path: PathBuf,
    },
    LoadConversations {
        project_id: String,
    },
    CreateConversationAndStartAgent {
        project_id: String,
        agent: AgentName,
        workspace: PathBuf,
        message_body: String,
        prompt: String,
    },
    StartAgentInConversation {
        project_id: String,
        conversation_id: String,
        agent: AgentName,
        workspace: PathBuf,
        message_body: String,
        prompt: String,
    },
    OpenConversation {
        conversation_id: String,
    },
    OpenAgentSession {
        thread_id: String,
    },
}

#[derive(Debug, Default)]
pub struct StateChange {
    pub needs_redraw: bool,
}

impl StateChange {
    pub fn redraw() -> Self {
        Self { needs_redraw: true }
    }

    pub fn none() -> Self {
        Self {
            needs_redraw: false,
        }
    }
}
