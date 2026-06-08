use crate::translation::{ChatSelection, ChatState, RenderedMessage, TextPart};
use minos_ui_protocol::MessageRole;
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

    let mut lines = visual_lines(
        build_lines(chat.messages.as_slice(), inner.width),
        inner.width,
    );
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

    let lines = visual_lines(build_lines(chat.messages.as_slice(), width), width);
    selected_text_from_lines(lines.as_slice(), selection)
}

fn build_lines(messages: &[RenderedMessage], separator_width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (idx, msg) in messages.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::from(Span::styled(
                "─".repeat(usize::from(separator_width.max(1))),
                ratatui::style::Style::new().fg(BORDER_FG),
            )));
        }

        let (label_text, label_style) = match msg.role {
            MessageRole::User => ("[You]", USER_LABEL),
            MessageRole::Assistant => ("[Agent]", ASSISTANT_LABEL),
            MessageRole::System => ("[System]", REASONING_STYLE),
        };
        lines.push(Line::from(Span::styled(label_text, label_style)));

        for part in &msg.text_parts {
            match part {
                TextPart::Plain(text) => {
                    push_markdown_lines(&mut lines, text, Style::default());
                }
                TextPart::Code { lang, code } => {
                    push_code_block(&mut lines, lang, code);
                }
            }
        }

        if let Some(reasoning) = &msg.reasoning {
            lines.push(Line::from(Span::styled("Thinking", REASONING_STYLE)));
            push_markdown_lines(&mut lines, reasoning, REASONING_STYLE);
        }

        for tc in &msg.tool_calls {
            let status_label = if tc.output_summary.is_none() {
                Span::styled("running", ratatui::style::Style::default())
            } else if tc.is_error {
                Span::styled("failed", TOOL_ERROR)
            } else {
                Span::styled("done", TOOL_SUCCESS)
            };
            let mut tc_spans = vec![
                Span::raw("Tool "),
                Span::styled(tc.name.clone(), TOOL_NAME_STYLE),
                Span::raw(" · "),
                status_label,
            ];
            if !tc.args_summary.is_empty() {
                tc_spans.push(Span::raw(format!(" {}", tc.args_summary)));
            }
            if tc.is_expanded {
                let mut emitted_detail = false;
                lines.push(Line::from(tc_spans.clone()));
                if let Some(args) = &tc.args_detail {
                    emitted_detail = true;
                    push_tool_detail_lines(&mut lines, "args", args);
                }
                if let Some(output) = tc.output_detail.as_ref().or(tc.output_summary.as_ref()) {
                    emitted_detail = true;
                    push_tool_detail_lines(&mut lines, "out", output);
                }
                if emitted_detail {
                    continue;
                }
            }
            lines.push(Line::from(tc_spans));
        }

        if let Some(err) = &msg.error {
            lines.push(Line::from(Span::styled(err.clone(), ERROR_STYLE)));
        }

        if msg.is_streaming && matches!(msg.role, MessageRole::Assistant) {
            lines.push(Line::from(Span::styled("▓", STREAMING_CURSOR)));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No messages yet. Press `n` to start another agent, then type below.",
            REASONING_STYLE,
        )));
    }

    lines
}

fn push_markdown_lines(lines: &mut Vec<Line<'static>>, text: &str, base_style: Style) {
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code = String::new();

    for raw_line in text.split('\n') {
        if let Some(lang) = raw_line.trim_start().strip_prefix("```") {
            if in_code {
                push_code_block(lines, &code_lang, code.trim_end_matches('\n'));
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

        lines.push(markdown_line(raw_line, base_style));
    }

    if in_code {
        push_code_block(lines, &code_lang, code.trim_end_matches('\n'));
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

fn push_code_block(lines: &mut Vec<Line<'static>>, lang: &str, code: &str) {
    let label = if lang.trim().is_empty() {
        "code"
    } else {
        lang.trim()
    };
    let diff_block = is_diff_block(label, code);
    lines.push(Line::from(Span::styled(
        format!("┌─ {label} ─"),
        ratatui::style::Style::new().fg(BORDER_FG),
    )));
    for code_line in code.split('\n') {
        let style = if diff_block && is_diff_line(code_line) {
            diff_style(code_line)
        } else {
            super::theme::MARKDOWN_CODE
        };
        lines.push(Line::from(vec![
            Span::styled("│ ", ratatui::style::Style::new().fg(BORDER_FG)),
            Span::styled(code_line.to_owned(), style),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "└──",
        ratatui::style::Style::new().fg(BORDER_FG),
    )));
}

fn push_tool_detail_lines(lines: &mut Vec<Line<'static>>, label: &str, text: &str) {
    lines.push(Line::from(Span::styled(
        format!("  {label}:"),
        ratatui::style::Style::new().fg(BORDER_FG),
    )));
    push_markdown_lines(lines, text, Style::default());
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
    use crate::translation::{ChatSelectionPoint, ToolCallBlock};
    use minos_domain::AgentName;

    fn message(role: MessageRole, text: &str, is_streaming: bool) -> RenderedMessage {
        RenderedMessage {
            message_id: "m1".into(),
            role,
            text_parts: vec![TextPart::Plain(text.into())],
            tool_calls: Vec::<ToolCallBlock>::new(),
            reasoning: None,
            is_streaming,
            error: None,
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn user_streaming_message_does_not_render_cursor() {
        let lines = build_lines(&[message(MessageRole::User, "sent", true)], 80);

        assert!(!lines.iter().any(|line| line_text(line).contains('▓')));
    }

    #[test]
    fn assistant_streaming_message_renders_cursor() {
        let lines = build_lines(&[message(MessageRole::Assistant, "thinking", true)], 80);

        assert!(lines.iter().any(|line| line_text(line).contains('▓')));
    }

    #[test]
    fn markdown_headings_lists_inline_code_and_fences_render_structurally() {
        let lines = build_lines(
            &[message(
                MessageRole::Assistant,
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
        let mut msg = message(MessageRole::Assistant, "final answer", false);
        msg.reasoning = Some("# Inspect\n- read `app.rs`".into());

        let lines = build_lines(&[msg], 80);
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line == "Thinking"));
        assert!(rendered.iter().any(|line| line == "Inspect"));
        assert!(rendered.iter().any(|line| line.contains("• read ")));
    }

    #[test]
    fn diff_lines_get_diff_styles_without_treating_markdown_bullets_as_diff() {
        let lines = build_lines(
            &[message(
                MessageRole::Assistant,
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
            &[message(
                MessageRole::Assistant,
                "```text\n- markdown bullet\n```",
                false,
            )],
            80,
        );

        let bullet = lines
            .iter()
            .find(|line| line_text(line).contains("- markdown bullet"))
            .expect("code line");
        assert_eq!(bullet.spans[1].style, super::super::theme::MARKDOWN_CODE);
    }

    #[test]
    fn selected_text_copies_after_wrapping_model() {
        let mut chat = ChatState::new("t1".into(), AgentName::Codex);
        chat.messages
            .push(message(MessageRole::User, "hello\nworld", false));
        chat.selection = Some(ChatSelection {
            anchor: ChatSelectionPoint { row: 1, col: 1 },
            focus: ChatSelectionPoint { row: 2, col: 2 },
        });

        assert_eq!(selected_text(&chat, 80).as_deref(), Some("ello\nwor"));
    }
}
