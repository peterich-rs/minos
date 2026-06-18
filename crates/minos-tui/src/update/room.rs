use minos_domain::AgentName;

use crate::action::RoomAction;
use crate::agent_route::{parse_agent_routing, thread_can_receive_message};
use crate::effect::{Effect, StateChange};
use crate::state::AppState;
use crate::ui::UiState;

enum RoomMessageTarget {
    ExistingThread(String),
    NewAgent(AgentName),
}

pub fn handle(
    _state: &mut AppState,
    ui: &mut UiState,
    action: RoomAction,
) -> (StateChange, Vec<Effect>) {
    let change = match action {
        RoomAction::Scroll(direction, lines) => {
            if matches!(
                ui.nav_level(),
                crate::nav::NavLevel::Conversation { .. }
                    | crate::nav::NavLevel::AgentDetail { .. }
            ) {
                super::scroll_conversation(ui, direction, lines)
            } else {
                super::scroll_group_chat(ui, direction, lines)
            }
        }
    };
    (change, vec![])
}

pub fn handle_submit(_state: &mut AppState, ui: &mut UiState) -> (StateChange, Vec<Effect>) {
    let text = ui.room_input.content.clone();
    if text.trim().is_empty() {
        ui.room_input.take_input();
        return (StateChange::redraw(), vec![]);
    }
    ui.room_input.history.record(text.as_str());

    if let Some((target, body)) = parse_agent_routing(text.as_str()) {
        ui.room_input.take_input();
        if let Some(thread_short_id) = target.thread_short_id {
            return (
                StateChange::redraw(),
                vec![Effect::DispatchPromptToExistingAgent {
                    agent: target.agent,
                    thread_short_id,
                    text: body,
                    group_text: text.trim().to_owned(),
                }],
            );
        }
        if body.trim().is_empty() {
            return (
                StateChange::redraw(),
                vec![Effect::InviteAgentToRoom {
                    agent: target.agent,
                    group_text: text.trim().to_owned(),
                }],
            );
        }
        return (
            StateChange::redraw(),
            vec![Effect::DispatchPromptToAgent {
                agent: target.agent,
                text: body,
                group_text: text.trim().to_owned(),
            }],
        );
    }

    let Some(target) = active_room_target(ui) else {
        ui.set_error("No agent selected. Use @agent or pick one from Agents.".into());
        return (StateChange::redraw(), vec![]);
    };

    ui.room_input.take_input();
    match target {
        RoomMessageTarget::ExistingThread(thread_id) => (
            StateChange::redraw(),
            vec![Effect::SendTextToThread {
                group_text: super::group_user_text_for_thread(ui, &thread_id, text.as_str()),
                thread_id,
                text,
            }],
        ),
        RoomMessageTarget::NewAgent(agent) => (
            StateChange::redraw(),
            vec![Effect::DispatchPromptToAgent {
                agent,
                group_text: format!("@{} {}", agent.bin_name(), text.trim()),
                text,
            }],
        ),
    }
}

fn active_room_target(ui: &UiState) -> Option<RoomMessageTarget> {
    if let Some(thread) = ui.selected_thread.and_then(|index| ui.threads.get(index)) {
        return Some(if thread_can_receive_message(&thread.state) {
            RoomMessageTarget::ExistingThread(thread.thread_id.clone())
        } else {
            RoomMessageTarget::NewAgent(thread.agent)
        });
    }

    let mut candidates = ui
        .threads
        .iter()
        .filter(|thread| thread_can_receive_message(&thread.state));
    let first = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    Some(RoomMessageTarget::ExistingThread(first.thread_id.clone()))
}
