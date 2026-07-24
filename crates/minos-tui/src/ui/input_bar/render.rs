use std::ops::Range;

use crate::render::Renderable;
use crate::ui::theme::{
    border_block, BORDER_FG, FOCUSED_BORDER, HIGHLIGHTED, INPUT_PROMPT, REASONING_STYLE,
};
use minos_domain::AgentStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthChar;

use super::{
    AgentMentionCandidate, AgentMentionCandidateKind, CursorStyle, InputPicker, InputState,
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
    #[allow(dead_code)]
    pub visible_rows: usize,
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

pub struct InputBarRenderable<'a> {
    title: &'a str,
    empty_hint: &'a str,
    state: &'a InputState,
    candidates: &'a [AgentMentionCandidate],
    metrics: &'a mut InputLayoutMetrics,
}

impl<'a> InputBarRenderable<'a> {
    pub fn new(
        title: &'a str,
        empty_hint: &'a str,
        state: &'a InputState,
        candidates: &'a [AgentMentionCandidate],
        metrics: &'a mut InputLayoutMetrics,
    ) -> Self {
        Self {
            title,
            empty_hint,
            state,
            candidates,
            metrics,
        }
    }
}

impl Renderable for InputBarRenderable<'_> {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        render_input_bar(
            f,
            area,
            self.title,
            self.empty_hint,
            self.state,
            self.candidates,
            self.metrics,
        );
    }

    fn desired_height(&self, width: u16) -> u16 {
        required_height(self.state, width)
    }

    fn cursor_pos(&self, _area: Rect) -> Option<(u16, u16)> {
        if !self.state.focused || self.metrics.width == 0 {
            return None;
        }

        let (row, col) = visual_cursor_position(
            self.state.content.as_str(),
            self.state.cursor_pos,
            self.metrics.width,
        );
        let row = row.saturating_sub(self.metrics.start_row);
        if row >= self.metrics.visible_rows {
            return None;
        }

        let x = self.metrics.editor_area.x.saturating_add(
            u16::try_from(col)
                .unwrap_or(u16::MAX)
                .min(self.metrics.width.saturating_sub(1)),
        );
        let y = self
            .metrics
            .editor_area
            .y
            .saturating_add(u16::try_from(row).unwrap_or(u16::MAX));
        Some((x, y))
    }
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

pub(super) fn agent_picker_status_label(
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
        AgentMentionCandidateKind::Profile { .. } => Some((
            format!("profile · {}", candidate.agent.bin_name()),
            ratatui::style::Style::new().fg(ratatui::style::Color::Cyan),
        )),
        AgentMentionCandidateKind::Existing { .. } => None,
    }
}

pub(super) fn active_agent_range(content: &str, cursor_pos: usize) -> Option<Range<usize>> {
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
        return EditorLines {
            lines,
            cursor_row: 0,
        };
    }

    // Focused with content: wrap text, then apply cursor style.
    let cursor_row = wrapped_row_for_cursor(state.content.as_str(), state.cursor_pos, width);
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

fn visual_cursor_position(content: &str, cursor_pos: usize, width: u16) -> (usize, usize) {
    let width = usize::from(width.max(1));
    let mut row = 0usize;
    let mut col_width = 0usize;

    for (index, ch) in content.char_indices() {
        if index == cursor_pos {
            return (row, col_width);
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

    (row, col_width)
}

/// Returns the index of the last visual row for `content` at `width`, i.e. the
/// row containing the final character (or `0` for empty/single-line input).
/// Used to detect when the cursor is on the bottom row so ↓ can fall through
/// to history navigation.
pub fn last_visual_row(content: &str, width: u16) -> usize {
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
