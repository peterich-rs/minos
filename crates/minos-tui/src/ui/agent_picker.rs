use minos_domain::{AgentDescriptor, AgentStatus};
use ratatui::{
    layout::{Constraint, Direction, Flex, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::{theme, AgentPickerState};

pub fn render_agent_picker(f: &mut Frame, agents: &[AgentDescriptor], state: &AgentPickerState) {
    let area = centered_rect(f.area(), 60, 16);
    f.render_widget(Clear, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(area);

    let items: Vec<ListItem> = agents
        .iter()
        .enumerate()
        .map(|(index, desc)| {
            let (status_label, style) = match &desc.status {
                AgentStatus::Ok => ("installed".to_owned(), theme::CLI_OK),
                AgentStatus::Missing => ("missing".to_owned(), theme::CLI_MISSING),
                AgentStatus::Error { reason } => (format!("error: {reason}"), theme::CLI_MISSING),
            };
            let version = desc.version.as_deref().unwrap_or("unknown");
            let line = Line::from(vec![
                Span::raw(format!("{} ", index + 1)),
                Span::styled(format!("{:<8}", desc.name.bin_name()), style),
                Span::raw(format!(" {status_label:<20} v{version}")),
            ]);
            ListItem::new(line)
        })
        .collect();

    let mut list_state = ListState::default();
    if !agents.is_empty() {
        list_state.select(Some(state.selected.min(agents.len().saturating_sub(1))));
    }

    let list = List::new(items)
        .block(
            Block::bordered()
                .title("New Thread")
                .border_style(ratatui::style::Style::new().fg(theme::BORDER_FG)),
        )
        .highlight_symbol("› ")
        .highlight_style(theme::HIGHLIGHTED);
    f.render_stateful_widget(list, sections[0], &mut list_state);

    let help = Paragraph::new("↑/↓ move  Enter start  Esc cancel  1-9 quick select")
        .block(theme::border_block());
    f.render_widget(help, sections[1]);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let [vertical] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);

    let [horizontal] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(vertical);

    horizontal
}
