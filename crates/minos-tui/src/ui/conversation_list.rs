use crate::backend::ConversationEntry;
use crate::render::Renderable;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame,
};

pub struct ConversationListRenderable<'a> {
    project_name: &'a str,
    conversations: &'a [ConversationEntry],
    selected: Option<usize>,
    list_state: &'a mut ListState,
    focused: bool,
}

impl<'a> ConversationListRenderable<'a> {
    pub fn new(
        project_name: &'a str,
        conversations: &'a [ConversationEntry],
        selected: Option<usize>,
        list_state: &'a mut ListState,
        focused: bool,
    ) -> Self {
        Self {
            project_name,
            conversations,
            selected,
            list_state,
            focused,
        }
    }
}

impl Renderable for ConversationListRenderable<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focused {
            theme::FOCUSED_BORDER
        } else {
            Style::new().fg(theme::BORDER_FG)
        };
        let title = format!("Conversations - {}", self.project_name);
        let block = Block::bordered().title(title).border_style(border_style);
        let items: Vec<ListItem> = self
            .conversations
            .iter()
            .enumerate()
            .map(|(index, conversation)| {
                let id_short =
                    &conversation.conversation_id[..8.min(conversation.conversation_id.len())];
                let prefix = if self.selected == Some(index) {
                    "> "
                } else {
                    "  "
                };
                let agents = if conversation.participating_agents.is_empty() {
                    String::new()
                } else {
                    let joined = conversation
                        .participating_agents
                        .iter()
                        .map(|agent| agent.bin_name())
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("  [{}]", joined)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(prefix, Style::new().fg(Color::Cyan)),
                    Span::styled(format!("#{} ", id_short), Style::new().fg(Color::DarkGray)),
                    Span::raw(conversation.title.clone()),
                    Span::raw(agents),
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

pub struct ConversationSidebarRenderable<'a> {
    conversations: &'a [ConversationEntry],
    selected: Option<usize>,
}

impl<'a> ConversationSidebarRenderable<'a> {
    pub fn new(conversations: &'a [ConversationEntry], selected: Option<usize>) -> Self {
        Self {
            conversations,
            selected,
        }
    }
}

impl Renderable for ConversationSidebarRenderable<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .title("Conversation")
            .border_style(Style::new().fg(theme::BORDER_FG));
        let content = match self.selected.and_then(|i| self.conversations.get(i)) {
            Some(conversation) => Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Title: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(conversation.title.clone()),
                ]),
                Line::from(vec![
                    Span::styled("Messages: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(conversation.message_count.to_string()),
                ]),
                Line::from(vec![
                    Span::styled("Sessions: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(conversation.agent_session_count.to_string()),
                ]),
            ])
            .block(block),
            None => Paragraph::new("Type below to create a conversation").block(block),
        };
        frame.render_widget(content, area);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}
