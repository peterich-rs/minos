use crate::ui::theme;
use crate::ui::ProjectCreateDialogState;
use ratatui::{
    layout::{Constraint, Direction, Flex, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(height), Constraint::Fill(1)])
        .flex(Flex::Center)
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Length(width), Constraint::Fill(1)])
        .flex(Flex::Center)
        .split(popup[1])[1]
}

pub fn render(f: &mut Frame, area: Rect, state: &ProjectCreateDialogState) {
    let dialog_area = centered_rect(50, 8, area);
    f.render_widget(Clear, dialog_area);

    let name_cursor = if state.editing_name { "█" } else { "" };
    let path_cursor = if !state.editing_name { "█" } else { "" };

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("Name: ", Style::new().fg(theme::BORDER_FG)),
            Span::raw(&state.name),
            Span::raw(name_cursor),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Path: ", Style::new().fg(theme::BORDER_FG)),
            Span::raw(&state.path),
            Span::raw(path_cursor),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("[Tab] ", theme::FOCUSED_BORDER),
            Span::raw("switch  "),
            Span::styled("[Enter] ", theme::FOCUSED_BORDER),
            Span::raw("create  "),
            Span::styled("[Esc] ", theme::FOCUSED_BORDER),
            Span::raw("cancel"),
        ]),
    ];

    let block = Block::bordered()
        .title("New Project")
        .border_style(theme::FOCUSED_BORDER);
    f.render_widget(Paragraph::new(lines).block(block), dialog_area);
}
