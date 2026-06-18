use crate::backend::{ConversationMessageEntry, ThreadSummaryEntry};
use crate::render::Renderable;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame,
};

pub struct ConversationMessagesRenderable<'a> {
    title: String,
    messages: &'a [ConversationMessageEntry],
    scroll_offset: &'a mut u16,
    auto_scroll: &'a mut bool,
    max_scroll: &'a mut u16,
    focused: bool,
}

impl<'a> ConversationMessagesRenderable<'a> {
    pub fn new(
        title: String,
        messages: &'a [ConversationMessageEntry],
        scroll_offset: &'a mut u16,
        auto_scroll: &'a mut bool,
        max_scroll: &'a mut u16,
        focused: bool,
    ) -> Self {
        Self {
            title,
            messages,
            scroll_offset,
            auto_scroll,
            max_scroll,
            focused,
        }
    }
}

impl Renderable for ConversationMessagesRenderable<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focused {
            theme::FOCUSED_BORDER
        } else {
            Style::new().fg(theme::BORDER_FG)
        };
        let block = Block::bordered()
            .title(self.title.clone())
            .border_style(border_style);
        if self.messages.is_empty() {
            *self.max_scroll = 0;
            *self.scroll_offset = 0;
            *self.auto_scroll = true;
            frame.render_widget(Paragraph::new("").block(block), area);
            return;
        }
        let lines = self
            .messages
            .iter()
            .flat_map(|message| {
                let sender = match (message.sender_role.as_str(), message.agent) {
                    ("agent", Some(agent)) => agent.bin_name().to_owned(),
                    ("agent", None) => "agent".to_owned(),
                    _ => "you".to_owned(),
                };
                let style = if message.sender_role == "agent" {
                    Style::new().fg(Color::Cyan)
                } else {
                    Style::new().fg(Color::Green)
                };
                [
                    Line::from(vec![
                        Span::styled(format!("{sender}: "), style),
                        Span::raw(message.body.clone()),
                    ]),
                    Line::raw(""),
                ]
            })
            .collect::<Vec<_>>();
        let inner_height = area.height.saturating_sub(2).max(1);
        *self.max_scroll = u16::try_from(lines.len().saturating_sub(usize::from(inner_height)))
            .unwrap_or(u16::MAX);
        if *self.auto_scroll {
            *self.scroll_offset = *self.max_scroll;
        } else {
            *self.scroll_offset = (*self.scroll_offset).min(*self.max_scroll);
        }
        frame.render_widget(
            Paragraph::new(lines)
                .block(block)
                .scroll((*self.scroll_offset, 0)),
            area,
        );
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

pub struct AgentSessionListRenderable<'a> {
    sessions: &'a [ThreadSummaryEntry],
    selected: Option<usize>,
    list_state: &'a mut ListState,
    focused: bool,
}

impl<'a> AgentSessionListRenderable<'a> {
    pub fn new(
        sessions: &'a [ThreadSummaryEntry],
        selected: Option<usize>,
        list_state: &'a mut ListState,
        focused: bool,
    ) -> Self {
        Self {
            sessions,
            selected,
            list_state,
            focused,
        }
    }
}

impl Renderable for AgentSessionListRenderable<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focused {
            theme::FOCUSED_BORDER
        } else {
            Style::new().fg(theme::BORDER_FG)
        };
        let block = Block::bordered()
            .title("Agent Sessions")
            .border_style(border_style);
        let items = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, session)| {
                let id_short = &session.thread_id[..8.min(session.thread_id.len())];
                let prefix = if self.selected == Some(index) {
                    "> "
                } else {
                    "  "
                };
                ListItem::new(Line::from(vec![
                    Span::styled(prefix, Style::new().fg(Color::Cyan)),
                    Span::raw(session.agent.bin_name()),
                    Span::styled(format!(" #{}", id_short), Style::new().fg(Color::DarkGray)),
                ]))
            })
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::HIGHLIGHTED);
        frame.render_stateful_widget(list, area, self.list_state);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}
