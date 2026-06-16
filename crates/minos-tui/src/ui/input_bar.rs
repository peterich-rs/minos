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

const MAX_EDITOR_ROWS: u16 = 8;

/// Layout metrics captured during `render_input_bar` so that mouse click
/// handlers can map screen coordinates back to byte offsets in the editor
/// content.
#[derive(Clone, Copy, Debug, Default)]
pub struct InputLayoutMetrics {
    pub outer: Rect,
    pub editor_area: Rect,
    pub width: u16,
    pub start_row: usize,
    pub visible_rows: usize,
}

/// Visual style for the focused input cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorStyle {
    /// Thin bar inserted before the cursor character (`│`).
    #[default]
    Bar,
    /// Block that reverses the character at the cursor position.
    Block,
}

pub struct InputAgentPickerState {
    pub candidate_indices: Vec<usize>,
    pub selected: usize,
    pub replace_range: Range<usize>,
}

#[derive(Default)]
pub enum InputPicker {
    #[default]
    None,
    Agent(InputAgentPickerState),
    Path(InputPathPickerState),
}

#[derive(Clone)]
pub struct InputPathPickerState {
    pub candidates: Vec<PathCandidate>,
    pub selected: usize,
    pub replace_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathCandidate {
    pub name: String,
    pub is_dir: bool,
}

/// Prompt history for an input bar, browsable with Up/Down arrow keys.
///
/// `entries` stores submitted prompts in chronological order. `cursor` is
/// `Some(index)` while the user is browsing history (pointing at the entry
/// currently loaded into the input). `draft` holds the in-progress text the
/// user had typed before browsing, so Esc or ↓-past-end restores it.
pub struct PromptHistory {
    pub entries: Vec<String>,
    pub cursor: Option<usize>,
    pub draft: String,
}

impl PromptHistory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            draft: String::new(),
        }
    }

    /// Records a submitted prompt. Blank (whitespace-only) entries are ignored
    /// so the history stays useful. Always resets browsing state.
    pub fn record(&mut self, entry: &str) {
        if !entry.trim().is_empty() {
            self.entries.push(entry.to_owned());
        }
        self.cursor = None;
        self.draft.clear();
    }

    /// Moves the browse cursor one entry older and returns that entry.
    ///
    /// On the first call (cursor is `None`) the current input text is captured
    /// into `draft` so it can be restored later, and the cursor jumps to the
    /// most recent entry. Returns `None` if there is no history, or if the
    /// cursor is already at the oldest entry — callers should treat `None` as
    /// "stay put" (clamped at the top).
    pub fn previous(&mut self, current_draft: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        match self.cursor {
            None => {
                self.draft = current_draft.to_owned();
                let idx = self.entries.len() - 1;
                self.cursor = Some(idx);
                Some(self.entries[idx].as_str())
            }
            Some(0) => None,
            Some(idx) => {
                let new_idx = idx - 1;
                self.cursor = Some(new_idx);
                Some(self.entries[new_idx].as_str())
            }
        }
    }

    /// Moves the browse cursor one entry newer. Returns `None` and clears
    /// browsing when moving past the most recent entry (the caller should
    /// restore the draft in that case, or call [`cancel`]).
    pub fn next(&mut self) -> Option<&str> {
        let idx = self.cursor?;
        let new_idx = idx + 1;
        if new_idx >= self.entries.len() {
            self.cursor = None;
            return None;
        }
        self.cursor = Some(new_idx);
        Some(self.entries[new_idx].as_str())
    }

    /// Cancels browsing, clears the cursor, and returns the saved draft.
    pub fn cancel(&mut self) -> &str {
        self.cursor = None;
        &self.draft
    }

    /// Returns `true` while the user is browsing history entries.
    pub fn is_browsing(&self) -> bool {
        self.cursor.is_some()
    }
}

impl Default for PromptHistory {
    fn default() -> Self {
        Self::new()
    }
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
    pub cursor_style: CursorStyle,
    pub multiline: bool,
    pub picker: InputPicker,
    pub history: PromptHistory,
}

impl InputState {
    pub fn new(readonly: bool) -> Self {
        Self {
            content: String::new(),
            cursor_pos: 0,
            preferred_column: None,
            focused: true,
            readonly,
            cursor_style: CursorStyle::default(),
            multiline: false,
            picker: InputPicker::None,
            history: PromptHistory::new(),
        }
    }

    pub fn toggle_cursor_style(&mut self) {
        self.cursor_style = match self.cursor_style {
            CursorStyle::Bar => CursorStyle::Block,
            CursorStyle::Block => CursorStyle::Bar,
        };
    }

    pub fn toggle_multiline(&mut self) {
        self.multiline = !self.multiline;
    }

    pub fn insert_char(&mut self, c: char) {
        if self.readonly {
            return;
        }
        self.content.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
        self.preferred_column = None;
    }

    pub fn insert_str(&mut self, text: &str) {
        if self.readonly || text.is_empty() {
            return;
        }
        self.content.insert_str(self.cursor_pos, text);
        self.cursor_pos += text.len();
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
        self.picker = InputPicker::None;
        self.history.cursor = None;
        taken
    }

    pub fn clear_agent_picker(&mut self) {
        self.picker = InputPicker::None;
    }

    pub fn sync_agent_picker(&mut self, candidates: &[AgentMentionCandidate], enabled: bool) {
        if !enabled || self.readonly {
            self.picker = InputPicker::None;
            return;
        }

        let Some(replace_range) = active_agent_range(&self.content, self.cursor_pos) else {
            self.picker = InputPicker::None;
            return;
        };
        let query = self.content[replace_range.start + 1..replace_range.end].to_ascii_lowercase();

        let previous_agent = match &self.picker {
            InputPicker::Agent(p) => p
                .candidate_indices
                .get(p.selected)
                .and_then(|index| candidates.get(*index))
                .map(|candidate| candidate.token.clone()),
            _ => None,
        };

        let candidate_indices: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                candidate.token.starts_with(query.as_str()).then_some(index)
            })
            .collect();

        if candidate_indices.is_empty() {
            self.picker = InputPicker::None;
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

        self.picker = InputPicker::Agent(InputAgentPickerState {
            candidate_indices,
            selected,
            replace_range,
        });
    }

    pub fn has_agent_picker(&self) -> bool {
        matches!(&self.picker, InputPicker::Agent(p) if !p.candidate_indices.is_empty())
    }

    pub fn has_path_picker(&self) -> bool {
        matches!(&self.picker, InputPicker::Path(p) if !p.candidates.is_empty())
    }

    pub fn select_previous_agent(&mut self) -> bool {
        let InputPicker::Agent(picker) = &mut self.picker else {
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
        let InputPicker::Agent(picker) = &mut self.picker else {
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
        let InputPicker::Agent(picker) = std::mem::take(&mut self.picker) else {
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

    pub fn sync_path_picker(&mut self, workspace_root: &std::path::Path) {
        if self.readonly {
            return;
        }
        let Some(replace_range) = active_path_range(&self.content, self.cursor_pos) else {
            return;
        };
        let token = self.content[replace_range.start..replace_range.end].to_owned();
        let Some(candidates) = list_path_candidates(&token, workspace_root) else {
            return;
        };
        self.picker = InputPicker::Path(InputPathPickerState {
            candidates,
            selected: 0,
            replace_range,
        });
    }

    pub fn clear_path_picker(&mut self) {
        if matches!(self.picker, InputPicker::Path(_)) {
            self.picker = InputPicker::None;
        }
    }

    pub fn select_previous_path(&mut self) -> bool {
        let InputPicker::Path(picker) = &mut self.picker else {
            return false;
        };
        let len = picker.candidates.len();
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

    pub fn select_next_path(&mut self) -> bool {
        let InputPicker::Path(picker) = &mut self.picker else {
            return false;
        };
        let len = picker.candidates.len();
        if len == 0 {
            return false;
        }
        picker.selected = (picker.selected + 1) % len;
        true
    }

    pub fn accept_path_completion(&mut self) -> bool {
        let (candidate, replace_range) = match &self.picker {
            InputPicker::Path(p) => {
                let candidate = p.candidates[p.selected].clone();
                (candidate, p.replace_range.clone())
            }
            _ => return false,
        };
        let existing_token = self.content[replace_range.start..replace_range.end].to_owned();
        let last_slash = existing_token.rfind('/').unwrap_or(0);
        let dir_prefix = &existing_token[..=last_slash];

        let mut replacement = format!("{dir_prefix}{}", candidate.name);
        let is_dir = candidate.is_dir;
        if is_dir {
            replacement.push('/');
        }

        self.content.replace_range(replace_range.clone(), &replacement);
        self.cursor_pos = replace_range.start + replacement.len();
        self.preferred_column = None;

        if is_dir {
            false
        } else {
            self.picker = InputPicker::None;
            true
        }
    }

    /// Replaces the input content with `entry`, placing the cursor at the end
    /// and clearing any preferred column. Used when loading a history entry.
    pub fn load_history_entry(&mut self, entry: &str) {
        self.content = entry.to_owned();
        self.cursor_pos = self.content.len();
        self.preferred_column = None;
    }
}

pub fn required_height(state: &InputState, width: u16) -> u16 {
    let picker_rows = match &state.picker {
        InputPicker::Agent(p) => p.candidate_indices.len().min(4) as u16,
        InputPicker::Path(p) => p.candidates.len().min(4) as u16,
        InputPicker::None => 0,
    };
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
    metrics: &mut InputLayoutMetrics,
) {
    let block = border_block().title(title).border_style(if state.focused {
        FOCUSED_BORDER
    } else {
        ratatui::style::Style::new().fg(BORDER_FG)
    });
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        *metrics = InputLayoutMetrics {
            outer: area,
            editor_area: Rect::default(),
            width: 0,
            start_row: 0,
            visible_rows: 0,
        };
        return;
    }

    let picker_rows = match &state.picker {
        InputPicker::Agent(p) => p.candidate_indices.len().min(4) as u16,
        InputPicker::Path(p) => p.candidates.len().min(4) as u16,
        InputPicker::None => 0,
    }
    .min(inner.height.saturating_sub(1));
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(picker_rows), Constraint::Min(1)])
        .split(inner);
    let picker_area = sections[0];
    let input_area = sections[1];

    if picker_rows > 0 && picker_area.height > 0 {
        match &state.picker {
            InputPicker::Agent(_) => {
                render_inline_agent_picker(f, picker_area, state, candidates);
            }
            InputPicker::Path(_) => {
                render_inline_path_picker(f, picker_area, state);
            }
            InputPicker::None => {}
        }
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

    *metrics = InputLayoutMetrics {
        outer: area,
        editor_area: input_area,
        width: input_area.width,
        start_row,
        visible_rows,
    };
}

fn render_inline_agent_picker(
    f: &mut Frame,
    area: Rect,
    state: &InputState,
    candidates: &[AgentMentionCandidate],
) {
    let InputPicker::Agent(picker) = &state.picker else {
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
            let mut spans = vec![Span::styled(
                format!("@{:<16}", candidate.token),
                INPUT_PROMPT,
            )];
            if let Some((status_label, status_style)) = agent_picker_status_label(candidate) {
                spans.push(Span::styled(status_label, status_style));
            }
            ListItem::new(Line::from(spans))
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

fn render_inline_path_picker(f: &mut Frame, area: Rect, state: &InputState) {
    let InputPicker::Path(picker) = &state.picker else {
        return;
    };
    if picker.candidates.is_empty() || area.height == 0 {
        return;
    }

    let items: Vec<ListItem> = picker
        .candidates
        .iter()
        .take(4)
        .map(|candidate| {
            let suffix = if candidate.is_dir { "/" } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled("path ", INPUT_PROMPT),
                Span::raw(format!("{}{}", candidate.name, suffix)),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(
        picker
            .selected
            .min(picker.candidates.len().saturating_sub(1))
            .min(3),
    ));

    let list = List::new(items)
        .highlight_symbol("› ")
        .highlight_style(HIGHLIGHTED);
    f.render_stateful_widget(list, area, &mut list_state);
}

fn agent_picker_status_label(
    candidate: &AgentMentionCandidate,
) -> Option<(String, ratatui::style::Style)> {
    match &candidate.kind {
        AgentMentionCandidateKind::Installed { status } => match status {
            AgentStatus::Ok => Some((
                "install".to_owned(),
                ratatui::style::Style::new().fg(ratatui::style::Color::Green),
            )),
            AgentStatus::Missing => Some((
                "missing".to_owned(),
                ratatui::style::Style::new().fg(ratatui::style::Color::Red),
            )),
            AgentStatus::Error { reason } => Some((
                format!("error: {reason}"),
                ratatui::style::Style::new().fg(ratatui::style::Color::Red),
            )),
        },
        AgentMentionCandidateKind::Existing { .. } => None,
    }
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

pub fn active_path_range(content: &str, cursor_pos: usize) -> Option<Range<usize>> {
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
    if !token.contains('/') && !token.starts_with("~/") {
        return None;
    }
    Some(token_start..cursor_pos)
}

pub fn list_path_candidates(token: &str, workspace_root: &std::path::Path) -> Option<Vec<PathCandidate>> {
    let last_slash = token.rfind('/')?;
    let dir_prefix = &token[..=last_slash];
    let partial_name = &token[last_slash + 1..];

    let resolved: std::path::PathBuf = if dir_prefix.starts_with("~/") {
        dirs::home_dir()?.join(&dir_prefix[2..])
    } else if dir_prefix.starts_with('/') {
        std::path::PathBuf::from(dir_prefix)
    } else {
        workspace_root.join(dir_prefix)
    };

    let entries = std::fs::read_dir(&resolved).ok()?;
    let mut candidates: Vec<PathCandidate> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(partial_name) {
                return None;
            }
            let is_dir = entry.file_type().ok()?.is_dir();
            Some(PathCandidate { name, is_dir })
        })
        .collect();
    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    candidates.truncate(8);
    if candidates.is_empty() {
        return None;
    }
    Some(candidates)
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

    let reversed = Style::new().add_modifier(ratatui::style::Modifier::REVERSED);

    if state.content.is_empty() {
        if state.focused {
            let line = match state.cursor_style {
                CursorStyle::Bar => Line::from(Span::styled("│", reversed)),
                CursorStyle::Block => Line::from(Span::styled(" ", reversed)),
            };
            return EditorLines {
                lines: vec![line],
                cursor_row: 0,
            };
        }
        return EditorLines {
            lines: vec![Line::from(Span::styled(
                empty_hint.to_owned(),
                REASONING_STYLE,
            ))],
            cursor_row: 0,
        };
    }

    if !state.focused {
        let lines = wrap_styled_text(state.content.as_str(), width, Style::default());
        return EditorLines { lines, cursor_row: 0 };
    }

    // Focused with content: wrap text, then apply cursor style.
    let cursor_row =
        wrapped_row_for_cursor(state.content.as_str(), state.cursor_pos, width);
    let mut lines = wrap_styled_text(state.content.as_str(), width, Style::default());
    if let Some(line) = lines.get_mut(cursor_row) {
        apply_cursor_to_line(
            line,
            state.content.as_str(),
            state.cursor_pos,
            width,
            state.cursor_style,
        );
    }

    EditorLines { lines, cursor_row }
}

/// Applies the visual cursor style to a single wrapped `Line`. `line` must be
/// one of the lines produced by `wrap_styled_text` (a single styled span whose
/// content is the visible text for that row). `content`/`cursor_pos`/`width`
/// describe the full editor content so the cursor's character column within
/// this visual row can be computed.
fn apply_cursor_to_line(
    line: &mut Line<'static>,
    content: &str,
    cursor_pos: usize,
    width: u16,
    cursor_style: CursorStyle,
) {
    let reversed = Style::new().add_modifier(ratatui::style::Modifier::REVERSED);
    let width = usize::from(width.max(1));

    let text = match line.spans.first() {
        Some(span) => span.content.clone().into_owned(),
        None => return,
    };

    // Walk the full content to find the character column of the cursor within
    // its visual row (the same row whose text `line` displays).
    let mut col = 0usize;
    let mut cursor_char_col: Option<usize> = None;
    for (byte_idx, ch) in content.char_indices() {
        if byte_idx == cursor_pos {
            cursor_char_col = Some(col);
            break;
        }
        if ch == '\n' {
            // Newline ends the current visual row; cursor is on a later row.
            break;
        }
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col > 0 && ch_width > 0 && col + ch_width > width {
            col = 0;
        }
        col += ch_width;
    }

    let Some(cursor_col) = cursor_char_col else {
        return;
    };

    let char_text: Vec<char> = text.chars().collect();
    let split_at = cursor_col.min(char_text.len());

    let before: String = char_text[..split_at].iter().collect();
    let cursor_char: String = if split_at < char_text.len() {
        char_text[split_at..=split_at].iter().collect()
    } else {
        String::new()
    };
    let after: String = if split_at + 1 < char_text.len() {
        char_text[split_at + 1..].iter().collect()
    } else {
        String::new()
    };

    let mut new_spans = Vec::new();
    if !before.is_empty() {
        new_spans.push(Span::raw(before));
    }
    match cursor_style {
        CursorStyle::Bar => {
            new_spans.push(Span::styled("│", reversed));
            if !cursor_char.is_empty() {
                new_spans.push(Span::raw(cursor_char));
            }
        }
        CursorStyle::Block => {
            if cursor_char.is_empty() {
                new_spans.push(Span::styled(" ", reversed));
            } else {
                new_spans.push(Span::styled(cursor_char, reversed));
            }
        }
    }
    if !after.is_empty() {
        new_spans.push(Span::raw(after));
    }

    line.spans = new_spans;
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

/// Returns the 0-indexed visual row the cursor currently sits on, accounting
/// for soft-wrapping at `width` and explicit `\n` line breaks.
pub fn visual_cursor_row(content: &str, cursor_pos: usize, width: u16) -> usize {
    wrapped_row_for_cursor(content, cursor_pos, width)
}

/// Returns the index of the last visual row for `content` at `width`, i.e. the
/// row containing the final character (or `0` for empty/single-line input).
/// Used to detect when the cursor is on the bottom row so ↓ can fall through
/// to history navigation.
pub fn visual_row_count(content: &str, width: u16) -> usize {
    let width = usize::from(width.max(1));
    let mut row = 0usize;
    let mut col_width = 0usize;
    for ch in content.chars() {
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

/// Maps a visual `(target_row, target_col)` position — as produced by a mouse
/// click — back to the nearest byte offset in `content`. Wrapping matches the
/// display logic in `wrap_styled_text`.
pub fn byte_offset_for_visual_position(
    content: &str,
    target_row: usize,
    target_col: usize,
    width: u16,
) -> usize {
    let width = usize::from(width.max(1));
    let mut row = 0usize;
    let mut col_width = 0usize;

    for (byte_idx, ch) in content.char_indices() {
        if row == target_row && col_width >= target_col {
            return byte_idx;
        }
        if ch == '\n' {
            if row == target_row {
                return byte_idx;
            }
            row += 1;
            col_width = 0;
            continue;
        }
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col_width > 0 && ch_width > 0 && col_width + ch_width > width {
            row += 1;
            col_width = 0;
            if row > target_row {
                return byte_idx;
            }
        }
        col_width = col_width.saturating_add(ch_width);
    }
    content.len()
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

    #[test]
    fn existing_agent_candidate_has_no_session_status_label() {
        let installed = AgentMentionCandidate::installed(AgentName::Codex, AgentStatus::Ok);
        let existing = AgentMentionCandidate::existing(
            AgentName::Codex,
            "thread-codex-1234".into(),
            "thread-c".into(),
        );

        assert_eq!(
            agent_picker_status_label(&installed).map(|(label, _)| label),
            Some("install".to_owned())
        );
        assert!(agent_picker_status_label(&existing).is_none());
    }

    #[test]
    fn prompt_history_prev_loads_last_entry() {
        let mut h = PromptHistory::new();
        h.record("first");
        h.record("second");
        assert_eq!(h.previous(""), Some("second"));
    }

    #[test]
    fn prompt_history_prev_walks_backward_then_clamps() {
        let mut h = PromptHistory::new();
        h.record("a");
        h.record("b");
        h.record("c");
        assert_eq!(h.previous(""), Some("c"));
        assert_eq!(h.previous(""), Some("b"));
        assert_eq!(h.previous(""), Some("a"));
        // At oldest entry — returns None (clamp); cursor stays at index 0.
        assert_eq!(h.previous(""), None);
        assert!(h.is_browsing());
    }

    #[test]
    fn prompt_history_next_past_end_returns_none_and_clears_browsing() {
        let mut h = PromptHistory::new();
        h.record("entry");
        let _ = h.previous("my draft");
        assert!(h.is_browsing());
        assert_eq!(h.next(), None);
        assert!(!h.is_browsing());
    }

    #[test]
    fn prompt_history_cancel_restores_draft_and_clears_browsing() {
        let mut h = PromptHistory::new();
        h.record("entry");
        let _ = h.previous("original draft");
        assert!(h.is_browsing());
        assert_eq!(h.cancel(), "original draft");
        assert!(!h.is_browsing());
    }

    #[test]
    fn prompt_history_cancel_without_browsing_returns_empty_draft() {
        let mut h = PromptHistory::new();
        h.record("entry");
        assert!(!h.is_browsing());
        assert_eq!(h.cancel(), "");
    }

    #[test]
    fn prompt_history_record_clears_browsing_state() {
        let mut h = PromptHistory::new();
        h.record("a");
        let _ = h.previous("draft");
        assert!(h.is_browsing());
        h.record("b");
        assert!(!h.is_browsing());
    }

    #[test]
    fn prompt_history_ignores_blank_submissions() {
        let mut h = PromptHistory::new();
        h.record("   ");
        h.record("");
        h.record("\t\n");
        assert_eq!(h.entries.len(), 0);
        assert_eq!(h.previous("current"), None);
    }

    #[test]
    fn prompt_history_empty_returns_none_from_previous() {
        let mut h = PromptHistory::new();
        assert_eq!(h.previous("anything"), None);
        assert!(!h.is_browsing());
    }

    #[test]
    fn prompt_history_next_walks_forward_through_entries() {
        let mut h = PromptHistory::new();
        h.record("a");
        h.record("b");
        h.record("c");
        assert_eq!(h.previous(""), Some("c"));
        assert_eq!(h.previous(""), Some("b"));
        assert_eq!(h.next(), Some("c"));
        assert_eq!(h.next(), None);
    }

    #[test]
    fn visual_row_count_counts_soft_wrapped_rows() {
        // 15 chars at width 5 → 3 visual rows (indices 0, 1, 2).
        assert_eq!(visual_row_count("abcdefghijklmno", 5), 2);
    }

    #[test]
    fn visual_row_count_single_line_short_text_is_zero() {
        assert_eq!(visual_row_count("hello", 80), 0);
    }

    #[test]
    fn visual_row_count_counts_explicit_newlines() {
        // "hello\nworld" at width 80 → row 0, then \n → row 1, "world" stays → returns 1.
        assert_eq!(visual_row_count("hello\nworld", 80), 1);
    }

    #[test]
    fn visual_row_count_empty_string_is_zero() {
        assert_eq!(visual_row_count("", 80), 0);
    }

    #[test]
    fn visual_cursor_row_at_start_is_zero() {
        assert_eq!(visual_cursor_row("hello world", 0, 80), 0);
    }

    #[test]
    fn visual_cursor_row_after_newline_is_one() {
        let content = "hello\nworld";
        let pos = "hello\n".len();
        assert_eq!(visual_cursor_row(content, pos, 80), 1);
    }

    #[test]
    fn visual_cursor_row_after_soft_wrap_is_one() {
        // 5 chars fill row 0; the 6th char 'f' triggers a wrap to row 1.
        let content = "abcdefghijklmno";
        // cursor at byte 6 (start of 'g', i.e. after 'f' has wrapped) → row 1.
        assert_eq!(visual_cursor_row(content, 6, 5), 1);
        // cursor at byte 11 (start of 'l', after 'k' wrapped) → row 2.
        assert_eq!(visual_cursor_row(content, 11, 5), 2);
        // cursor at byte 5 (boundary) is still on row 0 — no wrap processed yet.
        assert_eq!(visual_cursor_row(content, 5, 5), 0);
    }

    #[test]
    fn active_path_range_extracts_path_token() {
        assert_eq!(active_path_range("hello src/foo", 12), Some(6..12));
        assert_eq!(active_path_range("~/foo", 5), Some(0..5));
        assert_eq!(active_path_range("no path here", 12), None);
        assert_eq!(active_path_range("./bar", 5), Some(0..5));
    }

    #[test]
    fn active_path_range_rejects_plain_words() {
        // No slash present and not starting with ~/ → None
        assert_eq!(active_path_range("hello world", 11), None);
        assert_eq!(active_path_range("word", 4), None);
    }

    #[test]
    fn sync_path_picker_lists_directory_entries() {
        use std::fs;
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/foo.rs"), "fn main() {}").expect("write foo.rs");
        fs::write(root.join("src/bar.rs"), "fn other() {}").expect("write bar.rs");

        let mut state = InputState::new(false);
        state.content = "edit src/f".to_owned();
        state.cursor_pos = state.content.len();

        state.sync_path_picker(root);

        match &state.picker {
            InputPicker::Path(p) => {
                assert_eq!(p.candidates.len(), 1);
                assert_eq!(p.candidates[0].name, "foo.rs");
                assert!(!p.candidates[0].is_dir);
            }
            _ => panic!("expected path picker"),
        }
    }

    #[test]
    fn accept_path_completion_inserts_file_name() {
        use std::fs;
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/foo.rs"), "").expect("write foo.rs");

        let mut state = InputState::new(false);
        state.content = "edit src/f".to_owned();
        state.cursor_pos = state.content.len();
        state.sync_path_picker(root);

        let completed = state.accept_path_completion();
        assert!(completed);
        assert!(state.content.ends_with("src/foo.rs"));
        assert_eq!(state.cursor_pos, state.content.len());
        assert!(matches!(state.picker, InputPicker::None));
    }

    #[test]
    fn accept_path_completion_dir_triggers_re_sync() {
        use std::fs;
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        fs::create_dir_all(root.join("src/sub")).expect("mkdir src/sub");
        fs::write(root.join("src/sub/main.rs"), "").expect("write main.rs");

        let mut state = InputState::new(false);
        state.content = "edit src/su".to_owned();
        state.cursor_pos = state.content.len();
        state.sync_path_picker(root);

        let completed = state.accept_path_completion();
        assert!(!completed); // dir → caller should re-sync
        assert!(state.content.ends_with("src/sub/"));
    }

    #[test]
    fn byte_offset_for_visual_position_clamps_to_line_end() {
        let content = "hello world";
        let offset = byte_offset_for_visual_position(content, 0, 100, 80);
        assert_eq!(offset, content.len());
    }

    #[test]
    fn byte_offset_for_visual_position_handles_multiline() {
        let content = "hello\nworld";
        let offset = byte_offset_for_visual_position(content, 1, 0, 80);
        assert_eq!(offset, 6);
    }

    #[test]
    fn byte_offset_for_visual_position_single_line_basic() {
        let content = "hello";
        assert_eq!(byte_offset_for_visual_position(content, 0, 0, 80), 0);
        assert_eq!(byte_offset_for_visual_position(content, 0, 2, 80), 2);
        assert_eq!(byte_offset_for_visual_position(content, 0, 5, 80), 5);
    }

    #[test]
    fn byte_offset_for_visual_position_multiline_mid_row() {
        let content = "hello\nworld";
        assert_eq!(byte_offset_for_visual_position(content, 1, 3, 80), 9);
    }

    #[test]
    fn byte_offset_for_visual_position_past_last_row_clamps_to_end() {
        let content = "hello\nworld";
        assert_eq!(byte_offset_for_visual_position(content, 5, 0, 80), content.len());
    }

    #[test]
    fn cursor_style_toggle_flips_between_bar_and_block() {
        let mut state = InputState::new(false);
        assert_eq!(state.cursor_style, CursorStyle::Bar);
        state.toggle_cursor_style();
        assert_eq!(state.cursor_style, CursorStyle::Block);
        state.toggle_cursor_style();
        assert_eq!(state.cursor_style, CursorStyle::Bar);
    }

    #[test]
    fn multiline_toggle_flips_mode() {
        let mut state = InputState::new(false);
        assert!(!state.multiline);
        state.toggle_multiline();
        assert!(state.multiline);
        state.toggle_multiline();
        assert!(!state.multiline);
    }
}
