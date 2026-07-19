use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::{
    Action, AgentAction, ConversationAction, GlobalAction, InputTarget, ScrollDirection,
    ScrollTarget,
};
use crate::focus::PaneId;
use crate::nav::NavAction;
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
    if ui.overlays.delete_confirm.is_some() {
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
            KeyCode::Char('p') => {
                return KeyMapping::action(Action::Nav(NavAction::JumpToProjects))
            }
            KeyCode::Char('t') => {
                return KeyMapping::action(Action::Nav(NavAction::JumpToConversations));
            }
            _ => {}
        }
    }

    if ui.overlays.project_create.is_some() {
        return create_dialog_mapping(key);
    }
    if ui.active_approval_request().is_some() {
        return approval_overlay_key_to_mapping(ui, key);
    }
    match ui.nav_level() {
        crate::nav::NavLevel::Projects => return projects_level_mapping(key),
        crate::nav::NavLevel::Conversations { .. } => {
            return conversations_level_mapping(ui, key);
        }
        crate::nav::NavLevel::Conversation { .. } => {
            if is_input_focus(ui) {
                return KeyMapping::Input(InputTarget::Conversation);
            }
            if key.code == KeyCode::Esc && !is_input_focus(ui) {
                return KeyMapping::action(Action::Nav(crate::nav::NavAction::Uplevel));
            }
            match key.code {
                KeyCode::Up => {
                    return KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectPrev));
                }
                KeyCode::Down => {
                    return KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectNext));
                }
                KeyCode::Enter => {
                    return KeyMapping::action(Action::Nav(crate::nav::NavAction::Downlevel));
                }
                _ => {}
            }
        }
        crate::nav::NavLevel::AgentDetail { .. } => {
            if key.code == KeyCode::Esc {
                return KeyMapping::action(Action::Nav(crate::nav::NavAction::Uplevel));
            }
        }
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
        KeyCode::BackTab => {
            return KeyMapping::action(Action::Global(GlobalAction::CycleFocusPrev));
        }
        _ => {}
    }

    match ui.focus.current() {
        PaneId::Input => KeyMapping::Input(focused_input_target(ui)),
        PaneId::MainList => main_list_key_to_mapping(ui, key),
        PaneId::MainChat if matches!(ui.nav_level(), crate::nav::NavLevel::AgentDetail { .. }) => {
            agent_chat_key_to_mapping(key)
        }
        PaneId::MainChat => conversation_chat_key_to_mapping(key),
        PaneId::Sidebar => agent_list_key_to_mapping(ui, key),
        PaneId::ApprovalOverlay => approval_overlay_key_to_mapping(ui, key),
    }
}

fn focused_input_target(ui: &UiState) -> InputTarget {
    if matches!(ui.nav_level(), crate::nav::NavLevel::AgentDetail { .. }) {
        InputTarget::Agent
    } else {
        InputTarget::Conversation
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

fn main_list_key_to_mapping(_ui: &UiState, key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Up => KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectPrev)),
        KeyCode::Down => KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectNext)),
        KeyCode::Enter => KeyMapping::action(Action::Nav(crate::nav::NavAction::Downlevel)),
        KeyCode::Tab => KeyMapping::action(Action::Global(GlobalAction::CycleFocus)),
        KeyCode::BackTab => KeyMapping::action(Action::Global(GlobalAction::CycleFocusPrev)),
        KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Escape)),
        _ => KeyMapping::None,
    }
}

fn conversation_chat_key_to_mapping(key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Up => KeyMapping::action(Action::Conversation(ConversationAction::Scroll(
            ScrollDirection::Up,
            1,
        ))),
        KeyCode::Down => KeyMapping::action(Action::Conversation(ConversationAction::Scroll(
            ScrollDirection::Down,
            1,
        ))),
        KeyCode::PageUp => KeyMapping::action(Action::Conversation(ConversationAction::Scroll(
            ScrollDirection::Up,
            5,
        ))),
        KeyCode::PageDown => KeyMapping::action(Action::Conversation(ConversationAction::Scroll(
            ScrollDirection::Down,
            5,
        ))),
        KeyCode::Home => KeyMapping::action(Action::Conversation(ConversationAction::Scroll(
            ScrollDirection::Top,
            0,
        ))),
        KeyCode::End => KeyMapping::action(Action::Conversation(ConversationAction::Scroll(
            ScrollDirection::Bottom,
            0,
        ))),
        KeyCode::Enter => KeyMapping::action(Action::Global(GlobalAction::Enter)),
        KeyCode::Tab => KeyMapping::action(Action::Global(GlobalAction::CycleFocus)),
        KeyCode::BackTab => KeyMapping::action(Action::Global(GlobalAction::CycleFocusPrev)),
        KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Escape)),
        _ => KeyMapping::None,
    }
}

fn agent_list_key_to_mapping(ui: &UiState, key: KeyEvent) -> KeyMapping {
    if !ui.conversation.agent_sessions.items.is_empty() {
        return match key.code {
            KeyCode::Up => KeyMapping::action(Action::Nav(NavAction::SelectPrev)),
            KeyCode::Down => KeyMapping::action(Action::Nav(NavAction::SelectNext)),
            KeyCode::Enter => KeyMapping::action(Action::Global(GlobalAction::Enter)),
            KeyCode::Delete => KeyMapping::action(Action::Agent(AgentAction::Delete)),
            KeyCode::Tab => KeyMapping::action(Action::Global(GlobalAction::CycleFocus)),
            KeyCode::BackTab => KeyMapping::action(Action::Global(GlobalAction::CycleFocusPrev)),
            KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Escape)),
            _ => KeyMapping::None,
        };
    }
    match key.code {
        KeyCode::Up => ui
            .thread_panel
            .list
            .selected
            .map(|selected| Action::Agent(AgentAction::Select(selected.saturating_sub(1))))
            .map(KeyMapping::action)
            .unwrap_or_else(|| KeyMapping::action(Action::Global(GlobalAction::RequestRedraw))),
        KeyCode::Down => ui
            .thread_panel
            .list
            .selected
            .map(|selected| {
                let last = ui.thread_panel.list.items.len().saturating_sub(1);
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

fn approval_overlay_key_to_mapping(ui: &UiState, key: KeyEvent) -> KeyMapping {
    let Some(request) = ui.active_approval_request() else {
        return KeyMapping::None;
    };
    match key.code {
        KeyCode::Up => KeyMapping::action(Action::Agent(AgentAction::ApprovalSelectPrev)),
        KeyCode::Down => KeyMapping::action(Action::Agent(AgentAction::ApprovalSelectNext)),
        KeyCode::Enter => KeyMapping::action(Action::Agent(AgentAction::ApprovalConfirm)),
        KeyCode::Esc => KeyMapping::action(Action::Agent(AgentAction::ApprovalCancel)),
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let Some(index) = c.to_digit(10).and_then(|digit| digit.checked_sub(1)) else {
                return KeyMapping::None;
            };
            let index = index as usize;
            if index < crate::ui::approval_overlay::option_count(request).min(9) {
                KeyMapping::action(Action::Agent(AgentAction::ApprovalQuickPick(index)))
            } else {
                KeyMapping::None
            }
        }
        KeyCode::Char(c) => crate::ui::approval_overlay::shortcut_index(request, c)
            .map(|index| Action::Agent(AgentAction::ApprovalQuickPick(index)))
            .map(KeyMapping::action)
            .unwrap_or(KeyMapping::None),
        _ => KeyMapping::None,
    }
}

fn is_input_focus(ui: &UiState) -> bool {
    ui.focus.is(PaneId::Input)
}

fn projects_level_mapping(key: KeyEvent) -> KeyMapping {
    match key.code {
        KeyCode::Up => KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectPrev)),
        KeyCode::Down => KeyMapping::action(Action::Nav(crate::nav::NavAction::SelectNext)),
        KeyCode::Enter => KeyMapping::action(Action::Nav(crate::nav::NavAction::Downlevel)),
        KeyCode::Char('n') => {
            KeyMapping::action(Action::Nav(crate::nav::NavAction::OpenCreateProject))
        }
        KeyCode::Esc => KeyMapping::action(Action::Global(GlobalAction::Escape)),
        _ => KeyMapping::None,
    }
}

fn conversations_level_mapping(ui: &UiState, key: KeyEvent) -> KeyMapping {
    if is_input_focus(ui) {
        return KeyMapping::Input(InputTarget::Conversation);
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
