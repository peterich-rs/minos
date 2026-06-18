use crate::render::Renderable;
use minos_domain::AgentName;
use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthChar;

use super::theme::{ASSISTANT_LABEL, BORDER_FG, REASONING_STYLE, USER_LABEL};
use super::GroupChatState;

#[derive(Default)]
pub struct GroupChatRenderCache {
    indexed_version: u64,
    indexed_width: u16,
    segments: Vec<GroupChatCachedSegment>,
    total_lines: usize,
}

#[derive(Clone)]
struct GroupChatCachedSegment {
    start: usize,
    lines: Vec<Line<'static>>,
}

pub struct GroupChatRenderable<'a> {
    title: String,
    state: &'a mut GroupChatState,
    focused: bool,
}

impl<'a> GroupChatRenderable<'a> {
    pub fn new(title: String, state: &'a mut GroupChatState, focused: bool) -> Self {
        Self {
            title,
            state,
            focused,
        }
    }
}

impl Renderable for GroupChatRenderable<'_> {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        render_group_chat(f, area, self.title.as_str(), self.state, self.focused);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        u16::try_from(self.state.messages.len().saturating_add(2)).unwrap_or(u16::MAX)
    }
}

impl GroupChatRenderCache {
    pub fn is_valid(&self, version: u64, width: u16) -> bool {
        self.indexed_version == version && self.indexed_width == width
    }

    pub fn rebuild_if_stale(
        &mut self,
        messages: &[LocalGroupChatMessage],
        version: u64,
        width: u16,
    ) {
        if self.is_valid(version, width) {
            return;
        }
        self.rebuild(messages, width);
        self.indexed_version = version;
        self.indexed_width = width;
    }

    fn rebuild(&mut self, messages: &[LocalGroupChatMessage], width: u16) {
        self.segments.clear();
        self.segments.reserve(messages.len());
        let mut current_start = 0usize;

        for (index, message) in messages.iter().enumerate() {
            let lines = build_message_segment(index, message, width);
            let segment_len = lines.len();
            self.segments.push(GroupChatCachedSegment {
                start: current_start,
                lines,
            });
            current_start = current_start.saturating_add(segment_len);
        }

        self.total_lines = current_start;
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
}

pub fn render_group_chat(
    f: &mut Frame,
    area: Rect,
    title: &str,
    state: &mut GroupChatState,
    focused: bool,
) {
    let _span =
        tracing::trace_span!("render_group_chat", message_count = state.messages.len(),).entered();

    let block = super::theme::border_block()
        .title(title)
        .border_style(if focused {
            super::theme::FOCUSED_BORDER
        } else {
            ratatui::style::Style::new().fg(BORDER_FG)
        });
    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        f.render_widget(block, area);
        return;
    }

    if state.messages.is_empty() {
        state.update_max_scroll(0);
        let paragraph = Paragraph::new(vec![Line::from(Span::styled(
            "No group messages yet.",
            REASONING_STYLE,
        ))])
        .block(block);
        f.render_widget(paragraph, area);
        return;
    }

    state
        .render_cache
        .rebuild_if_stale(state.messages.as_slice(), state.version, inner.width);
    let max_scroll = state
        .render_cache
        .total_lines
        .saturating_sub(usize::from(inner.height))
        .min(usize::from(u16::MAX)) as u16;
    state.update_max_scroll(max_scroll);
    let visible_lines = state.render_cache.visible_lines(
        usize::from(state.active_scroll()),
        usize::from(inner.height),
    );

    let paragraph = Paragraph::new(visible_lines).block(block);
    f.render_widget(paragraph, area);
}

fn build_message_segment(
    index: usize,
    message: &LocalGroupChatMessage,
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if index > 0 {
        lines.push(Line::from(""));
    }
    let (label, style) = label_for_message(message);
    lines.push(Line::from(Span::styled(label, style)));
    for raw_line in message.text.split('\n') {
        lines.extend(wrap_plain_line(raw_line, width));
    }
    lines
}

fn label_for_message(message: &LocalGroupChatMessage) -> (String, ratatui::style::Style) {
    match message.kind {
        LocalGroupChatMessageKind::User => ("[You]".into(), USER_LABEL),
        LocalGroupChatMessageKind::AgentResult => {
            let agent = message.agent.map(agent_display).unwrap_or("Agent");
            let short_id = message
                .thread_short_id
                .as_deref()
                .or(message.thread_id.as_deref())
                .map(short_thread_id);
            let label = match short_id {
                Some(short_id) => format!("[{agent}@{short_id}]"),
                None => format!("[{agent}]"),
            };
            (label, ASSISTANT_LABEL)
        }
    }
}

fn agent_display(agent: AgentName) -> &'static str {
    match agent {
        AgentName::Codex => "Codex",
        AgentName::Claude => "Claude",
        AgentName::Gemini => "Gemini",
        AgentName::Opencode => "Opencode",
    }
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id[..8.min(thread_id.len())].to_owned()
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

    fn test_message(seq: u64, text: &str) -> LocalGroupChatMessage {
        LocalGroupChatMessage {
            seq,
            message_id: format!("m{seq}"),
            created_at_ms: seq as i64,
            kind: LocalGroupChatMessageKind::User,
            text: text.into(),
            agent: None,
            thread_id: None,
            thread_short_id: None,
            workspace: None,
        }
    }

    #[test]
    fn labels_agent_result_with_agent_and_short_thread() {
        let message = LocalGroupChatMessage {
            seq: 1,
            message_id: "m1".into(),
            created_at_ms: 10,
            kind: LocalGroupChatMessageKind::AgentResult,
            text: "done".into(),
            agent: Some(AgentName::Gemini),
            thread_id: Some("abcdef123456".into()),
            thread_short_id: Some("abcdef12".into()),
            workspace: None,
        };

        let (label, _) = label_for_message(&message);

        assert_eq!(label, "[Gemini@abcdef12]");
    }

    #[test]
    fn render_cache_indexes_message_starts() {
        let mut cache = GroupChatRenderCache::default();
        let messages = vec![test_message(1, "hello"), test_message(2, "world")];

        cache.rebuild_if_stale(messages.as_slice(), 1, 80);

        assert_eq!(cache.total_lines, 5);
        assert_eq!(
            cache
                .segments
                .iter()
                .map(|segment| segment.start)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn visible_lines_slice_from_cached_segments() {
        let mut cache = GroupChatRenderCache::default();
        let messages = vec![
            test_message(1, "aaa"),
            test_message(2, "bbb"),
            test_message(3, "ccc"),
        ];
        cache.rebuild_if_stale(messages.as_slice(), 1, 80);

        let lines = cache.visible_lines(3, 2);
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
            .collect::<Vec<_>>();

        assert_eq!(text, vec!["[You]", "bbb"]);
    }
}
