use crate::action::ConversationAction;
use crate::effect::{Effect, StateChange};
use crate::state::AppState;
use crate::ui::UiState;

pub fn handle(
    _state: &mut AppState,
    ui: &mut UiState,
    action: ConversationAction,
) -> (StateChange, Vec<Effect>) {
    let change = match action {
        ConversationAction::Scroll(direction, lines) => {
            if matches!(
                ui.nav_level(),
                crate::nav::NavLevel::Conversation { .. }
                    | crate::nav::NavLevel::AgentDetail { .. }
            ) {
                super::scroll_conversation(ui, direction, lines)
            } else {
                StateChange::none()
            }
        }
    };
    (change, vec![])
}
