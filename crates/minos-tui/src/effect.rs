//! Side-effect descriptions returned by update handlers.

use std::path::PathBuf;

use minos_agent_runtime::ManagerEvent;
use minos_domain::AgentName;
use minos_protocol::LocalIngestFrame;

use crate::event::McpToolEvent;
use crate::translation::PendingAgentRequestKind;

pub enum Effect {
    Quit,
    InterruptOrQuit,
    CloseCurrentThread,
    StartAgentAt(usize),
    HandleIngest(LocalIngestFrame),
    HandleManagerEvent(ManagerEvent),
    HandleTick,
    HandleMcpToolCall(McpToolEvent),
    AgentStartedForPrompt {
        agent: AgentName,
        thread_id: String,
        cwd: PathBuf,
        text: String,
    },
    DispatchPromptToExistingAgent {
        agent: AgentName,
        thread_short_id: String,
        text: String,
        group_text: String,
    },
    InviteAgentToRoom {
        agent: AgentName,
        group_text: String,
    },
    DispatchPromptToAgent {
        agent: AgentName,
        text: String,
        group_text: String,
    },
    SendTextToThread {
        thread_id: String,
        text: String,
        group_text: Option<String>,
    },
    SubmitPendingAgentRequest {
        thread_id: String,
        pending: PendingAgentRequestKind,
        text: String,
    },
    ConfirmDeleteThread,
    CopyToClipboard(String),
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
