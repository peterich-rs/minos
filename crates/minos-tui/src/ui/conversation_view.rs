use crate::agent_route::short_thread_id;
use crate::backend::ConversationMessageEntry;
use crate::render::Renderable;
use crate::translation::ChatSelection;
use crate::ui::theme::{self, ASSISTANT_LABEL, BORDER_FG, REASONING_STYLE, USER_LABEL};
use minos_domain::AgentName;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::collections::HashMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Max plain-text characters kept from a replied-to body for the quote preview.
const REPLY_PREVIEW_MAX_CHARS: usize = 160;
/// Max visual lines for a reply quote block (excluding the author line).
const REPLY_PREVIEW_MAX_BODY_LINES: usize = 2;

pub struct ConversationChatRenderable<'a> {
    title: String,
    messages: &'a [ConversationMessageEntry],
    messages_revision: u64,
    scroll_offset: &'a mut u32,
    auto_scroll: &'a mut bool,
    max_scroll: &'a mut u32,
    cache: &'a mut ConversationChatRenderCache,
    selection: Option<&'a ChatSelection>,
    focused: bool,
}

impl<'a> ConversationChatRenderable<'a> {
    pub fn new(
        title: String,
        messages: &'a [ConversationMessageEntry],
        messages_revision: u64,
        scroll_offset: &'a mut u32,
        auto_scroll: &'a mut bool,
        max_scroll: &'a mut u32,
        cache: &'a mut ConversationChatRenderCache,
        selection: Option<&'a ChatSelection>,
        focused: bool,
    ) -> Self {
        Self {
            title,
            messages,
            messages_revision,
            scroll_offset,
            auto_scroll,
            max_scroll,
            cache,
            selection,
            focused,
        }
    }
}

impl Renderable for ConversationChatRenderable<'_> {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        let border_style = if self.focused {
            theme::FOCUSED_BORDER
        } else {
            Style::new().fg(BORDER_FG)
        };
        let block = theme::border_block()
            .title(self.title.clone())
            .border_style(border_style);
        let inner = block.inner(area);
        if inner.width == 0 || inner.height == 0 {
            f.render_widget(block, area);
            return;
        }

        if self.messages.is_empty() {
            *self.max_scroll = 0;
            *self.scroll_offset = 0;
            *self.auto_scroll = true;
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "No messages yet. Type below to start the conversation.",
                    REASONING_STYLE,
                )))
                .block(block),
                area,
            );
            return;
        }

        // Revision is O(1); avoids hashing full message bodies every scroll frame.
        self.cache
            .rebuild_if_stale(self.messages, inner.width, self.messages_revision);
        *self.max_scroll = self
            .cache
            .total_lines
            .saturating_sub(usize::from(inner.height)) as u32;
        if *self.auto_scroll {
            *self.scroll_offset = *self.max_scroll;
        } else {
            *self.scroll_offset = (*self.scroll_offset).min(*self.max_scroll);
        }

        let base_row = *self.scroll_offset as usize;
        let mut visible = self
            .cache
            .visible_lines(base_row, usize::from(inner.height));
        apply_selection_highlight(visible.as_mut_slice(), self.selection, base_row);
        f.render_widget(Paragraph::new(visible).block(block), area);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

/// Full markdown layout retained only for the last N conversation messages.
const FULL_RENDER_TAIL_MESSAGES: usize = 256;

#[derive(Default)]
pub struct ConversationChatRenderCache {
    indexed_count: usize,
    indexed_width: u16,
    /// Panel-side revision: O(1) scroll-path validity without hashing bodies.
    indexed_revision: u64,
    /// Content fingerprint so reply previews rebuild when parent bodies change
    /// without a message-count change.
    indexed_fingerprint: u64,
    /// Fingerprint of `messages[..indexed_count.saturating_sub(1)]` when count > 0 —
    /// used to detect last-message-only updates and append-only growth.
    indexed_prefix_fingerprint: u64,
    segments: Vec<CachedSegment>,
    total_lines: usize,
}

#[derive(Clone)]
struct CachedSegment {
    start: usize,
    lines: Vec<Line<'static>>,
}

impl ConversationChatRenderCache {
    /// Ensure the layout cache matches `messages` / width / revision, then
    /// extract the currently selected plain text (full rebuild of visible
    /// selection range from cached lines — copy is infrequent).
    pub fn selected_text(
        &mut self,
        messages: &[ConversationMessageEntry],
        width: u16,
        revision: u64,
        selection: &ChatSelection,
    ) -> Option<String> {
        if selection.is_empty() || width == 0 {
            return None;
        }
        self.rebuild_if_stale(messages, width, revision);
        selected_text_from_cache(self, selection)
    }

    fn is_valid(&self, count: usize, width: u16, revision: u64) -> bool {
        self.indexed_count == count
            && self.indexed_width == width
            && self.indexed_revision == revision
    }

    fn rebuild_if_stale(
        &mut self,
        messages: &[ConversationMessageEntry],
        width: u16,
        revision: u64,
    ) {
        if self.is_valid(messages.len(), width, revision) {
            return;
        }

        let fingerprint = messages_fingerprint(messages);
        if width == self.indexed_width && !self.segments.is_empty() {
            // Append-only: prefix unchanged.
            if messages.len() > self.indexed_count {
                let prefix_fp = messages_fingerprint(&messages[..self.indexed_count]);
                if prefix_fp == self.indexed_fingerprint {
                    self.append_messages(messages, width);
                    self.indexed_count = messages.len();
                    self.indexed_width = width;
                    self.indexed_revision = revision;
                    self.indexed_fingerprint = fingerprint;
                    self.indexed_prefix_fingerprint = prefix_fingerprint(messages);
                    return;
                }
            }
            // Last message body updated in place (same count).
            if messages.len() == self.indexed_count && !messages.is_empty() {
                let without_last = messages_fingerprint(&messages[..messages.len() - 1]);
                if without_last == self.indexed_prefix_fingerprint {
                    self.rebuild_last_message(messages, width);
                    self.indexed_revision = revision;
                    self.indexed_fingerprint = fingerprint;
                    return;
                }
            }
        }

        self.rebuild(messages, width);
        self.indexed_count = messages.len();
        self.indexed_width = width;
        self.indexed_revision = revision;
        self.indexed_fingerprint = fingerprint;
        self.indexed_prefix_fingerprint = prefix_fingerprint(messages);
    }

    fn rebuild(&mut self, messages: &[ConversationMessageEntry], width: u16) {
        self.segments.clear();
        self.segments.reserve(messages.len());
        let by_id = message_index(messages);
        let mut current_start = 0usize;
        let full_from = messages.len().saturating_sub(FULL_RENDER_TAIL_MESSAGES);
        for (index, message) in messages.iter().enumerate() {
            let lines = if index < full_from {
                conversation_placeholder_lines(index, width)
            } else {
                build_message_segment(index, message, width, &by_id)
            };
            let segment_len = lines.len();
            self.segments.push(CachedSegment {
                start: current_start,
                lines,
            });
            current_start = current_start.saturating_add(segment_len);
        }
        self.total_lines = current_start;
    }

    fn append_messages(&mut self, messages: &[ConversationMessageEntry], width: u16) {
        let by_id = message_index(messages);
        let mut current_start = self.total_lines;
        for (index, message) in messages.iter().enumerate().skip(self.indexed_count) {
            let lines = build_message_segment(index, message, width, &by_id);
            let segment_len = lines.len();
            self.segments.push(CachedSegment {
                start: current_start,
                lines,
            });
            current_start = current_start.saturating_add(segment_len);
        }
        self.total_lines = current_start;
    }

    fn rebuild_last_message(&mut self, messages: &[ConversationMessageEntry], width: u16) {
        let last = messages.len() - 1;
        let by_id = message_index(messages);
        let lines = build_message_segment(last, &messages[last], width, &by_id);
        let start = if last == 0 {
            0
        } else {
            self.segments[last - 1].start + self.segments[last - 1].lines.len()
        };
        if last < self.segments.len() {
            self.segments[last] = CachedSegment { start, lines };
        } else {
            self.segments.push(CachedSegment { start, lines });
        }
        self.total_lines = start + self.segments[last].lines.len();
    }

    fn visible_lines(&self, base_row: usize, height: usize) -> Vec<Line<'static>> {
        if self.segments.is_empty() || height == 0 {
            return Vec::new();
        }
        let end_row = base_row.saturating_add(height);
        let start_index = self
            .segments
            .partition_point(|segment| segment.start <= base_row);
        let start_index = start_index.saturating_sub(1);
        let end_index = self
            .segments
            .partition_point(|segment| segment.start < end_row);
        let end_index = end_index.max(start_index + 1).min(self.segments.len());

        let mut lines = Vec::with_capacity(height);
        for segment in &self.segments[start_index..end_index] {
            for (line_index, line) in segment.lines.iter().enumerate() {
                let absolute_row = segment.start + line_index;
                if absolute_row < base_row {
                    continue;
                }
                if absolute_row >= end_row {
                    return lines;
                }
                lines.push(line.clone());
                if lines.len() >= height {
                    return lines;
                }
            }
        }
        lines
    }

    fn line_at(&self, absolute_row: usize) -> Option<&Line<'static>> {
        if self.segments.is_empty() || absolute_row >= self.total_lines {
            return None;
        }
        let start_index = self
            .segments
            .partition_point(|segment| segment.start <= absolute_row)
            .saturating_sub(1);
        let segment = self.segments.get(start_index)?;
        let line_index = absolute_row.checked_sub(segment.start)?;
        segment.lines.get(line_index)
    }
}

fn message_index(
    messages: &[ConversationMessageEntry],
) -> HashMap<&str, &ConversationMessageEntry> {
    let mut map = HashMap::with_capacity(messages.len());
    for message in messages {
        map.insert(message.message_id.as_str(), message);
    }
    map
}

fn conversation_placeholder_lines(index: usize, _width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if index > 0 {
        // Match conversation/agent chat: blank gap, not a full-width rule.
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("…", REASONING_STYLE)));
    lines
}

fn prefix_fingerprint(messages: &[ConversationMessageEntry]) -> u64 {
    if messages.is_empty() {
        return 0;
    }
    messages_fingerprint(&messages[..messages.len().saturating_sub(1)])
}

fn messages_fingerprint(messages: &[ConversationMessageEntry]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    messages.len().hash(&mut hasher);
    for message in messages {
        message.message_id.hash(&mut hasher);
        message.message_seq.hash(&mut hasher);
        message.sender_role.hash(&mut hasher);
        message.body.hash(&mut hasher);
        message.reply_to_message_id.hash(&mut hasher);
        message.delegation_id.hash(&mut hasher);
        message.thread_id.hash(&mut hasher);
    }
    hasher.finish()
}

fn build_message_segment(
    index: usize,
    message: &ConversationMessageEntry,
    width: u16,
    by_id: &HashMap<&str, &ConversationMessageEntry>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if index > 0 {
        lines.push(Line::from(""));
    }
    let (label, style) = label_for_message(message);
    lines.push(Line::from(Span::styled(label, style)));
    if let Some(reply_to) = message.reply_to_message_id.as_deref() {
        lines.extend(reply_preview_lines(
            by_id.get(reply_to).copied(),
            reply_to,
            width,
        ));
    }
    for raw_line in message.body.split('\n') {
        lines.extend(wrap_body_line_with_mentions(raw_line, width));
    }
    lines
}

/// Build a quoted reply block for the parent message when available.
fn reply_preview_lines(
    parent: Option<&ConversationMessageEntry>,
    reply_to_id: &str,
    width: u16,
) -> Vec<Line<'static>> {
    let Some(parent) = parent else {
        return vec![Line::from(Span::styled(
            format!("  ↳ (reply unavailable · {reply_to_id})"),
            REASONING_STYLE,
        ))];
    };

    let (author, _) = label_for_message(parent);
    let preview = collapse_reply_body(&parent.body);
    let header = format!("  ↳ {author}");
    let mut lines = vec![Line::from(Span::styled(header, REASONING_STYLE))];

    if preview.is_empty() {
        return lines;
    }

    // Reserve space for "  │ " prefix when wrapping quote body.
    let body_width = width.saturating_sub(4).max(8);
    let mut body_lines = 0usize;
    for raw_line in preview.split('\n') {
        if body_lines >= REPLY_PREVIEW_MAX_BODY_LINES {
            break;
        }
        for wrapped in wrap_plain_line(raw_line, body_width) {
            if body_lines >= REPLY_PREVIEW_MAX_BODY_LINES {
                break;
            }
            let text = line_to_plain(&wrapped);
            lines.push(Line::from(Span::styled(
                format!("  │ {text}"),
                REASONING_STYLE,
            )));
            body_lines = body_lines.saturating_add(1);
        }
    }
    lines
}

fn collapse_reply_body(body: &str) -> String {
    let collapsed = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    truncate_chars(&collapsed, REPLY_PREVIEW_MAX_CHARS)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out = text
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

fn line_to_plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn apply_selection_highlight(
    lines: &mut [Line<'static>],
    selection: Option<&ChatSelection>,
    base_row: usize,
) {
    let Some(selection) = selection.filter(|selection| !selection.is_empty()) else {
        return;
    };
    for (local_row, line) in lines.iter_mut().enumerate() {
        let absolute_row = base_row + local_row;
        let plain = line_to_plain(line);
        if let Some((start_col, end_col)) = selected_cols_for_row(selection, absolute_row, &plain) {
            *line = highlight_line(std::mem::take(line), start_col, end_col);
        }
    }
}

fn selected_text_from_cache(
    cache: &ConversationChatRenderCache,
    selection: &ChatSelection,
) -> Option<String> {
    let (start, end) = selection.normalized();
    if start.row >= cache.total_lines {
        return None;
    }
    let last_row = end.row.min(cache.total_lines.saturating_sub(1));
    let mut selected = Vec::new();
    for row in start.row..=last_row {
        let Some(line) = cache.line_at(row) else {
            selected.push(String::new());
            continue;
        };
        let plain = line_to_plain(line);
        if let Some((start_col, end_col)) = selected_cols_for_row(selection, row, &plain) {
            selected.push(cell_slice(&plain, start_col, end_col));
        } else {
            selected.push(String::new());
        }
    }
    let text = selected.join("\n");
    (!text.is_empty()).then_some(text)
}

fn selected_cols_for_row(
    selection: &ChatSelection,
    row: usize,
    line_text: &str,
) -> Option<(usize, usize)> {
    let (start, end) = selection.normalized();
    if row < start.row || row > end.row {
        return None;
    }
    let line_width = UnicodeWidthStr::width(line_text);
    let start_col = if row == start.row { start.col } else { 0 };
    let end_col = if row == end.row {
        end.col.saturating_add(1)
    } else {
        line_width
    }
    .min(line_width);
    (start_col < end_col).then_some((start_col, end_col))
}

fn highlight_line(line: Line<'static>, start_col: usize, end_col: usize) -> Line<'static> {
    let mut out = Line::default();
    let mut col = 0usize;
    for span in line.spans {
        let base_style = span.style;
        let mut plain = String::new();
        let mut selected = String::new();
        let mut selected_mode = None;

        for ch in span.content.into_owned().chars() {
            let ch_width = char_display_width(ch);
            let ch_end = col.saturating_add(ch_width);
            let is_selected = ch_end > start_col && col < end_col;
            match selected_mode {
                Some(mode) if mode == is_selected => {}
                Some(true) => {
                    push_span(&mut out, &mut selected, selection_style(base_style));
                    selected_mode = Some(is_selected);
                }
                Some(false) => {
                    push_span(&mut out, &mut plain, base_style);
                    selected_mode = Some(is_selected);
                }
                None => selected_mode = Some(is_selected),
            }
            if is_selected {
                selected.push(ch);
            } else {
                plain.push(ch);
            }
            col = ch_end;
        }
        match selected_mode {
            Some(true) => push_span(&mut out, &mut selected, selection_style(base_style)),
            Some(false) => push_span(&mut out, &mut plain, base_style),
            None => {}
        }
    }
    out
}

fn push_span(line: &mut Line<'static>, buf: &mut String, style: Style) {
    if buf.is_empty() {
        return;
    }
    line.spans.push(Span::styled(std::mem::take(buf), style));
}

fn cell_slice(text: &str, start_col: usize, end_col: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        let ch_width = char_display_width(ch);
        let ch_end = col.saturating_add(ch_width);
        if ch_end > start_col && col < end_col {
            out.push(ch);
        }
        col = ch_end;
    }
    out
}

fn selection_style(style: Style) -> Style {
    style.bg(Color::Blue).fg(Color::White)
}

fn char_display_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0).max(1)
}

fn wrap_body_line_with_mentions(raw: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    if raw.is_empty() {
        return vec![Line::from("")];
    }
    let spans = highlight_mentions_in_line(raw);
    wrap_spans(spans, width)
}

fn highlight_mentions_in_line(raw: &str) -> Vec<Span<'static>> {
    let mention_style = Style::new().fg(theme::CLI_OK.fg.unwrap_or(ratatui::style::Color::Cyan));
    let mut spans = Vec::new();
    let mut rest = raw;
    while let Some(at) = rest.find('@') {
        if at > 0 {
            spans.push(Span::raw(rest[..at].to_owned()));
        }
        let after = &rest[at + 1..];
        let token_end = after
            .find(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | ')' | ']'))
            .unwrap_or(after.len());
        let token = &after[..token_end];
        if looks_like_agent_mention(token) {
            spans.push(Span::styled(format!("@{token}"), mention_style));
            rest = &after[token_end..];
        } else {
            spans.push(Span::raw("@".to_owned()));
            rest = after;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::raw(rest.to_owned()));
    }
    if spans.is_empty() {
        spans.push(Span::raw(raw.to_owned()));
    }
    spans
}

fn looks_like_agent_mention(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let (agent, short) = match token.split_once('#') {
        Some((agent, short)) if !short.is_empty() => (agent, Some(short)),
        Some(_) => return false,
        None => (token, None),
    };
    let agent_ok = matches!(
        agent.to_ascii_lowercase().as_str(),
        "codex" | "claude" | "gemini" | "opencode" | "grok"
    );
    if !agent_ok {
        return false;
    }
    short.is_none_or(|s| s.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-'))
}

fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut current_width = 0usize;
    for span in spans {
        let text = span.content.clone().into_owned();
        let style = span.style;
        for ch in text.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width > 0 && ch_width > 0 && current_width + ch_width > width {
                lines.push(Line::from(std::mem::take(&mut current)));
                current_width = 0;
            }
            if let Some(last) = current.last_mut() {
                if last.style == style {
                    last.content.to_mut().push(ch);
                    current_width = current_width.saturating_add(ch_width);
                    continue;
                }
            }
            current.push(Span::styled(ch.to_string(), style));
            current_width = current_width.saturating_add(ch_width);
        }
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
}

fn label_for_message(message: &ConversationMessageEntry) -> (String, Style) {
    if message.sender_role == "user" {
        return ("[You]".into(), USER_LABEL);
    }
    let agent = message.agent.map(agent_display).unwrap_or("Agent");
    let short_id = message.thread_id.as_deref().map(short_thread_id);
    let label = match short_id {
        Some(short_id) => format!("[{agent}@{short_id}]"),
        None => format!("[{agent}]"),
    };
    (label, ASSISTANT_LABEL)
}

fn agent_display(agent: AgentName) -> &'static str {
    match agent {
        AgentName::Codex => "Codex",
        AgentName::Claude => "Claude",
        AgentName::Gemini => "Gemini",
        AgentName::Opencode => "Opencode",
        AgentName::Grok => "Grok",
    }
}

fn wrap_plain_line(raw: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    if raw.is_empty() {
        return vec![Line::from("")];
    }
    let mut lines = Vec::new();
    let mut buf = String::new();
    let mut current_width = 0usize;
    for ch in raw.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width > 0 && ch_width > 0 && current_width + ch_width > width {
            lines.push(Line::from(std::mem::take(&mut buf)));
            current_width = 0;
        }
        buf.push(ch);
        current_width = current_width.saturating_add(ch_width);
    }
    if !buf.is_empty() {
        lines.push(Line::from(buf));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_message(seq: i64, body: &str) -> ConversationMessageEntry {
        ConversationMessageEntry {
            message_seq: seq,
            message_id: format!("u{seq}"),
            conversation_id: "c1".into(),
            thread_id: None,
            created_at_ms: seq,
            sender_role: "user".into(),
            agent: None,
            body: body.into(),
            reply_to_message_id: None,
            delegation_id: None,
            mentions: Vec::new(),
        }
    }

    fn agent_message(
        seq: i64,
        body: &str,
        agent: AgentName,
        thread_id: &str,
    ) -> ConversationMessageEntry {
        ConversationMessageEntry {
            message_seq: seq,
            message_id: format!("a{seq}"),
            conversation_id: "c1".into(),
            thread_id: Some(thread_id.into()),
            created_at_ms: seq,
            sender_role: "agent".into(),
            agent: Some(agent),
            body: body.into(),
            reply_to_message_id: None,
            delegation_id: None,
            mentions: Vec::new(),
        }
    }

    #[test]
    fn label_for_user_message() {
        let msg = user_message(1, "hello");
        let (label, _) = label_for_message(&msg);
        assert_eq!(label, "[You]");
    }

    #[test]
    fn label_for_agent_message_includes_short_thread() {
        let msg = agent_message(2, "done", AgentName::Gemini, "abcdef1234567890");
        let (label, _) = label_for_message(&msg);
        assert_eq!(label, "[Gemini@abcdef12]");
    }

    #[test]
    fn reply_preview_shows_author_and_body_when_parent_loaded() {
        let parent = agent_message(
            1,
            "please inspect auth\nand report back",
            AgentName::Codex,
            "thread-codex-1234",
        );
        let lines = reply_preview_lines(Some(&parent), &parent.message_id, 80);
        let plain: Vec<String> = lines.iter().map(line_to_plain).collect();
        assert_eq!(plain[0], "  ↳ [Codex@thread-c]");
        assert!(plain
            .iter()
            .any(|line| line.contains("please inspect auth")));
        assert!(plain.iter().any(|line| line.starts_with("  │ ")));
    }

    #[test]
    fn reply_preview_falls_back_when_parent_missing() {
        let lines = reply_preview_lines(None, "missing-id", 80);
        let plain = line_to_plain(&lines[0]);
        assert!(plain.contains("reply unavailable"));
        assert!(plain.contains("missing-id"));
    }

    #[test]
    fn reply_preview_truncates_long_parent_body() {
        let long = "x".repeat(REPLY_PREVIEW_MAX_CHARS + 40);
        let parent = user_message(1, &long);
        let lines = reply_preview_lines(Some(&parent), &parent.message_id, 40);
        let body_lines: Vec<_> = lines
            .iter()
            .map(line_to_plain)
            .filter(|line| line.starts_with("  │ "))
            .collect();
        assert!(!body_lines.is_empty());
        let joined = body_lines.join("");
        assert!(joined.contains('…') || joined.chars().count() <= REPLY_PREVIEW_MAX_CHARS + 20);
    }

    #[test]
    fn render_cache_rebuilds_when_reply_parent_body_changes() {
        let mut cache = ConversationChatRenderCache::default();
        let mut parent = user_message(1, "old prompt");
        let mut reply = agent_message(2, "result", AgentName::Codex, "thread-1");
        reply.reply_to_message_id = Some(parent.message_id.clone());
        let messages = vec![parent.clone(), reply.clone()];
        cache.rebuild_if_stale(&messages, 80, 1);
        let first_fp = cache.indexed_fingerprint;

        parent.body = "new prompt that should refresh quote".into();
        let messages = vec![parent, reply];
        cache.rebuild_if_stale(&messages, 80, 2);
        assert_ne!(cache.indexed_fingerprint, first_fp);
        let quote = cache
            .segments
            .iter()
            .flat_map(|segment| segment.lines.iter())
            .map(line_to_plain)
            .find(|line| line.contains("new prompt"))
            .expect("quote should include updated parent body");
        assert!(quote.starts_with("  │ ") || quote.contains("new prompt"));
    }

    #[test]
    fn render_cache_indexes_message_starts() {
        let mut cache = ConversationChatRenderCache::default();
        let messages = vec![user_message(1, "hello"), user_message(2, "world")];
        cache.rebuild_if_stale(&messages, 80, 1);
        // msg1: [You]+hello = 2 lines (rows 0,1); msg2: ""+[You]+world = 3 lines (rows 2,3,4)
        assert_eq!(cache.total_lines, 5);
        assert_eq!(
            cache.segments.iter().map(|s| s.start).collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn visible_lines_slice_from_cached_segments() {
        let mut cache = ConversationChatRenderCache::default();
        let messages = vec![
            user_message(1, "aaa"),
            user_message(2, "bbb"),
            user_message(3, "ccc"),
        ];
        cache.rebuild_if_stale(&messages, 80, 1);
        // msg1: rows 0,1 ([You], aaa)
        // msg2: rows 2,3,4 ("", [You], bbb)
        // msg3: rows 5,6,7 ("", [You], ccc)
        // base_row=4 height=2 → rows 4,5 = "bbb", separator("")
        let lines = cache.visible_lines(4, 2);
        assert_eq!(lines.len(), 2);
        // row 4 = "bbb" (single span)
        let row4_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(row4_text, "bbb");
        // row 5 = separator (empty line)
        assert!(lines[1].spans.is_empty());
    }

    #[test]
    fn rebuild_if_stale_skips_when_valid() {
        let mut cache = ConversationChatRenderCache::default();
        let messages = vec![user_message(1, "hi")];
        cache.rebuild_if_stale(&messages, 80, 1);
        let segments_before = cache.segments.len();
        // Same revision: must not rehash / rebuild.
        cache.rebuild_if_stale(&messages, 80, 1);
        assert_eq!(cache.segments.len(), segments_before);
    }

    #[test]
    fn rebuild_if_stale_skips_body_hash_on_same_revision() {
        let mut cache = ConversationChatRenderCache::default();
        let mut messages = vec![user_message(1, "hi")];
        cache.rebuild_if_stale(&messages, 80, 7);
        let lines_before = cache.total_lines;
        // Mutate body without bumping revision (callers must bump; cache trusts it).
        messages[0].body = "changed without revision bump".into();
        cache.rebuild_if_stale(&messages, 80, 7);
        assert_eq!(cache.total_lines, lines_before);
    }

    #[test]
    fn selected_text_extracts_range_from_cached_lines() {
        use crate::translation::{ChatSelection, ChatSelectionPoint};

        let mut cache = ConversationChatRenderCache::default();
        let messages = vec![user_message(1, "hello\nworld")];
        // rows: 0=[You], 1=hello, 2=world
        let selection = ChatSelection {
            anchor: ChatSelectionPoint { row: 1, col: 0 },
            focus: ChatSelectionPoint { row: 2, col: 4 },
        };
        let text = cache
            .selected_text(&messages, 80, 1, &selection)
            .expect("selected text");
        assert!(text.contains("hello"), "got {text:?}");
        assert!(text.contains("world"), "got {text:?}");
    }
}
