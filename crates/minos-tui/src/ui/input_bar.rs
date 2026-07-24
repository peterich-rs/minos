use std::ops::Range;

use minos_domain::{AgentName, AgentStatus};

mod render;

use render::active_agent_range;
pub use render::{
    active_path_range, byte_offset_for_visual_position, last_visual_row, required_height,
    visual_cursor_row, InputBarRenderable, InputLayoutMetrics,
};
// Re-export so UI callers can keep importing from input_bar.
#[cfg(test)]
pub use crate::path_complete::list_path_candidates;
pub use crate::path_complete::PathCandidate;

#[cfg(test)]
use render::agent_picker_status_label;

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

struct PendingPathCompletion {
    sequence: u64,
    token: String,
    replace_range: Range<usize>,
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
    Installed {
        status: AgentStatus,
    },
    /// Host agent profile (`@Name` or `@p/<id>`).
    Profile {
        profile_id: String,
    },
    Existing {
        session_id: String,
    },
}

impl AgentMentionCandidate {
    pub fn installed(agent: AgentName, status: AgentStatus) -> Self {
        Self {
            token: agent.bin_name().to_owned(),
            agent,
            kind: AgentMentionCandidateKind::Installed { status },
        }
    }

    pub fn profile(token: String, agent: AgentName, profile_id: String) -> Self {
        Self {
            token,
            agent,
            kind: AgentMentionCandidateKind::Profile { profile_id },
        }
    }

    pub fn existing(agent: AgentName, session_id: String, short_id: String) -> Self {
        Self {
            token: format!("{}#{short_id}", agent.bin_name()),
            agent,
            kind: AgentMentionCandidateKind::Existing { session_id },
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
    path_completion_sequence: u64,
    pending_path_completion: Option<PendingPathCompletion>,
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
            path_completion_sequence: 0,
            pending_path_completion: None,
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
                candidate
                    .token
                    .to_ascii_lowercase()
                    .contains(query.as_str())
                    .then_some(index)
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

    pub fn sync_path_picker(&mut self) -> Option<(u64, String)> {
        if self.readonly {
            self.pending_path_completion = None;
            self.clear_path_picker();
            return None;
        }
        let Some(replace_range) = active_path_range(&self.content, self.cursor_pos) else {
            self.pending_path_completion = None;
            self.clear_path_picker();
            return None;
        };
        let token = self.content[replace_range.start..replace_range.end].to_owned();
        self.path_completion_sequence = self.path_completion_sequence.wrapping_add(1);
        let sequence = self.path_completion_sequence;
        self.pending_path_completion = Some(PendingPathCompletion {
            sequence,
            token: token.clone(),
            replace_range,
        });
        self.picker = InputPicker::None;
        Some((sequence, token))
    }

    pub fn apply_path_candidates(&mut self, sequence: u64, candidates: Vec<PathCandidate>) -> bool {
        let Some(pending) = self.pending_path_completion.as_ref() else {
            return false;
        };
        if pending.sequence != sequence {
            return false;
        }
        let Some(pending) = self.pending_path_completion.take() else {
            return false;
        };

        if self.content.get(pending.replace_range.clone()) != Some(pending.token.as_str()) {
            return false;
        }
        if candidates.is_empty() {
            return false;
        }
        self.picker = InputPicker::Path(InputPathPickerState {
            candidates,
            selected: 0,
            replace_range: pending.replace_range,
        });
        true
    }

    pub fn clear_path_picker(&mut self) {
        self.pending_path_completion = None;
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
                let Some(candidate) = p.candidates.get(p.selected).cloned() else {
                    self.picker = InputPicker::None;
                    return false;
                };
                (candidate, p.replace_range.clone())
            }
            _ => return false,
        };

        if replace_range.end > self.content.len()
            || !self.content.is_char_boundary(replace_range.start)
            || !self.content.is_char_boundary(replace_range.end)
        {
            self.picker = InputPicker::None;
            return false;
        }

        let existing_token = self.content[replace_range.start..replace_range.end].to_owned();
        let last_slash = existing_token.rfind('/').unwrap_or(0);
        let dir_prefix = &existing_token[..=last_slash];

        let mut replacement = format!("{dir_prefix}{}", candidate.name);
        let is_dir = candidate.is_dir;
        if is_dir {
            replacement.push('/');
        }

        self.content
            .replace_range(replace_range.clone(), &replacement);
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
#[path = "input_bar_tests.rs"]
mod tests;
