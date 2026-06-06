use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
    Frame,
};

use super::theme::{BORDER_FG, FOCUSED_BORDER, HIGHLIGHTED, THREAD_IDLE};

pub struct RoomEntry {
    pub room_id: String,
    pub title: String,
}

pub fn render_room_list(
    f: &mut Frame,
    area: Rect,
    rooms: &[RoomEntry],
    selected: Option<usize>,
    list_state: &mut ListState,
    focused: bool,
) {
    let items: Vec<ListItem> = rooms
        .iter()
        .enumerate()
        .map(|(index, room)| {
            let is_selected = selected == Some(index);
            let short_id = &room.room_id[..8.min(room.room_id.len())];
            let prefix = if is_selected { "> " } else { "  " };
            ListItem::new(Line::from(vec![
                Span::styled(prefix.to_owned(), THREAD_IDLE),
                Span::styled(
                    format!("{:<14}", room.title),
                    if is_selected {
                        HIGHLIGHTED
                    } else {
                        THREAD_IDLE
                    },
                ),
                Span::styled(short_id.to_owned(), THREAD_IDLE),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        ratatui::widgets::Block::bordered()
            .title("Threads")
            .border_style(if focused {
                FOCUSED_BORDER
            } else {
                ratatui::style::Style::new().fg(BORDER_FG)
            }),
    );
    f.render_stateful_widget(list, area, list_state);
}
