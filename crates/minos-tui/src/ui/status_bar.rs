use crate::backend::BackendConnectionState;
use crate::render::Renderable;
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
            backend_state: BackendConnectionState::Disconnected {
                endpoint: String::new(),
                last_error: None,
            },
        }
    }

    pub fn update_agents(&mut self, agents: Vec<AgentDescriptor>) {
        self.agents = agents;
    }

    pub fn update_backend_state(&mut self, state: BackendConnectionState) {
        self.backend_state = state;
    }
}

pub struct StatusBarRenderable<'a> {
    state: &'a StatusBarState,
    flash_active: bool,
}

impl<'a> StatusBarRenderable<'a> {
    pub fn new(state: &'a StatusBarState, flash_active: bool) -> Self {
        Self {
            state,
            flash_active,
        }
    }
}

impl Renderable for StatusBarRenderable<'_> {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        render_status_bar(f, area, self.state, self.flash_active);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        1
    }
}

pub fn render_status_bar(f: &mut Frame, area: Rect, state: &StatusBarState, flash_active: bool) {
    let mut spans: Vec<Span> = Vec::new();

    match &state.backend_state {
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
        "  ^P projects  ^T conversations  @agent route  Tab focus  Enter inspect/send  Esc back/close-detail  wheel/PgUp/PgDn scroll  Ctrl+J newline  Alt+Enter multi  Ctrl+Alt+B cursor  Ctrl+C interrupt  Ctrl+Q quit",
    ));
    let paragraph = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}
