use crate::render::Renderable;
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

mod cache;
pub use cache::RenderCache;

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

#[cfg(test)]
struct CountingSink {
    width: u16,
    count: usize,
}
#[cfg(test)]
impl LineSink for CountingSink {
    fn push_line(&mut self, line: Line<'static>) {
        self.count += visual_line_count(&line, self.width);
    }
}

#[cfg(test)]
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

pub enum AgentChatTarget<'a> {
    Chat {
        chat: &'a mut ChatState,
        cache: &'a mut RenderCache,
    },
    Empty,
}

pub struct AgentChatRenderable<'a> {
    target: AgentChatTarget<'a>,
    focused: bool,
}

impl<'a> AgentChatRenderable<'a> {
    pub fn new(target: AgentChatTarget<'a>, focused: bool) -> Self {
        Self { target, focused }
    }
}

impl Renderable for AgentChatRenderable<'_> {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        match &mut self.target {
            AgentChatTarget::Chat { chat, cache } => {
                render_chat(f, area, chat, self.focused, cache);
            }
            AgentChatTarget::Empty => {
                render_agent_chat_placeholder(f, area, self.focused);
            }
        }
    }

    fn desired_height(&self, _width: u16) -> u16 {
        match &self.target {
            AgentChatTarget::Chat { chat, .. } => {
                u16::try_from(chat.items.len().saturating_add(2)).unwrap_or(u16::MAX)
            }
            AgentChatTarget::Empty => 4,
        }
    }
}

pub fn render_agent_chat_placeholder(f: &mut Frame, area: Rect, focused: bool) {
    let paragraph = Paragraph::new(
        "No agent selected\n\nChoose an agent from the list to inspect its detailed transcript.",
    )
    .block(
        super::theme::border_block()
            .title("Agent Detail")
            .border_style(if focused {
                FOCUSED_BORDER
            } else {
                ratatui::style::Style::new().fg(BORDER_FG)
            }),
    );
    f.render_widget(paragraph, area);
}

pub fn render_chat(
    f: &mut Frame,
    area: Rect,
    chat: &mut ChatState,
    focused: bool,
    cache: &mut RenderCache,
) {
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

    let _span = tracing::trace_span!(
        "render_chat",
        item_count = chat.items.len(),
        version = chat.version,
    )
    .entered();

    cache.rebuild_if_stale(
        chat.thread_id.as_str(),
        &chat.items,
        chat.version,
        inner.width,
    );

    let max_scroll = cache
        .total_lines()
        .saturating_sub(usize::from(inner.height))
        .min(usize::from(u16::MAX)) as u16;
    chat.update_max_scroll(max_scroll);

    let base_row = usize::from(chat.active_scroll());
    let height = usize::from(inner.height);

    if cache.total_lines() == 0 {
        let lines = vec![Line::from(Span::styled(
            "No messages yet. Press `n` to start another agent, then type below.",
            REASONING_STYLE,
        ))];
        f.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }

    let mut visible_visual_lines = cache.visible_visual_lines(base_row, height);
    if visible_visual_lines.is_empty() {
        visible_visual_lines.push(VisualLine {
            line: Line::from(Span::styled(
                "No messages yet. Press `n` to start another agent, then type below.",
                REASONING_STYLE,
            )),
            text: "No messages yet. Press `n` to start another agent, then type below.".to_owned(),
        });
    }

    apply_selection_with_offset(
        visible_visual_lines.as_mut_slice(),
        chat.selection.as_ref(),
        base_row,
    );

    let lines: Vec<Line<'static>> = visible_visual_lines.into_iter().map(|vl| vl.line).collect();

    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Extracts selected text. Uses full-rebuild (not the cache) because copy is
/// infrequent and the selection range may span items unpredictably. The cache
/// parameter is reserved for a future optimization that localizes the build.
pub fn selected_text(chat: &ChatState, width: u16, _cache: &RenderCache) -> Option<String> {
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

fn build_segment_visual_lines(item_index: usize, item: &ChatItem, width: u16) -> Vec<VisualLine> {
    let mut lines = Vec::new();
    if item_index > 0 {
        lines.push(separator_line(width));
    }
    build_item_lines(&mut VecSinkRef(&mut lines), item);
    visual_lines(lines, width)
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

fn apply_selection_with_offset(
    lines: &mut [VisualLine],
    selection: Option<&ChatSelection>,
    base_row: usize,
) {
    let Some(selection) = selection.filter(|selection| !selection.is_empty()) else {
        return;
    };

    for (local_row, visual) in lines.iter_mut().enumerate() {
        let absolute_row = base_row + local_row;
        if let Some((start_col, end_col)) =
            selected_cols_for_row(selection, absolute_row, &visual.text)
        {
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
#[path = "chat_tests.rs"]
mod tests;
