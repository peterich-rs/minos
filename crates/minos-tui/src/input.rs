//! Shared input-bar key mapping and action application.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::{
    CursorDirection, CursorLineDirection, HistoryDirection, InputAction, InputTarget,
};
use crate::effect::{Effect, StateChange};
use crate::ui::input_bar::{self, AgentMentionCandidate, InputState};

pub fn is_text_input_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(_))
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
}

pub fn normalize_pasted_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn key_to_input_action(
    key: KeyEvent,
    input: &InputState,
    visual_width: u16,
    target: InputTarget,
) -> Option<InputAction> {
    let is_conversation_input = matches!(target, InputTarget::Conversation);

    match key.code {
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
            Some(InputAction::ToggleMultilineMode)
        }
        KeyCode::Enter => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                if input.multiline {
                    Some(InputAction::Submit)
                } else {
                    Some(InputAction::NewLine)
                }
            } else if input.has_path_picker() {
                Some(InputAction::AcceptPathCompletion)
            } else if is_conversation_input && input.has_agent_picker() {
                Some(InputAction::AcceptMentionCompletion)
            } else if input.multiline {
                Some(InputAction::NewLine)
            } else {
                Some(InputAction::Submit)
            }
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(InputAction::MoveCursorWord(CursorDirection::Left))
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(InputAction::MoveCursorWord(CursorDirection::Right))
        }
        KeyCode::Left => Some(InputAction::MoveCursor(CursorDirection::Left)),
        KeyCode::Right => Some(InputAction::MoveCursor(CursorDirection::Right)),
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(InputAction::MoveToBufferStart)
        }
        KeyCode::Home => Some(InputAction::MoveCursor(CursorDirection::LineStart)),
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(InputAction::MoveToBufferEnd)
        }
        KeyCode::End => Some(InputAction::MoveCursor(CursorDirection::LineEnd)),
        KeyCode::Char('b')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.modifiers.contains(KeyModifiers::ALT) =>
        {
            Some(InputAction::ToggleCursorStyle)
        }
        KeyCode::Char(c)
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            match c.to_ascii_lowercase() {
                'a' => Some(InputAction::MoveCursor(CursorDirection::LineStart)),
                'b' => Some(InputAction::MoveCursor(CursorDirection::Left)),
                'e' => Some(InputAction::MoveCursor(CursorDirection::LineEnd)),
                'f' => Some(InputAction::MoveCursor(CursorDirection::Right)),
                'j' => Some(InputAction::NewLine),
                'k' => Some(InputAction::DeleteToEndOfLine),
                'u' => Some(InputAction::DeleteToStartOfLine),
                'w' => Some(InputAction::DeleteWord),
                _ => None,
            }
        }
        KeyCode::Char(c)
            if key.modifiers.contains(KeyModifiers::ALT)
                && !key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            match c.to_ascii_lowercase() {
                'b' => Some(InputAction::MoveCursorWord(CursorDirection::Left)),
                'd' => Some(InputAction::DeleteNextWord),
                'f' => Some(InputAction::MoveCursorWord(CursorDirection::Right)),
                _ => None,
            }
        }
        KeyCode::Char(c) if is_text_input_key(key) => Some(InputAction::InsertChar(c)),
        KeyCode::Backspace
            if key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
        {
            Some(InputAction::DeleteWord)
        }
        KeyCode::Backspace => Some(InputAction::DeleteBackward),
        KeyCode::Up if input.has_path_picker() => Some(InputAction::SelectPreviousPickerItem),
        KeyCode::Down if input.has_path_picker() => Some(InputAction::SelectNextPickerItem),
        KeyCode::Up if is_conversation_input && input.has_agent_picker() => {
            Some(InputAction::SelectPreviousPickerItem)
        }
        KeyCode::Down if is_conversation_input && input.has_agent_picker() => {
            Some(InputAction::SelectNextPickerItem)
        }
        KeyCode::Up => {
            let visual_row = input_bar::visual_cursor_row(
                input.content.as_str(),
                input.cursor_pos,
                visual_width.max(1),
            );
            if visual_row == 0 {
                Some(InputAction::HistoryNavigate(HistoryDirection::Previous))
            } else {
                Some(InputAction::MoveCursorLine(CursorLineDirection::Up))
            }
        }
        KeyCode::Down => {
            let width = visual_width.max(1);
            let last_row = input_bar::last_visual_row(input.content.as_str(), width);
            let current_row =
                input_bar::visual_cursor_row(input.content.as_str(), input.cursor_pos, width);
            if current_row >= last_row {
                Some(InputAction::HistoryNavigate(HistoryDirection::Next))
            } else {
                Some(InputAction::MoveCursorLine(CursorLineDirection::Down))
            }
        }
        KeyCode::Delete
            if key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
        {
            Some(InputAction::DeleteNextWord)
        }
        KeyCode::Delete => Some(InputAction::DeleteForward),
        KeyCode::Tab => {
            if input.has_path_picker() {
                Some(InputAction::AcceptPathCompletion)
            } else if is_conversation_input && input.has_agent_picker() {
                Some(InputAction::Consume)
            } else if input_bar::active_path_range(input.content.as_str(), input.cursor_pos)
                .is_some()
            {
                Some(InputAction::TogglePathPicker)
            } else {
                None
            }
        }
        KeyCode::Esc => {
            if input.has_path_picker() || input.has_agent_picker() || input.history.is_browsing() {
                Some(InputAction::DismissPicker)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn apply_input_action(
    input: &mut InputState,
    action: InputAction,
    target: InputTarget,
    path_workspace: Option<&std::path::Path>,
    mention_candidates: &[AgentMentionCandidate],
) -> (StateChange, Vec<Effect>) {
    let mut effects = Vec::new();
    let changed = match action {
        InputAction::InsertChar(c) => {
            input.insert_char(c);
            true
        }
        InputAction::InsertText(text) => {
            input.insert_str(&text);
            true
        }
        InputAction::DeleteBackward => {
            input.backspace();
            true
        }
        InputAction::DeleteForward => {
            input.delete_forward();
            true
        }
        InputAction::DeleteWord => input.delete_prev_word(),
        InputAction::DeleteNextWord => input.delete_next_word(),
        InputAction::DeleteToStartOfLine => input.delete_to_line_start(),
        InputAction::DeleteToEndOfLine => input.delete_to_line_end(),
        InputAction::MoveCursor(CursorDirection::Left) => input.move_left(),
        InputAction::MoveCursor(CursorDirection::Right) => input.move_right(),
        InputAction::MoveCursor(CursorDirection::LineStart) => input.move_line_start(),
        InputAction::MoveCursor(CursorDirection::LineEnd) => input.move_line_end(),
        InputAction::MoveCursorWord(CursorDirection::Left) => input.move_word_left(),
        InputAction::MoveCursorWord(CursorDirection::Right) => input.move_word_right(),
        InputAction::MoveCursorWord(CursorDirection::LineStart) => input.move_line_start(),
        InputAction::MoveCursorWord(CursorDirection::LineEnd) => input.move_line_end(),
        InputAction::MoveCursorLine(CursorLineDirection::Up) => input.move_up(),
        InputAction::MoveCursorLine(CursorLineDirection::Down) => input.move_down(),
        InputAction::MoveToBufferStart => input.move_to_start(),
        InputAction::MoveToBufferEnd => input.move_to_end(),
        InputAction::Submit => false,
        InputAction::NewLine => {
            input.insert_char('\n');
            true
        }
        InputAction::ToggleMultilineMode => {
            input.toggle_multiline();
            true
        }
        InputAction::ToggleCursorStyle => {
            input.toggle_cursor_style();
            true
        }
        InputAction::HistoryNavigate(HistoryDirection::Previous) => {
            let draft = input.content.clone();
            if let Some(entry) = input.history.previous(draft.as_str()).map(str::to_owned) {
                input.load_history_entry(entry.as_str());
            }
            true
        }
        InputAction::HistoryNavigate(HistoryDirection::Next) => {
            if let Some(entry) = input.history.next().map(str::to_owned) {
                input.load_history_entry(entry.as_str());
            } else if input.history.is_browsing() {
                let draft = input.history.cancel().to_owned();
                input.load_history_entry(draft.as_str());
            }
            true
        }
        InputAction::TogglePathPicker => {
            request_path_candidates(input, target, path_workspace, &mut effects)
        }
        InputAction::AcceptMentionCompletion => {
            input.accept_agent_completion(mention_candidates);
            true
        }
        InputAction::AcceptPathCompletion => {
            let re_trigger = !input.accept_path_completion();
            if re_trigger {
                request_path_candidates(input, target, path_workspace, &mut effects);
            }
            true
        }
        InputAction::SelectPreviousPickerItem => {
            if input.has_path_picker() {
                input.select_previous_path()
            } else {
                input.select_previous_agent()
            }
        }
        InputAction::SelectNextPickerItem => {
            if input.has_path_picker() {
                input.select_next_path()
            } else {
                input.select_next_agent()
            }
        }
        InputAction::DismissPicker => {
            if input.has_path_picker() {
                input.clear_path_picker();
                true
            } else if input.has_agent_picker() {
                input.clear_agent_picker();
                true
            } else if input.history.is_browsing() {
                let draft = input.history.cancel().to_owned();
                input.load_history_entry(draft.as_str());
                true
            } else {
                false
            }
        }
        InputAction::Consume => true,
    };

    let change = if changed {
        StateChange::redraw()
    } else {
        StateChange::none()
    };
    (change, effects)
}

fn request_path_candidates(
    input: &mut InputState,
    target: InputTarget,
    path_workspace: Option<&std::path::Path>,
    effects: &mut Vec<Effect>,
) -> bool {
    let Some(workspace_root) = path_workspace else {
        return false;
    };
    let Some((sequence, token)) = input.sync_path_picker() else {
        return false;
    };
    effects.push(Effect::ResolvePathCandidates {
        target,
        sequence,
        token,
        workspace_root: workspace_root.to_path_buf(),
    });
    true
}
