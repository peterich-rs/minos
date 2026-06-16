use crate::translation::{ChatItem, ChatSelection, ChatState, TextPart};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::theme::{
    ASSISTANT_LABEL, BORDER_FG, ERROR_STYLE, FOCUSED_BORDER, REASONING_STYLE, STREAMING_CURSOR,
    TOOL_ERROR, TOOL_NAME_STYLE, TOOL_SUCCESS, USER_LABEL,
};

trait LineSink {
    fn push_line(&mut self, line: Line<'static>);
}

struct VecSink(Vec<Line<'static>>);
impl LineSink for VecSink {
    fn push_line(&mut self, line: Line<'static>) {
        self.0.push(line);
    }
}

struct VecSinkRef<'a>(&'a mut Vec<Line<'static>>);
impl<'a> LineSink for VecSinkRef<'a> {
    fn push_line(&mut self, line: Line<'static>) {
        self.0.push(line);
    }
}

struct CountingSink {
    width: u16,
    count: usize,
}
impl LineSink for CountingSink {
    fn push_line(&mut self, line: Line<'static>) {
        self.count += visual_line_count(&line, self.width);
    }
}

fn visual_line_count(line: &Line<'static>, width: u16) -> usize {
    let width = usize::from(width.max(1));
    let mut rows = 1usize;
    let mut current_width = 0usize;

    for span in &line.spans {
        for ch in span.content.chars() {
            let ch_width = char_width(ch);
            if current_width > 0 && ch_width > 0 && current_width + ch_width > width {
                rows += 1;
                current_width = 0;
            }
            current_width = current_width.saturating_add(ch_width);
        }
    }

    rows
}

pub fn render_chat(f: &mut Frame, area: Rect, chat: &mut ChatState, focused: bool) {
    let title = format!(
        "Chat: {} #{}{}",
        chat.agent.bin_name(),
        short_thread_id(&chat.thread_id),
        if chat.auto_scroll {
            ""
        } else {
            " [manual scroll]"
        }
    );
    let block = super::theme::border_block()
        .title(title)
        .border_style(if focused {
            FOCUSED_BORDER
        } else {
            ratatui::style::Style::new().fg(BORDER_FG)
        });
    let inner = block.inner(area);

    if inner.width == 0 || inner.height == 0 {
        f.render_widget(block, area);
        return;
    }

    let mut lines = visual_lines(build_lines(chat.items.as_slice(), inner.width), inner.width);
    let max_scroll = lines
        .len()
        .saturating_sub(usize::from(inner.height))
        .min(usize::from(u16::MAX)) as u16;
    chat.update_max_scroll(max_scroll);
    apply_selection(lines.as_mut_slice(), chat.selection.as_ref());
    let visible_lines: Vec<Line<'static>> = lines
        .into_iter()
        .skip(usize::from(chat.active_scroll()))
        .take(usize::from(inner.height))
        .map(|line| line.line)
        .collect();

    let paragraph = Paragraph::new(visible_lines).block(block);
    f.render_widget(paragraph, area);
}

pub fn selected_text(chat: &ChatState, width: u16) -> Option<String> {
    let selection = chat.selection.as_ref()?;
    if selection.is_empty() || width == 0 {
        return None;
    }

    let lines = visual_lines(build_lines(chat.items.as_slice(), width), width);
    selected_text_from_lines(lines.as_slice(), selection)
}

fn build_lines(items: &[ChatItem], separator_width: u16) -> Vec<Line<'static>> {
    let mut sink = VecSink(Vec::new());

    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            sink.push_line(separator_line(separator_width));
        }
        build_item_lines(&mut sink, item);
    }

    if sink.0.is_empty() {
        sink.push_line(Line::from(Span::styled(
            "No messages yet. Press `n` to start another agent, then type below.",
            REASONING_STYLE,
        )));
    }

    sink.0
}

fn separator_line(separator_width: u16) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(usize::from(separator_width.max(1))),
        ratatui::style::Style::new().fg(BORDER_FG),
    ))
}

fn build_item_lines<S: LineSink>(sink: &mut S, item: &ChatItem) {
    match item {
        ChatItem::UserMessage {
            text_parts,
            is_streaming,
            ..
        } => {
            sink.push_line(Line::from(Span::styled("[You]", USER_LABEL)));
            push_text_parts(sink, text_parts, Style::default());
            if *is_streaming {
                sink.push_line(Line::from(Span::styled("▓", STREAMING_CURSOR)));
            }
        }
        ChatItem::AssistantText {
            text_parts,
            is_streaming,
            ..
        } => {
            sink.push_line(Line::from(Span::styled("[Agent]", ASSISTANT_LABEL)));
            push_text_parts(sink, text_parts, Style::default());
            if *is_streaming {
                sink.push_line(Line::from(Span::styled("▓", STREAMING_CURSOR)));
            }
        }
        ChatItem::Reasoning {
            text, is_streaming, ..
        } => {
            sink.push_line(Line::from(Span::styled("Thinking", REASONING_STYLE)));
            push_markdown_lines(sink, text, REASONING_STYLE);
            if *is_streaming {
                sink.push_line(Line::from(Span::styled("▓", STREAMING_CURSOR)));
            }
        }
        ChatItem::ToolCall {
            name,
            args_summary,
            args_detail,
            output_summary,
            output_detail,
            is_error,
            is_expanded,
            is_streaming,
            ..
        } => {
            let status_label = if *is_streaming || output_summary.is_none() {
                Span::styled("running", ratatui::style::Style::default())
            } else if *is_error {
                Span::styled("failed", TOOL_ERROR)
            } else {
                Span::styled("done", TOOL_SUCCESS)
            };
            let mut tc_spans = vec![
                Span::raw("Tool "),
                Span::styled(name.clone(), TOOL_NAME_STYLE),
                Span::raw(" · "),
                status_label,
            ];
            if !args_summary.is_empty() {
                tc_spans.push(Span::raw(format!(" {}", args_summary)));
            }
            if *is_expanded {
                let mut emitted_detail = false;
                sink.push_line(Line::from(tc_spans.clone()));
                if let Some(args) = args_detail {
                    emitted_detail = true;
                    push_tool_detail_lines(sink, "args", args);
                }
                if let Some(output) = output_detail.as_ref().or(output_summary.as_ref()) {
                    emitted_detail = true;
                    push_tool_detail_lines(sink, "out", output);
                }
                if emitted_detail {
                    return;
                }
            }
            sink.push_line(Line::from(tc_spans));
        }
        ChatItem::SystemMessage { text } => {
            sink.push_line(Line::from(Span::styled("[System]", REASONING_STYLE)));
            push_markdown_lines(sink, text, Style::default());
        }
        ChatItem::Error { text, .. } => {
            sink.push_line(Line::from(Span::styled(text.clone(), ERROR_STYLE)));
        }
    }
}

fn push_text_parts<S: LineSink>(sink: &mut S, text_parts: &[TextPart], base_style: Style) {
    for part in text_parts {
        match part {
            TextPart::Plain(text) => {
                push_markdown_lines(sink, text, base_style);
            }
            TextPart::Code { lang, code } => {
                push_code_block(sink, lang, code);
            }
        }
    }
}

fn push_markdown_lines<S: LineSink>(sink: &mut S, text: &str, base_style: Style) {
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code = String::new();

    for raw_line in text.split('\n') {
        if let Some(lang) = raw_line.trim_start().strip_prefix("```") {
            if in_code {
                push_code_block(sink, &code_lang, code.trim_end_matches('\n'));
                code.clear();
                code_lang.clear();
                in_code = false;
            } else {
                in_code = true;
                code_lang = lang.trim().to_owned();
            }
            continue;
        }

        if in_code {
            code.push_str(raw_line);
            code.push('\n');
            continue;
        }

        sink.push_line(markdown_line(raw_line, base_style));
    }

    if in_code {
        push_code_block(sink, &code_lang, code.trim_end_matches('\n'));
    }
}

fn markdown_line(raw: &str, base_style: Style) -> Line<'static> {
    let trimmed = raw.trim_start();
    let indent = &raw[..raw.len().saturating_sub(trimmed.len())];

    if trimmed.starts_with('#') {
        let content = trimmed.trim_start_matches('#').trim_start();
        return Line::from(vec![
            Span::raw(indent.to_owned()),
            Span::styled(content.to_owned(), super::theme::MARKDOWN_HEADING),
        ]);
    }
    if let Some(content) = trimmed.strip_prefix("> ") {
        return Line::from(vec![
            Span::raw(indent.to_owned()),
            Span::styled("│ ", super::theme::MARKDOWN_QUOTE),
            Span::styled(content.to_owned(), super::theme::MARKDOWN_QUOTE),
        ]);
    }
    if is_markdown_rule(trimmed) {
        return Line::from(Span::styled(
            "─".repeat(trimmed.len().max(3)),
            ratatui::style::Style::new().fg(BORDER_FG),
        ));
    }
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        let content = trimmed[2..].to_owned();
        let mut spans = vec![
            Span::raw(indent.to_owned()),
            Span::styled("• ", super::theme::MARKDOWN_HEADING),
        ];
        spans.extend(inline_markdown_spans(&content, base_style));
        return Line::from(spans);
    }
    if is_diff_line(trimmed) {
        return Line::from(Span::styled(raw.to_owned(), diff_style(trimmed)));
    }

    Line::from(inline_markdown_spans(raw, base_style))
}

fn inline_markdown_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        if start > 0 {
            spans.push(Span::styled(rest[..start].to_owned(), base_style));
        }
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            spans.push(Span::styled(
                after[..end].to_owned(),
                super::theme::MARKDOWN_CODE,
            ));
            rest = &after[end + 1..];
        } else {
            spans.push(Span::styled("`".to_owned(), base_style));
            rest = after;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_owned(), base_style));
    }
    spans
}

fn push_code_block<S: LineSink>(sink: &mut S, lang: &str, code: &str) {
    let label = if lang.trim().is_empty() {
        "code"
    } else {
        lang.trim()
    };
    let diff_block = is_diff_block(label, code);
    sink.push_line(Line::from(Span::styled(
        format!("┌─ {label} ─"),
        ratatui::style::Style::new().fg(BORDER_FG),
    )));
    for code_line in code.split('\n') {
        let style = if diff_block && is_diff_line(code_line) {
            diff_style(code_line)
        } else {
            super::theme::MARKDOWN_CODE
        };
        sink.push_line(Line::from(vec![
            Span::styled("│ ", ratatui::style::Style::new().fg(BORDER_FG)),
            Span::styled(code_line.to_owned(), style),
        ]));
    }
    sink.push_line(Line::from(Span::styled(
        "└──",
        ratatui::style::Style::new().fg(BORDER_FG),
    )));
}

fn push_tool_detail_lines<S: LineSink>(sink: &mut S, label: &str, text: &str) {
    sink.push_line(Line::from(Span::styled(
        format!("  {label}:"),
        ratatui::style::Style::new().fg(BORDER_FG),
    )));
    push_markdown_lines(sink, text, Style::default());
}

pub struct RenderCache {
    indexed_thread_id: Option<String>,
    item_starts: Vec<usize>,
    total_lines: usize,
    indexed_version: u64,
    indexed_width: u16,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            indexed_thread_id: None,
            item_starts: Vec::new(),
            total_lines: 0,
            indexed_version: 0,
            indexed_width: 0,
        }
    }
}

pub struct VisibleWindow<'a> {
    pub items: &'a [ChatItem],
    pub start_item_index: usize,
    pub line_offset_within_first_segment: usize,
}

impl RenderCache {
    pub fn rebuild_if_stale(
        &mut self,
        thread_id: &str,
        items: &[ChatItem],
        version: u64,
        width: u16,
    ) {
        if self.is_valid(thread_id, version, width) {
            return;
        }
        self.rebuild(items, width);
        self.indexed_version = version;
        self.indexed_width = width;
        self.indexed_thread_id = Some(thread_id.to_owned());
    }

    pub(crate) fn is_valid(&self, thread_id: &str, version: u64, width: u16) -> bool {
        self.indexed_thread_id.as_deref() == Some(thread_id)
            && self.indexed_version == version
            && self.indexed_width == width
    }

    fn rebuild(&mut self, items: &[ChatItem], width: u16) {
        let mut item_starts = Vec::with_capacity(items.len());
        let mut current_start = 0usize;

        for (idx, item) in items.iter().enumerate() {
            if idx > 0 {
                current_start += 1; // separator line
            }
            item_starts.push(current_start);
            let mut sink = CountingSink { width, count: 0 };
            build_item_lines(&mut sink, item);
            current_start += sink.count;
        }

        self.item_starts = item_starts;
        self.total_lines = current_start;
    }

    pub fn visible_window<'a>(
        &self,
        items: &'a [ChatItem],
        base_row: usize,
        height: usize,
    ) -> VisibleWindow<'a> {
        if self.item_starts.is_empty() || items.is_empty() {
            return VisibleWindow {
                items: &[],
                start_item_index: 0,
                line_offset_within_first_segment: 0,
            };
        }

        let end_row = base_row + height;

        // Find first item whose start <= base_row
        let start_item_index = self.item_starts.partition_point(|&start| start <= base_row);
        let start_item_index = start_item_index.saturating_sub(1);

        // Find last item that starts before end_row
        let end_item_index = self
            .item_starts
            .partition_point(|&start| start < end_row)
            .min(self.item_starts.len());

        let item_count = end_item_index.saturating_sub(start_item_index).max(1);
        let item_count = item_count.min(items.len().saturating_sub(start_item_index));

        let item_start_abs = self.item_starts[start_item_index];
        let line_offset_within_first_segment = base_row.saturating_sub(item_start_abs);

        VisibleWindow {
            items: &items[start_item_index..start_item_index + item_count],
            start_item_index,
            line_offset_within_first_segment,
        }
    }
}

fn is_markdown_rule(line: &str) -> bool {
    line.len() >= 3 && line.chars().all(|ch| matches!(ch, '-' | '*' | '_'))
}

fn is_diff_line(line: &str) -> bool {
    line.starts_with('+')
        || line.starts_with('-')
        || line.starts_with("@@")
        || line.starts_with("diff --git")
}

fn is_diff_block(lang: &str, code: &str) -> bool {
    let lang = lang.to_ascii_lowercase();
    lang.contains("diff")
        || lang.contains("patch")
        || code.contains("diff --git")
        || code.contains("\n@@")
        || code.starts_with("@@")
        || code.contains("*** Begin Patch")
        || code
            .lines()
            .any(|line| line.starts_with("+++ ") || line.starts_with("--- "))
}

fn diff_style(line: &str) -> Style {
    if line.starts_with('+') && !line.starts_with("+++") {
        super::theme::DIFF_ADD
    } else if line.starts_with('-') && !line.starts_with("---") {
        super::theme::DIFF_DEL
    } else if line.starts_with("@@") || line.starts_with("diff --git") {
        super::theme::DIFF_HUNK
    } else {
        super::theme::MARKDOWN_CODE
    }
}

#[derive(Clone)]
struct VisualLine {
    line: Line<'static>,
    text: String,
}

fn visual_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<VisualLine> {
    let width = usize::from(width.max(1));
    let mut out = Vec::new();

    for line in lines {
        let mut current = VisualLine {
            line: Line::default(),
            text: String::new(),
        };
        let mut current_width = 0usize;

        for span in line.spans {
            let style = span.style;
            let mut span_buf = String::new();
            for ch in span.content.into_owned().chars() {
                let ch_width = char_width(ch);
                if current_width > 0 && ch_width > 0 && current_width + ch_width > width {
                    push_span(&mut current.line, &mut span_buf, style);
                    out.push(current);
                    current = VisualLine {
                        line: Line::default(),
                        text: String::new(),
                    };
                    current_width = 0;
                }

                span_buf.push(ch);
                current.text.push(ch);
                current_width = current_width.saturating_add(ch_width);
            }
            push_span(&mut current.line, &mut span_buf, style);
        }

        out.push(current);
    }

    out
}

fn push_span(line: &mut Line<'static>, buf: &mut String, style: Style) {
    if buf.is_empty() {
        return;
    }
    line.spans.push(Span::styled(std::mem::take(buf), style));
}

fn apply_selection(lines: &mut [VisualLine], selection: Option<&ChatSelection>) {
    let Some(selection) = selection.filter(|selection| !selection.is_empty()) else {
        return;
    };

    for (row, visual) in lines.iter_mut().enumerate() {
        if let Some((start_col, end_col)) = selected_cols_for_row(selection, row, &visual.text) {
            visual.line = highlight_line(std::mem::take(&mut visual.line), start_col, end_col);
        }
    }
}

fn selected_text_from_lines(lines: &[VisualLine], selection: &ChatSelection) -> Option<String> {
    let (start, end) = selection.normalized();
    if start.row >= lines.len() {
        return None;
    }

    let last_row = end.row.min(lines.len().saturating_sub(1));
    let mut selected = Vec::new();
    for row in start.row..=last_row {
        if let Some((start_col, end_col)) = selected_cols_for_row(selection, row, &lines[row].text)
        {
            selected.push(cell_slice(&lines[row].text, start_col, end_col));
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
            let ch_width = char_width(ch);
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

fn cell_slice(text: &str, start_col: usize, end_col: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        let ch_width = char_width(ch);
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

fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0).max(1)
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translation::ChatSelectionPoint;
    use minos_domain::AgentName;

    fn user_item(text: &str, is_streaming: bool) -> ChatItem {
        ChatItem::UserMessage {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain(text.into())],
            is_streaming,
        }
    }

    fn assistant_item(text: &str, is_streaming: bool) -> ChatItem {
        ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain(text.into())],
            is_streaming,
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn user_streaming_item_renders_cursor() {
        let lines = build_lines(&[user_item("sent", true)], 80);

        assert!(lines.iter().any(|line| line_text(line).contains('▓')));
    }

    #[test]
    fn assistant_streaming_item_renders_cursor() {
        let lines = build_lines(&[assistant_item("thinking", true)], 80);

        assert!(lines.iter().any(|line| line_text(line).contains('▓')));
    }

    #[test]
    fn markdown_headings_lists_inline_code_and_fences_render_structurally() {
        let lines = build_lines(
            &[assistant_item(
                "# Plan\n- run `cargo test`\n```rust\nfn main() {}\n```",
                false,
            )],
            80,
        );

        let rendered = lines.iter().map(line_text).collect::<Vec<_>>();
        assert!(rendered.iter().any(|line| line == "Plan"));
        assert!(rendered.iter().any(|line| line.contains("• run ")));
        assert!(rendered.iter().any(|line| line.contains("┌─ rust ─")));
        assert!(rendered.iter().any(|line| line.contains("fn main() {}")));
    }

    #[test]
    fn reasoning_renders_as_thinking_with_markdown() {
        let lines = build_lines(
            &[
                ChatItem::Reasoning {
                    message_id: "m1".into(),
                    text: "# Inspect\n- read `app.rs`".into(),
                    is_streaming: false,
                },
                assistant_item("final answer", false),
            ],
            80,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line == "Thinking"));
        assert!(rendered.iter().any(|line| line == "Inspect"));
        assert!(rendered.iter().any(|line| line.contains("• read ")));
    }

    #[test]
    fn diff_lines_get_diff_styles_without_treating_markdown_bullets_as_diff() {
        let lines = build_lines(
            &[assistant_item(
                "- markdown bullet\n```diff\n@@ -1 +1\n-old\n+new\n```",
                false,
            )],
            80,
        );

        let bullet = lines
            .iter()
            .find(|line| line_text(line).contains("markdown bullet"))
            .expect("bullet line");
        assert!(line_text(bullet).starts_with("• "));
        let added = lines
            .iter()
            .find(|line| line_text(line).contains("+new"))
            .expect("added diff line");
        assert_eq!(added.spans[1].style, super::super::theme::DIFF_ADD);
    }

    #[test]
    fn non_diff_code_blocks_do_not_color_markdown_lists_as_diff() {
        let lines = build_lines(
            &[assistant_item("```text\n- markdown bullet\n```", false)],
            80,
        );

        let bullet = lines
            .iter()
            .find(|line| line_text(line).contains("- markdown bullet"))
            .expect("code line");
        assert_eq!(bullet.spans[1].style, super::super::theme::MARKDOWN_CODE);
    }

    #[test]
    fn tool_call_item_renders_status_and_summary() {
        let lines = build_lines(
            &[ChatItem::ToolCall {
                message_id: "m1".into(),
                tool_call_id: "tc1".into(),
                name: "read_file".into(),
                args_summary: "file=src/main.rs".into(),
                args_detail: None,
                output_summary: Some("ok".into()),
                output_detail: None,
                is_error: false,
                is_expanded: false,
                is_streaming: false,
            }],
            80,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

        assert!(rendered
            .iter()
            .any(|line| line.contains("Tool read_file") && line.contains("done")));
    }

    #[test]
    fn selected_text_copies_after_wrapping_model() {
        let mut chat = ChatState::new("t1".into(), AgentName::Codex);
        chat.items.push(user_item("hello\nworld", false));
        chat.selection = Some(ChatSelection {
            anchor: ChatSelectionPoint { row: 1, col: 1 },
            focus: ChatSelectionPoint { row: 2, col: 2 },
        });

        assert_eq!(selected_text(&chat, 80).as_deref(), Some("ello\nwor"));
    }

    #[test]
    fn counting_sink_matches_vec_sink_line_count() {
        let items = vec![
            ChatItem::AssistantText {
                message_id: "m1".into(),
                text_parts: vec![TextPart::Plain(
                    "# Heading\n\nA long line that will definitely wrap at width 20: the quick brown fox jumps over the lazy dog repeatedly\n```\ncode line\ncode line 2\n```".into(),
                )],
                is_streaming: false,
            },
            ChatItem::ToolCall {
                message_id: "m1".into(),
                tool_call_id: "tc1".into(),
                name: "bash".into(),
                args_summary: "ls -la".into(),
                args_detail: Some("detailed args".into()),
                output_summary: Some("file1.txt\nfile2.txt".into()),
                output_detail: None,
                is_error: false,
                is_expanded: true,
                is_streaming: false,
            },
        ];

        for width in [10u16, 20, 40, 80] {
            // Build via VecSink, then wrap to count visual lines
            let mut vec_sink = VecSink(Vec::new());
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    vec_sink.push_line(separator_line(width));
                }
                build_item_lines(&mut vec_sink, item);
            }
            let actual_count = visual_lines(vec_sink.0, width).len();

            // Build via CountingSink
            let mut counting_sink = CountingSink { width, count: 0 };
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    counting_sink.push_line(separator_line(width));
                }
                build_item_lines(&mut counting_sink, item);
            }

            assert_eq!(
                counting_sink.count, actual_count,
                "CountingSink mismatch at width {width}"
            );
        }
    }

    #[test]
    fn counting_sink_counts_soft_wrapped_visual_lines() {
        let text = "abcdefghijklmno"; // 15 chars, wraps to 2 rows at width 10
        let mut vec_sink = VecSink(Vec::new());
        vec_sink.push_line(Line::from(Span::raw(text)));
        let wrapped = visual_lines(vec_sink.0, 10);
        assert_eq!(wrapped.len(), 2);

        let mut counting_sink = CountingSink { width: 10, count: 0 };
        counting_sink.push_line(Line::from(Span::raw(text)));
        assert_eq!(counting_sink.count, 2);
    }

    #[test]
    fn render_cache_rebuilds_on_version_change() {
        let mut cache = RenderCache::default();
        let items = vec![ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("hello world".into())],
            is_streaming: false,
        }];
        cache.rebuild_if_stale("t1", &items, 1, 80);
        assert!(cache.is_valid("t1", 1, 80));
        assert!(!cache.is_valid("t1", 2, 80));
        cache.rebuild_if_stale("t1", &items, 2, 80);
        assert!(cache.is_valid("t1", 2, 80));
    }

    #[test]
    fn render_cache_rebuilds_on_width_change() {
        let mut cache = RenderCache::default();
        let items = vec![ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("hello world".into())],
            is_streaming: false,
        }];
        cache.rebuild_if_stale("t1", &items, 1, 80);
        assert!(cache.is_valid("t1", 1, 80));
        assert!(!cache.is_valid("t1", 1, 40));
    }

    #[test]
    fn render_cache_rebuilds_on_thread_id_change() {
        let mut cache = RenderCache::default();
        let items = vec![ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("hello world".into())],
            is_streaming: false,
        }];
        cache.rebuild_if_stale("t1", &items, 1, 80);
        assert!(cache.is_valid("t1", 1, 80));
        assert!(!cache.is_valid("t2", 1, 80));
    }

    #[test]
    fn visible_window_returns_items_covering_scroll_range() {
        let items = vec![
            ChatItem::AssistantText {
                message_id: "m1".into(),
                text_parts: vec![TextPart::Plain("line1".into())],
                is_streaming: false,
            },
            ChatItem::AssistantText {
                message_id: "m2".into(),
                text_parts: vec![TextPart::Plain("line2".into())],
                is_streaming: false,
            },
            ChatItem::AssistantText {
                message_id: "m3".into(),
                text_parts: vec![TextPart::Plain("line3".into())],
                is_streaming: false,
            },
        ];
        let mut cache = RenderCache::default();
        cache.rebuild_if_stale("t1", &items, 1, 80);
        let window = cache.visible_window(&items, 3, 3);
        assert!(window.start_item_index <= 2);
        assert!(!window.items.is_empty());
    }

    #[test]
    fn visible_window_handles_scroll_at_boundary() {
        let items = vec![ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("line1".into())],
            is_streaming: false,
        }];
        let mut cache = RenderCache::default();
        cache.rebuild_if_stale("t1", &items, 1, 80);
        let window = cache.visible_window(&items, 0, 10);
        assert_eq!(window.start_item_index, 0);
        assert_eq!(window.line_offset_within_first_segment, 0);
    }

    #[test]
    fn visible_window_handles_empty_items() {
        let cache = RenderCache::default();
        let items: Vec<ChatItem> = vec![];
        let window = cache.visible_window(&items, 0, 10);
        assert!(window.items.is_empty());
    }
}
