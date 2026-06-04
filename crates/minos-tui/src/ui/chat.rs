use crate::translation::{RenderedMessage, TextPart};
use minos_ui_protocol::MessageRole;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use super::theme::{
    ASSISTANT_LABEL, ERROR_STYLE, REASONING_STYLE, STREAMING_CURSOR,
    TOOL_ERROR, TOOL_NAME_STYLE, TOOL_SUCCESS, USER_LABEL,
};

pub fn render_chat(
    f: &mut Frame,
    area: Rect,
    messages: &[RenderedMessage],
    scroll_offset: u16,
) {
    let mut lines: Vec<Line> = Vec::new();

    for (idx, msg) in messages.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::from(Span::styled(
                "─".repeat(area.width as usize),
                ratatui::style::Style::new().fg(super::theme::BORDER_FG),
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
                        ratatui::style::Style::new().fg(super::theme::BORDER_FG),
                    )));
                    for code_line in code.split('\n') {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", ratatui::style::Style::new().fg(super::theme::BORDER_FG)),
                            Span::raw(code_line.to_owned()),
                        ]));
                    }
                    lines.push(Line::from(Span::styled(
                        "└──",
                        ratatui::style::Style::new().fg(super::theme::BORDER_FG),
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
            lines.push(Line::from(tc_spans));
        }

        if let Some(err) = &msg.error {
            lines.push(Line::from(Span::styled(err.clone(), ERROR_STYLE)));
        }

        if msg.is_streaming {
            lines.push(Line::from(Span::styled("▓", STREAMING_CURSOR)));
        }
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll_offset, 0));
    f.render_widget(paragraph, area);
}
