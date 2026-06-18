use crate::render::Renderable;
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

pub struct RoomListRenderable<'a> {
    rooms: &'a [RoomEntry],
    selected: Option<usize>,
    list_state: &'a mut ListState,
    focused: bool,
}

impl<'a> RoomListRenderable<'a> {
    pub fn new(
        rooms: &'a [RoomEntry],
        selected: Option<usize>,
        list_state: &'a mut ListState,
        focused: bool,
    ) -> Self {
        Self {
            rooms,
            selected,
            list_state,
            focused,
        }
    }
}

impl Renderable for RoomListRenderable<'_> {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        render_room_list(
            f,
            area,
            self.rooms,
            self.selected,
            self.list_state,
            self.focused,
        );
    }

    fn desired_height(&self, _width: u16) -> u16 {
        u16::try_from(self.rooms.len().saturating_add(2)).unwrap_or(u16::MAX)
    }
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
