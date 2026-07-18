//! Chat / conversation timeline text-selection helpers.
//!
//! Mirrors Grok CLI scrollback drag-select: mouse down anchors, drag updates
//! focus, mouse up copies selected plain text and clears the highlight.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::translation::ChatSelectionPoint;
use crate::ui::UiState;

pub(crate) struct SelectionMouseResult {
    pub changed: bool,
    pub selected_text: Option<String>,
}

pub(crate) fn current_chat_selection_active(ui: &UiState) -> bool {
    agent_chat_selection_active(ui) || conversation_selection_active(ui)
}

pub(crate) fn agent_chat_selection_active(ui: &UiState) -> bool {
    ui.thread_panel
        .list
        .selected
        .and_then(|index| ui.thread_panel.list.items.get(index))
        .and_then(|thread| ui.thread_panel.chat_states.get(&thread.thread_id))
        .is_some_and(|chat| chat.selection.is_some())
}

pub(crate) fn conversation_selection_active(ui: &UiState) -> bool {
    ui.conversation.selection.is_some()
}

pub(crate) fn begin_chat_selection(ui: &mut UiState, column: u16, row: u16) -> bool {
    // Selecting agent transcript clears conversation selection and vice versa.
    ui.conversation.clear_selection();

    let content_area = chat_content_area(ui.panel_areas.agent_chat);
    if !rect_contains(content_area, column, row) {
        if let Some(chat) = ui.current_chat_mut() {
            chat.clear_selection();
        }
        return false;
    }

    let Some(chat) = ui.current_chat_mut() else {
        return false;
    };
    let point = chat_selection_point(content_area, chat.active_scroll(), column, row);
    chat.begin_selection(point);
    true
}

pub(crate) fn begin_conversation_selection(ui: &mut UiState, column: u16, row: u16) -> bool {
    if let Some(chat) = ui.current_chat_mut() {
        chat.clear_selection();
    }

    let content_area = chat_content_area(ui.panel_areas.conversation_chat);
    if !rect_contains(content_area, column, row) {
        ui.conversation.clear_selection();
        return false;
    }

    let point = chat_selection_point(content_area, ui.conversation.active_scroll(), column, row);
    ui.conversation.begin_selection(point);
    true
}

/// Click on a tool/thinking header toggles fold instead of starting a selection.
/// Returns true when a fold was toggled (caller should skip selection).
pub(crate) fn try_toggle_fold_at_click(ui: &mut UiState, column: u16, row: u16) -> bool {
    let content_area = chat_content_area(ui.panel_areas.agent_chat);
    if !rect_contains(content_area, column, row) {
        return false;
    }

    // Rebuild cache so item_starts match the currently painted layout width.
    let width = content_area.width;
    let Some((chat, cache)) = ui.current_chat_and_cache_mut() else {
        return false;
    };
    let scroll = cache.prepare_layout(crate::ui::chat::LayoutPass {
        thread_id: chat.thread_id.as_str(),
        items: &chat.items,
        version: chat.version,
        structure_version: chat.structure_version,
        width,
        verb_group_expanded: &chat.verb_group_expanded,
        viewport_height: content_area.height,
        follow_mode: chat.auto_scroll,
        scroll_offset: chat.active_scroll(),
    });
    if !chat.auto_scroll {
        chat.scroll_offset = scroll;
    }
    let abs_row = chat_selection_point(content_area, chat.active_scroll(), column, row).row;
    let Some(item_index) =
        cache.foldable_header_item_at_row(&chat.items, abs_row, &chat.verb_group_expanded)
    else {
        return false;
    };
    if chat.toggle_fold_at(item_index) {
        chat.clear_selection();
        true
    } else {
        false
    }
}

pub(crate) fn handle_chat_selection_mouse(
    ui: &mut UiState,
    mouse: MouseEvent,
) -> SelectionMouseResult {
    if conversation_selection_active(ui) {
        return handle_conversation_selection_mouse(ui, mouse);
    }
    handle_agent_chat_selection_mouse(ui, mouse)
}

fn handle_agent_chat_selection_mouse(ui: &mut UiState, mouse: MouseEvent) -> SelectionMouseResult {
    let content_area = chat_content_area(ui.panel_areas.agent_chat);
    let Some((chat, cache)) = ui.current_chat_and_cache_mut() else {
        return SelectionMouseResult {
            changed: false,
            selected_text: None,
        };
    };

    let point = chat_selection_point(content_area, chat.active_scroll(), mouse.column, mouse.row);
    chat.update_selection(point);

    let selected_text = if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
        let text = crate::ui::chat::selected_text(chat, content_area.width, cache);
        if text.is_none() {
            chat.clear_selection();
        }
        text
    } else {
        None
    };

    SelectionMouseResult {
        changed: true,
        selected_text,
    }
}

fn handle_conversation_selection_mouse(
    ui: &mut UiState,
    mouse: MouseEvent,
) -> SelectionMouseResult {
    let content_area = chat_content_area(ui.panel_areas.conversation_chat);
    let point = chat_selection_point(
        content_area,
        ui.conversation.active_scroll(),
        mouse.column,
        mouse.row,
    );
    ui.conversation.update_selection(point);

    let selected_text = if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
        let width = content_area.width;
        let revision = ui.conversation.messages_revision;
        let selection = ui.conversation.selection.clone();
        let text = selection.and_then(|selection| {
            let conversation = &mut ui.conversation;
            conversation.chat_cache.selected_text(
                &conversation.messages,
                width,
                revision,
                &selection,
            )
        });
        if text.is_none() {
            ui.conversation.clear_selection();
        }
        text
    } else {
        None
    };

    SelectionMouseResult {
        changed: true,
        selected_text,
    }
}

pub(crate) fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    area.width > 0
        && area.height > 0
        && column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

pub(crate) fn clicked_thread_index(
    area: Rect,
    list_state: &ListState,
    row: u16,
    thread_count: usize,
) -> Option<usize> {
    if area.height <= 2
        || row <= area.y
        || row >= area.y.saturating_add(area.height).saturating_sub(1)
    {
        return None;
    }

    let item_row = usize::from(row.saturating_sub(area.y + 1));
    let index = list_state.offset().saturating_add(item_row);
    (index < thread_count).then_some(index)
}

fn chat_content_area(area: Rect) -> Rect {
    if area.width <= 2 || area.height <= 2 {
        return Rect::new(area.x, area.y, 0, 0);
    }
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

fn chat_selection_point(area: Rect, scroll: u32, column: u16, row: u16) -> ChatSelectionPoint {
    let col = if area.width == 0 || column <= area.x {
        0
    } else if column >= area.x.saturating_add(area.width) {
        area.width.saturating_sub(1)
    } else {
        column.saturating_sub(area.x)
    };
    let row_offset = if area.height == 0 || row <= area.y {
        0
    } else if row >= area.y.saturating_add(area.height) {
        area.height.saturating_sub(1)
    } else {
        row.saturating_sub(area.y)
    };
    ChatSelectionPoint {
        row: (scroll as usize).saturating_add(usize::from(row_offset)),
        col: usize::from(col),
    }
}
