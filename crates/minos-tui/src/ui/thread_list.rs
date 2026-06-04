use crate::ui::ThreadEntry;
use minos_agent_runtime::state_machine::status_str;
use minos_domain::AgentName;
use ratatui::{
    layout::Rect,
    widgets::{List, ListItem, ListState},
    Frame,
};

use super::theme::{HIGHLIGHTED, THREAD_ACTIVE, THREAD_CLOSED, THREAD_IDLE, THREAD_RUNNING};

fn agent_label(agent: AgentName) -> &'static str {
    match agent {
        AgentName::Codex => "codex",
        AgentName::Claude => "claude",
        AgentName::Gemini => "gemini",
        AgentName::Opencode => "opencode",
    }
}

pub fn render_thread_list(
    f: &mut Frame,
    area: Rect,
    threads: &[ThreadEntry],
    selected: Option<usize>,
    list_state: &mut ListState,
) {
    let items: Vec<ListItem> = threads
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let state_style = style_for_state(&entry.state);
            let is_selected = selected == Some(i);
            let prefix = if is_selected { "> " } else { "  " };
            let tid_short = &entry.thread_id[..8.min(entry.thread_id.len())];
            let state_label = status_str(&entry.state);
            let agent_name = agent_label(entry.agent);
            let line = ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(prefix.to_owned(), state_style),
                ratatui::text::Span::styled(
                    format!("{:<7}", agent_name),
                    state_style,
                ),
                ratatui::text::Span::styled(
                    format!("{:>8} ", entry.workspace.file_name().unwrap_or_default().to_string_lossy()),
                    state_style,
                ),
                ratatui::text::Span::styled(
                    format!("{} {}", tid_short, state_label),
                    if is_selected { HIGHLIGHTED } else { state_style },
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(
        ratatui::widgets::Block::bordered()
            .title("Threads")
            .border_style(ratatui::style::Style::new().fg(ratatui::style::Color::DarkGray)),
    );
    f.render_stateful_widget(list, area, list_state);
}

fn style_for_state(state: &minos_agent_runtime::ThreadState) -> ratatui::style::Style {
    match state {
        minos_agent_runtime::ThreadState::Starting => THREAD_ACTIVE,
        minos_agent_runtime::ThreadState::Running { .. } => THREAD_RUNNING,
        minos_agent_runtime::ThreadState::Idle => THREAD_IDLE,
        minos_agent_runtime::ThreadState::Suspended { .. } => THREAD_RUNNING,
        minos_agent_runtime::ThreadState::Resuming => THREAD_ACTIVE,
        minos_agent_runtime::ThreadState::Closed { .. } => THREAD_CLOSED,
    }
}
