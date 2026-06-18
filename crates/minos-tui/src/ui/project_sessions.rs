use crate::backend::ThreadSummaryEntry;
use crate::render::Renderable;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame,
};

pub struct SessionListRenderable<'a> {
    project_name: &'a str,
    sessions: &'a [ThreadSummaryEntry],
    selected: Option<usize>,
    list_state: &'a mut ListState,
    focused: bool,
}

impl<'a> SessionListRenderable<'a> {
    pub fn new(
        project_name: &'a str,
        sessions: &'a [ThreadSummaryEntry],
        selected: Option<usize>,
        list_state: &'a mut ListState,
        focused: bool,
    ) -> Self {
        Self {
            project_name,
            sessions,
            selected,
            list_state,
            focused,
        }
    }
}

impl Renderable for SessionListRenderable<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focused {
            theme::FOCUSED_BORDER
        } else {
            Style::new().fg(theme::BORDER_FG)
        };
        let title = format!("Sessions — {}", self.project_name);
        let block = Block::bordered().title(title).border_style(border_style);
        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, s)| {
                let id_short = &s.thread_id[..8.min(s.thread_id.len())];
                let title = s.title.clone().unwrap_or_else(|| "(untitled)".to_owned());
                let prefix = if self.selected == Some(index) {
                    "> "
                } else {
                    "  "
                };
                ListItem::new(Line::from(vec![
                    Span::styled(prefix, Style::new().fg(Color::Cyan)),
                    Span::styled(format!("#{} ", id_short), Style::new().fg(Color::DarkGray)),
                    Span::raw(title),
                    Span::raw(format!("  [{}]", s.agent.bin_name())),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::HIGHLIGHTED);
        frame.render_stateful_widget(list, area, self.list_state);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

pub struct SessionSidebarRenderable<'a> {
    sessions: &'a [ThreadSummaryEntry],
    selected: Option<usize>,
}

impl<'a> SessionSidebarRenderable<'a> {
    pub fn new(sessions: &'a [ThreadSummaryEntry], selected: Option<usize>) -> Self {
        Self { sessions, selected }
    }
}

impl Renderable for SessionSidebarRenderable<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .title("Session Info")
            .border_style(Style::new().fg(theme::BORDER_FG));
        let content = match self.selected.and_then(|i| self.sessions.get(i)) {
            Some(session) => {
                let title = session.title.clone().unwrap_or_else(|| "(untitled)".to_owned());
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::styled("Title: ", Style::new().fg(theme::BORDER_FG)),
                        Span::raw(title),
                    ]),
                    Line::from(vec![
                        Span::styled("Agent: ", Style::new().fg(theme::BORDER_FG)),
                        Span::raw(session.agent.bin_name()),
                    ]),
                    Line::from(vec![
                        Span::styled("Messages: ", Style::new().fg(theme::BORDER_FG)),
                        Span::raw(session.message_count.to_string()),
                    ]),
                ])
                .block(block)
            }
            None => Paragraph::new("Type a message below to start").block(block),
        };
        frame.render_widget(content, area);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}
