use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::{
    Action, AgentAction, GlobalAction, InputTarget, RoomAction, ScrollDirection, ScrollTarget,
};
use crate::focus::PaneId;
use crate::ui::UiState;

pub(super) enum KeyMapping {
    Actions(Vec<Action>),
    Input(InputTarget),
    ClipboardPaste,
    None,
}

impl KeyMapping {
    fn action(action: Action) -> Self {
        Self::Actions(vec![action])
    }
}

pub(super) fn key_to_mapping(ui: &UiState, key: KeyEvent) -> KeyMapping {
    if ui.delete_confirm.is_some() {
        return delete_confirm_key_to_mapping(key);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('q') => return KeyMapping::action(Action::Global(GlobalAction::Quit)),
            KeyCode::Char('c') => {
                return KeyMapping::action(Action::Global(GlobalAction::InterruptOrQuit));
            }
            KeyCode::Char('v') => return KeyMapping::ClipboardPaste,
            KeyCode::Char('d') => return KeyMapping::action(Action::Agent(AgentAction::Close)),
            _ => {}
        }
    }

    if ui.project_create_dialog.is_some() {
        return create_dialog_mapping(key);
    }
    match &ui.nav_level {
        crate::nav::NavLevel::Projects => return projects_level_mapping(key),
        crate::nav::NavLevel::Sessions { .. } => {
            return sessions_level_mapping(ui, key);
        }
        crate::nav::NavLevel::Session { .. } => {
            if key.code == KeyCode::Esc && !is_input_focus(ui) {
                return KeyMapping::action(Action::Nav(crate::nav::NavAction::Uplevel));
            }
        }
        crate::nav::NavLevel::AgentDetail { .. } => {
            if key.code == KeyCode::Esc {
                return KeyMapping::action(Action::Nav(crate::nav::NavAction::Uplevel));
            }
        }
    }

    if ui.agent_picker.is_some() {
        return agent_picker_key_to_mapping(ui, key);
    }

    match key.code {
        KeyCode::PageUp => {
            return KeyMapping::action(Action::Global(GlobalAction::Scroll(
                ScrollTarget::ActivePane,
                ScrollDirection::Up,
                5,
            )));
        }
        KeyCode::PageDown => {
            return KeyMapping::action(Action::Global(GlobalAction::Scroll(
                ScrollTarget::ActivePane,
                ScrollDirection::Down,
                5,
            )));
        }
        KeyCode::Home if !is_input_focus(ui) => {
            return KeyMapping::action(Action::Global(GlobalAction::Scroll(
                ScrollTarget::ActivePane,
                ScrollDirection::Top,
                0,
            )));
        }
        KeyCode::End if !is_input_focus(ui) => {
            return KeyMapping::action(Action::Global(GlobalAction::Scroll(
                ScrollTarget::ActivePane,
                ScrollDirection::Bottom,
                0,
            )));
        }
        KeyCode::Char('n') if !is_input_focus(ui) => {
            return KeyMapping::action(Action::Global(GlobalAction::OpenAgentPicker));
        }
        KeyCode::BackTab => {
            return KeyMapping::action(Action::Global(GlobalAction::CycleFocusPrev));
        }
        _ => {}
    }

    match ui.focus.current() {
        PaneId::RoomInput => KeyMapping::Input(InputTarget::Room),
        PaneId::AgentInput => KeyMapping::Input(InputTarget::Agent),
        PaneId::RoomList => room_list_key_to_mapping(ui, key),
        PaneId::GroupChat => room_chat_key_to_mapping(key),
        PaneId::AgentList => agent_list_key_to_mapping(ui, key),
        PaneId::AgentChat => agent_chat_key_to_mapping(key),
    }
}

fn delete_confirm_key_to_mapping(key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
            KeyMapping::action(Action::Global(GlobalAction::ConfirmDelete))
        }
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
            KeyMapping::action(Action::Global(GlobalAction::CancelDelete))
        }
        _ => KeyMapping::action(Action::Global(GlobalAction::RequestRedraw)),
    }
}

fn agent_picker_key_to_mapping(ui: &UiState, key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Up => KeyMapping::action(Action::Global(GlobalAction::SelectPrevious)),
        KeyCode::Down => KeyMapping::action(Action::Global(GlobalAction::SelectNext)),
        KeyCode::Enter => KeyMapping::action(Action::Global(GlobalAction::Enter)),
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            let index = usize::from(c as u8 - b'1');
            if index < ui.status.agents.len() {
                KeyMapping::action(Action::Global(GlobalAction::SelectIndex(index)))
            } else {
                KeyMapping::None
            }
        }
        KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Escape)),
        _ => KeyMapping::None,
    }
}

fn room_list_key_to_mapping(ui: &UiState, key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Up => ui
            .selected_room
            .map(|selected| Action::Room(RoomAction::Select(selected.saturating_sub(1))))
            .map(KeyMapping::action)
            .unwrap_or_else(|| KeyMapping::action(Action::Global(GlobalAction::RequestRedraw))),
        KeyCode::Down => ui
            .selected_room
            .map(|selected| {
                let last = ui.rooms.len().saturating_sub(1);
                Action::Room(RoomAction::Select((selected + 1).min(last)))
            })
            .map(KeyMapping::action)
            .unwrap_or_else(|| KeyMapping::action(Action::Global(GlobalAction::RequestRedraw))),
        KeyCode::Enter => KeyMapping::action(Action::Global(GlobalAction::Enter)),
        KeyCode::Tab => KeyMapping::action(Action::Global(GlobalAction::CycleFocus)),
        KeyCode::BackTab => KeyMapping::action(Action::Global(GlobalAction::CycleFocusPrev)),
        KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Escape)),
        _ => KeyMapping::None,
    }
}

fn room_chat_key_to_mapping(key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Up => KeyMapping::action(Action::Room(RoomAction::Scroll(ScrollDirection::Up, 1))),
        KeyCode::Down => {
            KeyMapping::action(Action::Room(RoomAction::Scroll(ScrollDirection::Down, 1)))
        }
        KeyCode::PageUp => {
            KeyMapping::action(Action::Room(RoomAction::Scroll(ScrollDirection::Up, 5)))
        }
        KeyCode::PageDown => {
            KeyMapping::action(Action::Room(RoomAction::Scroll(ScrollDirection::Down, 5)))
        }
        KeyCode::Home => {
            KeyMapping::action(Action::Room(RoomAction::Scroll(ScrollDirection::Top, 0)))
        }
        KeyCode::End => {
            KeyMapping::action(Action::Room(RoomAction::Scroll(ScrollDirection::Bottom, 0)))
        }
        KeyCode::Enter => KeyMapping::action(Action::Global(GlobalAction::Enter)),
        KeyCode::Tab => KeyMapping::action(Action::Global(GlobalAction::CycleFocus)),
        KeyCode::BackTab => KeyMapping::action(Action::Global(GlobalAction::CycleFocusPrev)),
        KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Escape)),
        _ => KeyMapping::None,
    }
}

fn agent_list_key_to_mapping(ui: &UiState, key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Up => ui
            .selected_thread
            .map(|selected| Action::Agent(AgentAction::Select(selected.saturating_sub(1))))
            .map(KeyMapping::action)
            .unwrap_or_else(|| KeyMapping::action(Action::Global(GlobalAction::RequestRedraw))),
        KeyCode::Down => ui
            .selected_thread
            .map(|selected| {
                let last = ui.threads.len().saturating_sub(1);
                Action::Agent(AgentAction::Select((selected + 1).min(last)))
            })
            .map(KeyMapping::action)
            .unwrap_or_else(|| KeyMapping::action(Action::Global(GlobalAction::RequestRedraw))),
        KeyCode::Enter => KeyMapping::action(Action::Global(GlobalAction::Enter)),
        KeyCode::Delete => KeyMapping::action(Action::Agent(AgentAction::Delete)),
        KeyCode::Tab => KeyMapping::action(Action::Global(GlobalAction::CycleFocus)),
        KeyCode::BackTab => KeyMapping::action(Action::Global(GlobalAction::CycleFocusPrev)),
        KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Escape)),
        _ => KeyMapping::None,
    }
}

fn agent_chat_key_to_mapping(key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Up => {
            KeyMapping::action(Action::Agent(AgentAction::Scroll(ScrollDirection::Up, 1)))
        }
        KeyCode::Down => {
            KeyMapping::action(Action::Agent(AgentAction::Scroll(ScrollDirection::Down, 1)))
        }
        KeyCode::PageUp => {
            KeyMapping::action(Action::Agent(AgentAction::Scroll(ScrollDirection::Up, 5)))
        }
        KeyCode::PageDown => {
            KeyMapping::action(Action::Agent(AgentAction::Scroll(ScrollDirection::Down, 5)))
        }
        KeyCode::Home => {
            KeyMapping::action(Action::Agent(AgentAction::Scroll(ScrollDirection::Top, 0)))
        }
        KeyCode::End => KeyMapping::action(Action::Agent(AgentAction::Scroll(
            ScrollDirection::Bottom,
            0,
        ))),
        KeyCode::Enter => KeyMapping::action(Action::Global(GlobalAction::Enter)),
        KeyCode::Char('e') => KeyMapping::action(Action::Agent(AgentAction::ToggleToolExpansion)),
        KeyCode::Tab => KeyMapping::action(Action::Global(GlobalAction::CycleFocus)),
        KeyCode::BackTab => KeyMapping::action(Action::Global(GlobalAction::CycleFocusPrev)),
        KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Escape)),
        _ => KeyMapping::None,
    }
}

fn is_input_focus(ui: &UiState) -> bool {
    ui.focus.is(PaneId::RoomInput) || ui.focus.is(PaneId::AgentInput)
}

fn projects_level_mapping(key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Up => KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectPrev)),
        KeyCode::Down => KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectNext)),
        KeyCode::Enter => KeyMapping::action(Action::Nav(crate::nav::NavAction::Downlevel)),
        KeyCode::Char('n') => {
            KeyMapping::action(Action::Nav(crate::nav::NavAction::OpenCreateProject))
        }
        KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Quit)),
        _ => KeyMapping::None,
    }
}

fn sessions_level_mapping(ui: &UiState, key: KeyEvent) -> KeyMapping {
    if is_input_focus(ui) {
        return KeyMapping::Input(InputTarget::Room);
    }
    match key.code {
        KeyCode::Up => KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectPrev)),
        KeyCode::Down => KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectNext)),
        KeyCode::Enter => KeyMapping::action(Action::Nav(crate::nav::NavAction::Downlevel)),
        KeyCode::Esc => KeyMapping::action(Action::Nav(crate::nav::NavAction::Uplevel)),
        _ => KeyMapping::None,
    }
}

fn create_dialog_mapping(key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Esc => KeyMapping::action(Action::Nav(crate::nav::NavAction::CancelDialog)),
        KeyCode::Tab => KeyMapping::action(Action::Nav(crate::nav::NavAction::SwitchField)),
        KeyCode::Enter => {
            KeyMapping::action(Action::Nav(crate::nav::NavAction::ConfirmCreateProject))
        }
        KeyCode::Backspace => KeyMapping::action(Action::Nav(crate::nav::NavAction::Backspace)),
        KeyCode::Char(c) => KeyMapping::action(Action::Nav(crate::nav::NavAction::TypeChar(c))),
        _ => KeyMapping::None,
    }
}
