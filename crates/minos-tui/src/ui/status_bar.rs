use minos_domain::{AgentDescriptor, AgentName, AgentStatus};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use super::theme::{CLI_MISSING, CLI_OK};

pub struct StatusBarState {
    pub agents: Vec<AgentDescriptor>,
}

impl StatusBarState {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
        }
    }

    pub fn update_agents(&mut self, agents: Vec<AgentDescriptor>) {
        self.agents = agents;
    }

    pub fn installed_agents(&self) -> Vec<AgentName> {
        self.agents
            .iter()
            .filter(|a| matches!(a.status, AgentStatus::Ok))
            .map(|a| a.name)
            .collect()
    }
}

pub fn render_status_bar(f: &mut Frame, area: Rect, state: &StatusBarState) {
    let mut spans: Vec<Span> = Vec::new();
    for desc in &state.agents {
        let (icon, style) = match desc.status {
            AgentStatus::Ok => ("✓", CLI_OK),
            AgentStatus::Missing => ("✗", CLI_MISSING),
            AgentStatus::Error { .. } => ("✗", CLI_MISSING),
        };
        let label = format!(" {} {} ", desc.name.bin_name(), icon);
        spans.push(Span::styled(label, style));
    }
    let paragraph = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}
