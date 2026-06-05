use crate::translation::{ChatState, RenderedMessage, TextPart};
use minos_ui_protocol::MessageRole;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

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

    let lines = build_lines(chat.messages.as_slice(), inner.width);
    let total_lines = wrapped_line_count(lines.as_slice(), inner.width);
    let max_scroll = total_lines.saturating_sub(inner.height);
    chat.update_max_scroll(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((chat.active_scroll(), 0));
    f.render_widget(paragraph, area);
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
                if let Some(output) = &tc.output_summary {
                    lines.push(Line::from(tc_spans.clone()));
                    for detail_line in output.split('\n') {
                        lines.push(Line::from(vec![
                            Span::styled("    ", ratatui::style::Style::new().fg(BORDER_FG)),
                            Span::raw(detail_line.to_owned()),
                        ]));
                    }
                    continue;
                }
            }
            lines.push(Line::from(tc_spans));
        }

        if let Some(err) = &msg.error {
            lines.push(Line::from(Span::styled(err.clone(), ERROR_STYLE)));
        }

        if msg.is_streaming {
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

fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> u16 {
    if width == 0 {
        return 0;
    }

    let width = usize::from(width);
    let mut total = 0usize;
    for line in lines {
        let line_width = line.width().max(1);
        total += (line_width - 1) / width + 1;
    }

    total.min(usize::from(u16::MAX)) as u16
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id.chars().take(8).collect()
}
