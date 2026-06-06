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

pub fn render_group_chat(
    f: &mut Frame,
    area: Rect,
    title: &str,
    state: &mut GroupChatState,
    focused: bool,
) {
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

    let lines = build_lines(state.messages.as_slice(), inner.width);
    let max_scroll = lines
        .len()
        .saturating_sub(usize::from(inner.height))
        .min(usize::from(u16::MAX)) as u16;
    state.update_max_scroll(max_scroll);
    let visible_lines: Vec<Line<'static>> = lines
        .into_iter()
        .skip(usize::from(state.active_scroll()))
        .take(usize::from(inner.height))
        .collect();

    let paragraph = Paragraph::new(visible_lines).block(block);
    f.render_widget(paragraph, area);
}

fn build_lines(messages: &[LocalGroupChatMessage], width: u16) -> Vec<Line<'static>> {
    if messages.is_empty() {
        return vec![Line::from(Span::styled(
            "No group messages yet.",
            REASONING_STYLE,
        ))];
    }

    let mut lines = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }

        let (label, style) = label_for_message(message);
        lines.push(Line::from(Span::styled(label, style)));
        for raw_line in message.text.split('\n') {
            lines.extend(wrap_plain_line(raw_line, width));
        }
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
}
