use crate::action::AgentAction;
use crate::agent_route::thread_can_receive_message;
use crate::effect::{Effect, StateChange};
use crate::state::AppState;
use crate::translation::ChatState;
use crate::ui::UiState;

pub fn handle(
    _state: &mut AppState,
    ui: &mut UiState,
    action: AgentAction,
) -> (StateChange, Vec<Effect>) {
    let change = match action {
        AgentAction::Select(index) => super::select_thread(ui, index),
        AgentAction::Scroll(direction, lines) => super::scroll_current_chat(ui, direction, lines),
        AgentAction::ToggleToolExpansion => {
            let Some(chat) = ui.current_chat_mut() else {
                return (StateChange::none(), vec![]);
            };
            chat.toggle_tool_expansion();
            StateChange::redraw()
        }
        AgentAction::Delete => super::request_delete_current_thread(ui),
        AgentAction::Close => return (StateChange::none(), vec![Effect::CloseCurrentThread]),
    };
    (change, vec![])
}

pub fn handle_submit(_state: &mut AppState, ui: &mut UiState) -> (StateChange, Vec<Effect>) {
    let text = ui.agent_input.content.clone();
    if text.trim().is_empty() {
        ui.agent_input.take_input();
        return (StateChange::redraw(), vec![]);
    }
    ui.agent_input.history.record(text.as_str());

    if ui.current_thread_is_subagent() {
        ui.agent_input.take_input();
        ui.set_error("Subagent transcripts are read-only.".into());
        return (StateChange::redraw(), vec![]);
    }

    let Some(thread_id) = ui.current_thread_id().map(str::to_owned) else {
        ui.set_error("No agent selected for direct chat.".into());
        return (StateChange::redraw(), vec![]);
    };
    let Some(thread) = ui
        .threads
        .iter()
        .find(|thread| thread.thread_id == thread_id)
    else {
        ui.set_error("Selected agent thread is not active.".into());
        return (StateChange::redraw(), vec![]);
    };
    let agent = thread.agent;

    if !thread_can_receive_message(&thread.state) {
        ui.agent_input.take_input();
        return (
            StateChange::redraw(),
            vec![Effect::DispatchPromptToAgent {
                agent,
                group_text: format!("@{} {}", agent.bin_name(), text.trim()),
                text,
            }],
        );
    }

    if let Some(pending) = ui
        .chat_states
        .get(&thread_id)
        .and_then(ChatState::active_pending_request)
        .cloned()
    {
        ui.agent_input.take_input();
        return (
            StateChange::redraw(),
            vec![Effect::SubmitPendingAgentRequest {
                thread_id,
                pending: pending.kind,
                text,
            }],
        );
    }

    let group_text = super::group_user_text_for_thread(ui, &thread_id, text.as_str());
    if let (Some(conversation_id), Some(body)) = (
        ui.nav_level().conversation_id().map(str::to_owned),
        group_text.as_deref(),
    ) {
        super::push_pending_conversation_user_message(ui, &conversation_id, body);
    }
    ui.agent_input.take_input();
    (
        StateChange::redraw(),
        vec![Effect::SendTextToThread {
            thread_id,
            text,
            group_text,
        }],
    )
}
