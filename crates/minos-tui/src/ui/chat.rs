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
                    for line_text in text.split('\n') {
                        lines.push(Line::from(Span::raw(line_text.to_owned())));
                    }
                }
                TextPart::Code { lang, code } => {
                    lines.push(Line::from(Span::styled(
                        format!("┌─ {} ─", lang),
                        ratatui::style::Style::new().fg(BORDER_FG),
                    )));
                    for code_line in code.split('\n') {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", ratatui::style::Style::new().fg(BORDER_FG)),
                            Span::raw(code_line.to_owned()),
                        ]));
                    }
                    lines.push(Line::from(Span::styled(
                        "└──",
                        ratatui::style::Style::new().fg(BORDER_FG),
                    )));
                }
            }
        }

        if let Some(reasoning) = &msg.reasoning {
            for r_line in reasoning.split('\n') {
                lines.push(Line::from(Span::styled(r_line.to_owned(), REASONING_STYLE)));
            }
        }

        for tc in &msg.tool_calls {
            let status_icon = if tc.output_summary.is_none() {
                Span::styled("⏳", ratatui::style::Style::default())
            } else if tc.is_error {
                Span::styled("✗", TOOL_ERROR)
            } else {
                Span::styled("✓", TOOL_SUCCESS)
            };
            let mut tc_spans = vec![
                Span::raw("🔧 "),
                Span::styled(tc.name.clone(), TOOL_NAME_STYLE),
                Span::raw(" "),
                status_icon,
            ];
            if !tc.args_summary.is_empty() {
                tc_spans.push(Span::raw(format!(" {}", tc.args_summary)));
            }
            if tc.is_expanded {
                let mut emitted_detail = false;
                lines.push(Line::from(tc_spans.clone()));
                if let Some(args) = &tc.args_detail {
                    emitted_detail = true;
                    for detail_line in args.split('\n') {
                        lines.push(Line::from(vec![
                            Span::styled("    args: ", ratatui::style::Style::new().fg(BORDER_FG)),
                            Span::raw(detail_line.to_owned()),
                        ]));
                    }
                }
                if let Some(output) = &tc.output_summary {
                    emitted_detail = true;
                    for detail_line in output.split('\n') {
                        lines.push(Line::from(vec![
                            Span::styled("    out: ", ratatui::style::Style::new().fg(BORDER_FG)),
                            Span::raw(detail_line.to_owned()),
                        ]));
                    }
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
