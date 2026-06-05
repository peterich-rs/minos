use std::ops::Range;

use minos_domain::{AgentDescriptor, AgentStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

use super::theme::{
    border_block, BORDER_FG, FOCUSED_BORDER, HIGHLIGHTED, INPUT_PROMPT, REASONING_STYLE,
};

pub struct InputAgentPickerState {
    pub candidate_indices: Vec<usize>,
    pub selected: usize,
    pub replace_range: Range<usize>,
}

pub struct InputState {
    pub content: String,
    pub cursor_pos: usize,
    pub focused: bool,
    pub readonly: bool,
    pub agent_picker: Option<InputAgentPickerState>,
}

impl InputState {
    pub fn new(readonly: bool) -> Self {
        Self {
            content: String::new(),
            cursor_pos: 0,
            focused: true,
            readonly,
            agent_picker: None,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if self.readonly {
            return;
        }
        self.content.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.readonly || self.cursor_pos == 0 {
            return;
        }
        let prev = self.content[..self.cursor_pos]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.content.drain(prev..self.cursor_pos);
        self.cursor_pos = prev;
    }

    pub fn take_input(&mut self) -> String {
        let taken = std::mem::take(&mut self.content);
        self.cursor_pos = 0;
        self.agent_picker = None;
        taken
    }

    pub fn clear_agent_picker(&mut self) {
        self.agent_picker = None;
    }

    pub fn sync_agent_picker(&mut self, agents: &[AgentDescriptor], enabled: bool) {
        if !enabled || self.readonly {
            self.agent_picker = None;
            return;
        }

        let Some(replace_range) = active_agent_range(&self.content, self.cursor_pos) else {
            self.agent_picker = None;
            return;
        };
        let query = self.content[replace_range.start + 1..replace_range.end].to_ascii_lowercase();

        let previous_agent = self
            .agent_picker
            .as_ref()
            .and_then(|picker| picker.candidate_indices.get(picker.selected))
            .and_then(|index| agents.get(*index))
            .map(|desc| desc.name);

        let candidate_indices: Vec<usize> = agents
            .iter()
            .enumerate()
            .filter_map(|(index, desc)| {
                desc.name
                    .bin_name()
                    .starts_with(query.as_str())
                    .then_some(index)
            })
            .collect();

        if candidate_indices.is_empty() {
            self.agent_picker = None;
            return;
        }

        let selected = previous_agent
            .and_then(|name| {
                candidate_indices
                    .iter()
                    .position(|index| agents[*index].name == name)
            })
            .or_else(|| {
                candidate_indices
                    .iter()
                    .position(|index| agents[*index].name.bin_name() == query.as_str())
            })
            .unwrap_or(0);

        self.agent_picker = Some(InputAgentPickerState {
            candidate_indices,
            selected,
            replace_range,
        });
    }

    pub fn has_agent_picker(&self) -> bool {
        self.agent_picker
            .as_ref()
            .is_some_and(|picker| !picker.candidate_indices.is_empty())
    }

    pub fn select_previous_agent(&mut self) -> bool {
        let Some(picker) = self.agent_picker.as_mut() else {
            return false;
        };
        let len = picker.candidate_indices.len();
        if len == 0 {
            return false;
        }
        picker.selected = if picker.selected == 0 {
            len - 1
        } else {
            picker.selected - 1
        };
        true
    }

    pub fn select_next_agent(&mut self) -> bool {
        let Some(picker) = self.agent_picker.as_mut() else {
            return false;
        };
        let len = picker.candidate_indices.len();
        if len == 0 {
            return false;
        }
        picker.selected = (picker.selected + 1) % len;
        true
    }

    pub fn accept_agent_completion(&mut self, agents: &[AgentDescriptor]) -> bool {
        let Some(picker) = self.agent_picker.take() else {
            return false;
        };
        let Some(agent_index) = picker.candidate_indices.get(picker.selected).copied() else {
            return false;
        };
        let Some(agent) = agents.get(agent_index) else {
            return false;
        };

        let replacement = format!("@{} ", agent.name.bin_name());
        self.content
            .replace_range(picker.replace_range.clone(), replacement.as_str());
        self.cursor_pos = picker.replace_range.start + replacement.len();
        true
    }
}

pub fn required_height(state: &InputState) -> u16 {
    let picker_rows = state
        .agent_picker
        .as_ref()
        .map(|picker| picker.candidate_indices.len().min(4) as u16)
        .unwrap_or(0);
    3 + picker_rows
}

pub fn render_input_bar(f: &mut Frame, area: Rect, state: &InputState, agents: &[AgentDescriptor]) {
    let block = border_block()
        .title("Input")
        .border_style(if state.focused {
            FOCUSED_BORDER
        } else {
            ratatui::style::Style::new().fg(BORDER_FG)
        });
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    let picker_area = sections[0];
    let input_area = sections[1];

    if picker_area.height > 0 {
        render_inline_agent_picker(f, picker_area, state, agents);
    }

    let line = if state.readonly {
        Line::from(Span::styled(
            "[readonly mode]",
            ratatui::style::Style::new().fg(ratatui::style::Color::DarkGray),
        ))
    } else {
        let mut spans: Vec<Span> = Vec::new();
        if state.focused {
            spans.push(Span::styled("> ", INPUT_PROMPT));
        }
        if state.content.is_empty() {
            spans.push(Span::styled("Type @ to choose an agent", REASONING_STYLE));
        } else {
            spans.push(Span::raw(state.content.clone()));
        }
        if state.focused {
            spans.push(Span::raw("▎"));
        }
        Line::from(spans)
    };

    let paragraph = Paragraph::new(line);
    f.render_widget(paragraph, input_area);
}

fn render_inline_agent_picker(
    f: &mut Frame,
    area: Rect,
    state: &InputState,
    agents: &[AgentDescriptor],
) {
    let Some(picker) = state.agent_picker.as_ref() else {
        return;
    };
    if picker.candidate_indices.is_empty() || area.height == 0 {
        return;
    }

    let items: Vec<ListItem> = picker
        .candidate_indices
        .iter()
        .take(4)
        .filter_map(|index| agents.get(*index))
        .map(|desc| {
            let (status_label, status_style) = match &desc.status {
                AgentStatus::Ok => (
                    "installed".to_owned(),
                    ratatui::style::Style::new().fg(ratatui::style::Color::Green),
                ),
                AgentStatus::Missing => (
                    "missing".to_owned(),
                    ratatui::style::Style::new().fg(ratatui::style::Color::Red),
                ),
                AgentStatus::Error { reason } => (
                    format!("error: {reason}"),
                    ratatui::style::Style::new().fg(ratatui::style::Color::Red),
                ),
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("@{:<8}", desc.name.bin_name()), INPUT_PROMPT),
                Span::styled(status_label, status_style),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(
        picker
            .selected
            .min(picker.candidate_indices.len().saturating_sub(1))
            .min(3),
    ));

    let list = List::new(items)
        .highlight_symbol("› ")
        .highlight_style(HIGHLIGHTED);
    f.render_stateful_widget(list, area, &mut list_state);
}

fn active_agent_range(content: &str, cursor_pos: usize) -> Option<Range<usize>> {
    if cursor_pos > content.len() || !content.is_char_boundary(cursor_pos) {
        return None;
    }

    let prefix = &content[..cursor_pos];
    let mut token_start = 0;
    for (index, ch) in prefix.char_indices() {
        if ch.is_whitespace() {
            token_start = index + ch.len_utf8();
        }
    }

    let token = &prefix[token_start..];
    if !token.starts_with('@') {
        return None;
    }

    let query = &token[1..];
    if !query
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return None;
    }

    Some(token_start..cursor_pos)
}
