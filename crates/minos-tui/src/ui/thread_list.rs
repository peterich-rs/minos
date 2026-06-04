use minos_agent_runtime::state_machine::status_str;
use minos_agent_runtime::store_facing::ThreadSnapshot;
use ratatui::{
    layout::Rect,
    widgets::{List, ListItem, ListState},
    Frame,
};

use super::theme::{HIGHLIGHTED, THREAD_ACTIVE, THREAD_CLOSED, THREAD_IDLE, THREAD_RUNNING};

pub fn render_thread_list(
    f: &mut Frame,
    area: Rect,
    threads: &[ThreadSnapshot],
    selected: Option<usize>,
    list_state: &mut ListState,
) {
    let items: Vec<ListItem> = threads
        .iter()
        .enumerate()
        .map(|(i, snap)| {
            let state_style = style_for_state(&snap.state);
            let is_selected = selected == Some(i);
            let prefix = if is_selected { "> " } else { "  " };
            let tid_short = &snap.thread_id[..8.min(snap.thread_id.len())];
            let state_label = status_str(&snap.state);
            let line = ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(prefix.to_owned(), state_style),
                ratatui::text::Span::styled(
                    format!("{:>8} ", snap.workspace.file_name().unwrap_or_default().to_string_lossy()),
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
