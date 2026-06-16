use crate::backend::BackendConnectionState;
use minos_domain::{AgentDescriptor, AgentStatus};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use super::theme::{CLI_MISSING, CLI_OK, DAEMON_CONNECTED, DAEMON_DISCONNECTED};

pub struct StatusBarState {
    pub agents: Vec<AgentDescriptor>,
    pub backend_state: BackendConnectionState,
}

impl StatusBarState {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            backend_state: BackendConnectionState::Embedded,
        }
    }

    pub fn update_agents(&mut self, agents: Vec<AgentDescriptor>) {
        self.agents = agents;
    }

    pub fn update_backend_state(&mut self, state: BackendConnectionState) {
        self.backend_state = state;
    }
}

pub fn render_status_bar(f: &mut Frame, area: Rect, state: &StatusBarState, flash_active: bool) {
    let mut spans: Vec<Span> = Vec::new();

    match &state.backend_state {
        BackendConnectionState::Embedded => {
            spans.push(Span::styled(" backend:embedded ", DAEMON_CONNECTED));
        }
        BackendConnectionState::Connected { .. } => {
            spans.push(Span::styled(" daemon:connected ", DAEMON_CONNECTED));
        }
        BackendConnectionState::Disconnected { last_error, .. } => {
            let label = match last_error {
                Some(e) => format!(" daemon:disconnected ({}) ", e),
                None => " daemon:disconnected ".to_owned(),
            };
            spans.push(Span::styled(label, DAEMON_DISCONNECTED));
        }
    }

    for desc in &state.agents {
        let (icon, style) = match desc.status {
            AgentStatus::Ok => ("✓", CLI_OK),
            AgentStatus::Missing => ("✗", CLI_MISSING),
            AgentStatus::Error { .. } => ("✗", CLI_MISSING),
        };
        let label = format!(" {} {} ", desc.name.bin_name(), icon);
        spans.push(Span::styled(label, style));
    }
    if flash_active {
        spans.push(Span::styled(
            "  ✓ Copied",
            ratatui::style::Style::new().fg(ratatui::style::Color::Green),
        ));
    }
    spans.push(Span::raw(
        "  n new-agent  @agent route  Tab focus  Enter inspect/send  Esc back/close-detail  wheel/PgUp/PgDn scroll  Ctrl+C interrupt  Ctrl+Q quit",
    ));
    let paragraph = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}
