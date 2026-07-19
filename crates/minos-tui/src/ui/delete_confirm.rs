use std::path::PathBuf;

use crate::render::Renderable;
use minos_domain::AgentName;
use ratatui::{
    layout::{Constraint, Direction, Flex, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Wrap},
    Frame,
};

use super::theme;

pub struct DeleteConfirmState {
    pub thread_id: String,
    pub agent: AgentName,
    pub workspace: PathBuf,
    pub selected_index: usize,
}

pub struct DeleteConfirmRenderable<'a> {
    state: &'a DeleteConfirmState,
}

impl<'a> DeleteConfirmRenderable<'a> {
    pub fn new(state: &'a DeleteConfirmState) -> Self {
        Self { state }
    }
}

impl Renderable for DeleteConfirmRenderable<'_> {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        render_delete_confirm_in_area(f, area, self.state);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        8
    }
}

fn render_delete_confirm_in_area(f: &mut Frame, root: Rect, state: &DeleteConfirmState) {
    let area = centered_rect(root, 64, 8);
    f.render_widget(Clear, area);

    let tid_short = &state.thread_id[..8.min(state.thread_id.len())];
    let workspace = state
        .workspace
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let lines = vec![
        Line::from("Delete this local thread?"),
        Line::from(""),
        Line::from(vec![
            Span::raw("Thread "),
            Span::styled(tid_short.to_owned(), theme::HIGHLIGHTED),
            Span::raw(format!(
                "  Agent {}  Workspace {}",
                state.agent.bin_name(),
                workspace
            )),
        ]),
        Line::from(""),
        Line::from("Enter/Y confirm    Esc/N cancel"),
    ];
    let paragraph = Paragraph::new(lines)
        .block(
            Block::bordered()
                .title("Confirm Delete")
                .border_style(theme::FOCUSED_BORDER),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let [vertical] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);

    let [horizontal] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(vertical);

    horizontal
}
