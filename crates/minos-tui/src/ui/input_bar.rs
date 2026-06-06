use std::ops::Range;

use minos_domain::{AgentName, AgentStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthChar;

use super::theme::{
    border_block, BORDER_FG, FOCUSED_BORDER, HIGHLIGHTED, INPUT_PROMPT, REASONING_STYLE,
};

const CURSOR_GLYPH: &str = "▎";
const MAX_EDITOR_ROWS: u16 = 8;

pub struct InputAgentPickerState {
    pub candidate_indices: Vec<usize>,
    pub selected: usize,
    pub replace_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMentionCandidate {
    pub token: String,
    pub agent: AgentName,
    pub kind: AgentMentionCandidateKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentMentionCandidateKind {
    Installed { status: AgentStatus },
    Existing { thread_id: String },
}

impl AgentMentionCandidate {
    pub fn installed(agent: AgentName, status: AgentStatus) -> Self {
        Self {
            token: agent.bin_name().to_owned(),
            agent,
            kind: AgentMentionCandidateKind::Installed { status },
        }
    }

    pub fn existing(agent: AgentName, thread_id: String, short_id: String) -> Self {
        Self {
            token: format!("{}#{short_id}", agent.bin_name()),
            agent,
            kind: AgentMentionCandidateKind::Existing { thread_id },
        }
    }
}

pub struct InputState {
    pub content: String,
    pub cursor_pos: usize,
    pub preferred_column: Option<usize>,
    pub focused: bool,
    pub readonly: bool,
    pub agent_picker: Option<InputAgentPickerState>,
}

impl InputState {
    pub fn new(readonly: bool) -> Self {
        Self {
            content: String::new(),
            cursor_pos: 0,
            preferred_column: None,
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
        self.preferred_column = None;
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
        self.preferred_column = None;
    }

    pub fn delete_forward(&mut self) {
        if self.readonly || self.cursor_pos >= self.content.len() {
            return;
        }
        let next =
            next_boundary(self.content.as_str(), self.cursor_pos).unwrap_or(self.content.len());
        self.content.drain(self.cursor_pos..next);
        self.preferred_column = None;
    }

    pub fn move_left(&mut self) -> bool {
        let Some(prev) = prev_boundary(self.content.as_str(), self.cursor_pos) else {
            return false;
        };
        self.cursor_pos = prev;
        self.preferred_column = None;
        true
    }

    pub fn move_right(&mut self) -> bool {
        let Some(next) = next_boundary(self.content.as_str(), self.cursor_pos) else {
            return false;
        };
        self.cursor_pos = next;
        self.preferred_column = None;
        true
    }

    pub fn move_word_left(&mut self) -> bool {
        let target = prev_word_boundary(self.content.as_str(), self.cursor_pos);
        if target == self.cursor_pos {
            return false;
        }
        self.cursor_pos = target;
        self.preferred_column = None;
        true
    }

    pub fn move_word_right(&mut self) -> bool {
        let target = next_word_boundary(self.content.as_str(), self.cursor_pos);
        if target == self.cursor_pos {
            return false;
        }
        self.cursor_pos = target;
        self.preferred_column = None;
        true
    }

    pub fn move_line_start(&mut self) -> bool {
        let target = current_line_start(self.content.as_str(), self.cursor_pos);
        if target == self.cursor_pos {
            return false;
        }
        self.cursor_pos = target;
        self.preferred_column = None;
        true
    }

    pub fn move_line_end(&mut self) -> bool {
        let target = current_line_end(self.content.as_str(), self.cursor_pos);
        if target == self.cursor_pos {
            return false;
        }
        self.cursor_pos = target;
        self.preferred_column = None;
        true
    }

    pub fn move_to_start(&mut self) -> bool {
        if self.cursor_pos == 0 {
            return false;
        }
        self.cursor_pos = 0;
        self.preferred_column = None;
        true
    }

    pub fn move_to_end(&mut self) -> bool {
        if self.cursor_pos == self.content.len() {
            return false;
        }
        self.cursor_pos = self.content.len();
        self.preferred_column = None;
        true
    }

    pub fn move_up(&mut self) -> bool {
        let current_start = current_line_start(self.content.as_str(), self.cursor_pos);
        if current_start == 0 {
            return false;
        }

        let current_col = self
            .preferred_column
            .unwrap_or_else(|| char_count(&self.content[current_start..self.cursor_pos]));
        let previous_end = current_start.saturating_sub(1);
        let previous_start = current_line_start(self.content.as_str(), previous_end);
        self.cursor_pos = byte_index_for_char_column(
            self.content.as_str(),
            previous_start,
            previous_end,
            current_col,
        );
        self.preferred_column = Some(current_col);
        true
    }

    pub fn move_down(&mut self) -> bool {
        let current_end = current_line_end(self.content.as_str(), self.cursor_pos);
        if current_end >= self.content.len() {
            return false;
        }

        let current_start = current_line_start(self.content.as_str(), self.cursor_pos);
        let current_col = self
            .preferred_column
            .unwrap_or_else(|| char_count(&self.content[current_start..self.cursor_pos]));
        let next_start = current_end + 1;
        let next_end = current_line_end(self.content.as_str(), next_start);
        self.cursor_pos =
            byte_index_for_char_column(self.content.as_str(), next_start, next_end, current_col);
        self.preferred_column = Some(current_col);
        true
    }

    pub fn delete_prev_word(&mut self) -> bool {
        if self.readonly || self.cursor_pos == 0 {
            return false;
        }
        let target = prev_word_boundary(self.content.as_str(), self.cursor_pos);
        if target == self.cursor_pos {
            return false;
        }
        self.content.drain(target..self.cursor_pos);
        self.cursor_pos = target;
        self.preferred_column = None;
        true
    }

    pub fn delete_next_word(&mut self) -> bool {
        if self.readonly || self.cursor_pos >= self.content.len() {
            return false;
        }
        let target = next_word_boundary(self.content.as_str(), self.cursor_pos);
        if target == self.cursor_pos {
            return false;
        }
        self.content.drain(self.cursor_pos..target);
        self.preferred_column = None;
        true
    }

    pub fn delete_to_line_start(&mut self) -> bool {
        if self.readonly || self.cursor_pos == 0 {
            return false;
        }
        let target = current_line_start(self.content.as_str(), self.cursor_pos);
        if target == self.cursor_pos {
            return false;
        }
        self.content.drain(target..self.cursor_pos);
        self.cursor_pos = target;
        self.preferred_column = None;
        true
    }

    pub fn delete_to_line_end(&mut self) -> bool {
        if self.readonly || self.cursor_pos >= self.content.len() {
            return false;
        }
        let target = current_line_end(self.content.as_str(), self.cursor_pos);
        if target == self.cursor_pos {
            return false;
        }
        self.content.drain(self.cursor_pos..target);
        self.preferred_column = None;
        true
    }

    pub fn take_input(&mut self) -> String {
        let taken = std::mem::take(&mut self.content);
        self.cursor_pos = 0;
        self.preferred_column = None;
        self.agent_picker = None;
        taken
    }

    pub fn clear_agent_picker(&mut self) {
        self.agent_picker = None;
    }

    pub fn sync_agent_picker(&mut self, candidates: &[AgentMentionCandidate], enabled: bool) {
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
            .and_then(|index| candidates.get(*index))
            .map(|candidate| candidate.token.clone());

        let candidate_indices: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                candidate.token.starts_with(query.as_str()).then_some(index)
            })
            .collect();

        if candidate_indices.is_empty() {
            self.agent_picker = None;
            return;
        }

        let selected = previous_agent
            .and_then(|token| {
                candidate_indices
                    .iter()
                    .position(|index| candidates[*index].token == token)
            })
            .or_else(|| {
                candidate_indices
                    .iter()
                    .position(|index| candidates[*index].token == query.as_str())
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

    pub fn accept_agent_completion(&mut self, candidates: &[AgentMentionCandidate]) -> bool {
        let Some(picker) = self.agent_picker.take() else {
            return false;
        };
        let Some(candidate_index) = picker.candidate_indices.get(picker.selected).copied() else {
            return false;
        };
        let Some(candidate) = candidates.get(candidate_index) else {
            return false;
        };

        let replacement = format!("@{} ", candidate.token);
        self.content
            .replace_range(picker.replace_range.clone(), replacement.as_str());
        self.cursor_pos = picker.replace_range.start + replacement.len();
        true
    }
}

pub fn required_height(state: &InputState, width: u16) -> u16 {
    let picker_rows = state
        .agent_picker
        .as_ref()
        .map(|picker| picker.candidate_indices.len().min(4) as u16)
        .unwrap_or(0);
    let editor_rows = editor_row_count(state, width);
    2 + picker_rows + editor_rows
}

pub fn render_input_bar(
    f: &mut Frame,
    area: Rect,
    title: &str,
    empty_hint: &str,
    state: &InputState,
    candidates: &[AgentMentionCandidate],
) {
    let block = border_block().title(title).border_style(if state.focused {
        FOCUSED_BORDER
    } else {
        ratatui::style::Style::new().fg(BORDER_FG)
    });
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let picker_rows = state
        .agent_picker
        .as_ref()
        .map(|picker| picker.candidate_indices.len().min(4) as u16)
        .unwrap_or(0)
        .min(inner.height.saturating_sub(1));
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(picker_rows), Constraint::Min(1)])
        .split(inner);
    let picker_area = sections[0];
    let input_area = sections[1];

    if picker_rows > 0 && picker_area.height > 0 {
        render_inline_agent_picker(f, picker_area, state, candidates);
    }

    let editor = build_editor_lines(state, input_area.width, empty_hint);
    let visible_rows = usize::from(MAX_EDITOR_ROWS.min(input_area.height).max(1));
    let start_row = editor
        .cursor_row
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(editor.lines.len().saturating_sub(visible_rows));
    let lines: Vec<Line<'static>> = editor
        .lines
        .into_iter()
        .skip(start_row)
        .take(visible_rows)
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, input_area);
}

fn render_inline_agent_picker(
    f: &mut Frame,
    area: Rect,
    state: &InputState,
    candidates: &[AgentMentionCandidate],
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
        .filter_map(|index| candidates.get(*index))
        .map(|candidate| {
            let (status_label, status_style) = match &candidate.kind {
                AgentMentionCandidateKind::Installed { status } => match status {
                    AgentStatus::Ok => (
                        "install".to_owned(),
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
                },
                AgentMentionCandidateKind::Existing { .. } => (
                    "session".to_owned(),
                    ratatui::style::Style::new().fg(ratatui::style::Color::Cyan),
                ),
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("@{:<16}", candidate.token), INPUT_PROMPT),
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
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '#')
    {
        return None;
    }

    Some(token_start..cursor_pos)
}

fn editor_row_count(state: &InputState, width: u16) -> u16 {
    let editor = build_editor_lines(state, width.saturating_sub(2), "Type @ to choose an agent");
    (editor.lines.len() as u16).clamp(1, MAX_EDITOR_ROWS)
}

struct EditorLines {
    lines: Vec<Line<'static>>,
    cursor_row: usize,
}

fn build_editor_lines(state: &InputState, width: u16, empty_hint: &str) -> EditorLines {
    let width = width.max(1);
    if state.readonly {
        return EditorLines {
            lines: vec![Line::from(Span::styled(
                "[readonly mode]",
                Style::new().fg(ratatui::style::Color::DarkGray),
            ))],
            cursor_row: 0,
        };
    }

    let display = match (state.content.is_empty(), state.focused) {
        (true, true) => (CURSOR_GLYPH.to_owned(), Style::default()),
        (true, false) => (empty_hint.to_owned(), REASONING_STYLE),
        (false, true) => (
            insert_cursor_marker(state.content.as_str(), state.cursor_pos),
            Style::default(),
        ),
        (false, false) => (state.content.clone(), Style::default()),
    };

    let lines = wrap_styled_text(display.0.as_str(), width, display.1);
    let cursor_row = if state.focused {
        wrapped_row_for_cursor(state.content.as_str(), state.cursor_pos, width)
    } else {
        0
    };

    EditorLines { lines, cursor_row }
}

fn insert_cursor_marker(content: &str, cursor_pos: usize) -> String {
    let mut rendered = String::with_capacity(content.len() + CURSOR_GLYPH.len());
    rendered.push_str(&content[..cursor_pos]);
    rendered.push_str(CURSOR_GLYPH);
    rendered.push_str(&content[cursor_pos..]);
    rendered
}

fn wrap_styled_text(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for ch in text.chars() {
        if ch == '\n' {
            lines.push(Line::from(Span::styled(
                std::mem::take(&mut current),
                style,
            )));
            current_width = 0;
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width > 0 && ch_width > 0 && current_width + ch_width > width {
            lines.push(Line::from(Span::styled(
                std::mem::take(&mut current),
                style,
            )));
            current_width = 0;
        }
        current.push(ch);
        current_width = current_width.saturating_add(ch_width);
    }

    if current.is_empty() && lines.is_empty() {
        lines.push(Line::from(Span::styled(String::new(), style)));
    } else {
        lines.push(Line::from(Span::styled(current, style)));
    }

    lines
}

fn wrapped_row_for_cursor(content: &str, cursor_pos: usize, width: u16) -> usize {
    let width = usize::from(width.max(1));
    let mut row = 0usize;
    let mut col_width = 0usize;

    for (index, ch) in content.char_indices() {
        if index == cursor_pos {
            return row;
        }

        if ch == '\n' {
            row += 1;
            col_width = 0;
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col_width > 0 && ch_width > 0 && col_width + ch_width > width {
            row += 1;
            col_width = 0;
        }
        col_width = col_width.saturating_add(ch_width);
    }

    row
}

fn prev_boundary(content: &str, cursor_pos: usize) -> Option<usize> {
    content[..cursor_pos]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
}

fn next_boundary(content: &str, cursor_pos: usize) -> Option<usize> {
    content[cursor_pos..]
        .chars()
        .next()
        .map(|ch| cursor_pos + ch.len_utf8())
}

fn prev_word_boundary(content: &str, cursor_pos: usize) -> usize {
    let mut cursor = cursor_pos;
    while let Some((index, ch)) = prev_char(content, cursor) {
        if !ch.is_whitespace() {
            break;
        }
        cursor = index;
    }
    while let Some((index, ch)) = prev_char(content, cursor) {
        if ch.is_whitespace() {
            break;
        }
        cursor = index;
    }
    cursor
}

fn next_word_boundary(content: &str, cursor_pos: usize) -> usize {
    let mut cursor = cursor_pos;
    while let Some((next, ch)) = next_char(content, cursor) {
        if !ch.is_whitespace() {
            break;
        }
        cursor = next;
    }
    while let Some((next, ch)) = next_char(content, cursor) {
        if ch.is_whitespace() {
            break;
        }
        cursor = next;
    }
    cursor
}

fn prev_char(content: &str, cursor_pos: usize) -> Option<(usize, char)> {
    content[..cursor_pos].char_indices().next_back()
}

fn next_char(content: &str, cursor_pos: usize) -> Option<(usize, char)> {
    let ch = content[cursor_pos..].chars().next()?;
    Some((cursor_pos + ch.len_utf8(), ch))
}

fn current_line_start(content: &str, cursor_pos: usize) -> usize {
    content[..cursor_pos]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn current_line_end(content: &str, cursor_pos: usize) -> usize {
    content[cursor_pos..]
        .find('\n')
        .map(|offset| cursor_pos + offset)
        .unwrap_or(content.len())
}

fn char_count(content: &str) -> usize {
    content.chars().count()
}

fn byte_index_for_char_column(
    content: &str,
    line_start: usize,
    line_end: usize,
    column: usize,
) -> usize {
    if column == 0 {
        return line_start;
    }

    let line = &content[line_start..line_end];
    match line.char_indices().nth(column) {
        Some((offset, _)) => line_start + offset,
        None => line_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_mid_string_editing() {
        let mut state = InputState::new(false);
        state.content = "helo".into();
        state.cursor_pos = 3;

        assert!(state.move_left());
        state.insert_char('l');

        assert_eq!(state.content, "hello");
        assert_eq!(state.cursor_pos, 3);
    }

    #[test]
    fn word_motion_and_word_delete_follow_whitespace_chunks() {
        let mut state = InputState::new(false);
        state.content = "hello   brave new world".into();
        state.cursor_pos = state.content.len();

        assert!(state.delete_prev_word());
        assert_eq!(state.content, "hello   brave new ");
        assert_eq!(state.cursor_pos, "hello   brave new ".len());

        assert!(state.move_word_left());
        assert_eq!(state.cursor_pos, "hello   brave ".len());

        assert!(state.delete_next_word());
        assert_eq!(state.content, "hello   brave  ");
    }

    #[test]
    fn vertical_motion_preserves_column_when_possible() {
        let mut state = InputState::new(false);
        state.content = "alpha\nbravo\ncar".into();
        state.cursor_pos = "alpha\nbra".len();

        assert!(state.move_down());
        assert_eq!(state.cursor_pos, "alpha\nbravo\ncar".len());

        assert!(state.move_up());
        assert_eq!(state.cursor_pos, "alpha\nbra".len());
    }

    #[test]
    fn required_height_grows_with_multiline_input_and_caps() {
        let mut state = InputState::new(false);
        state.content = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine".into();
        state.cursor_pos = state.content.len();

        assert_eq!(required_height(&state, 40), 10);
    }
}
