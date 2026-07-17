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
            if chat.toggle_tool_expansion() {
                StateChange::redraw()
            } else {
                StateChange::none()
            }
        }
        AgentAction::ApprovalSelectNext => move_approval_selection(ui, 1),
        AgentAction::ApprovalSelectPrev => move_approval_selection(ui, -1),
        AgentAction::ApprovalConfirm => return submit_selected_approval(ui),
        AgentAction::ApprovalQuickPick(index) => return submit_approval_at(ui, index),
        AgentAction::ApprovalCancel => return cancel_approval(ui),
        AgentAction::Delete => super::request_delete_current_thread(ui),
        AgentAction::Close => return (StateChange::none(), vec![Effect::CloseCurrentThread]),
    };
    (change, vec![])
}

fn move_approval_selection(ui: &mut UiState, delta: isize) -> StateChange {
    let Some(count) = ui
        .active_approval_request()
        .map(crate::ui::approval_overlay::option_count)
        .filter(|count| *count > 0)
    else {
        return StateChange::none();
    };
    let current = ui.overlays.approval_selected.min(count - 1);
    ui.overlays.approval_selected = if delta < 0 {
        current.checked_sub(1).unwrap_or(count - 1)
    } else {
        (current + 1) % count
    };
    StateChange::redraw()
}

fn submit_selected_approval(ui: &mut UiState) -> (StateChange, Vec<Effect>) {
    submit_approval_at(ui, ui.overlays.approval_selected)
}

fn cancel_approval(ui: &mut UiState) -> (StateChange, Vec<Effect>) {
    let Some(request) = ui.active_approval_request() else {
        return (StateChange::none(), vec![]);
    };
    // Plan approval: Esc → request changes (keep plan mode). Other: No/deny.
    let index = crate::ui::approval_overlay::shortcut_index(request, 's')
        .or_else(|| crate::ui::approval_overlay::shortcut_index(request, 'n'))
        .or_else(|| (crate::ui::approval_overlay::option_count(request) > 1).then_some(1));
    match index {
        Some(index) => submit_approval_at(ui, index),
        None => (StateChange::none(), vec![]),
    }
}

fn submit_approval_at(ui: &mut UiState, selected: usize) -> (StateChange, Vec<Effect>) {
    let Some(thread_id) = ui.current_thread_id().map(str::to_owned) else {
        return (StateChange::none(), vec![]);
    };
    let Some(pending) = ui.active_approval_request().cloned() else {
        return (StateChange::none(), vec![]);
    };
    let Some(text) = crate::ui::approval_overlay::selected_text(&pending, selected) else {
        return (StateChange::none(), vec![]);
    };
    ui.overlays.approval_selected = 0;
    (
        StateChange::redraw(),
        vec![Effect::SubmitPendingAgentRequest {
            thread_id,
            pending: pending.kind,
            text,
        }],
    )
}

pub fn handle_submit(_state: &mut AppState, ui: &mut UiState) -> (StateChange, Vec<Effect>) {
    let text = ui.inputs.agent.content.clone();
    if text.trim().is_empty() {
        ui.inputs.agent.take_input();
        return (StateChange::redraw(), vec![]);
    }
    ui.inputs.agent.history.record(text.as_str());

    if ui.current_thread_is_subagent() {
        ui.inputs.agent.take_input();
        ui.set_error("Subagent transcripts are read-only.".into());
        return (StateChange::redraw(), vec![]);
    }

    let Some(thread_id) = ui.current_thread_id().map(str::to_owned) else {
        ui.set_error("No agent selected for direct chat.".into());
        return (StateChange::redraw(), vec![]);
    };
    let Some((agent, runtime_state)) = thread_agent_and_state(ui, &thread_id) else {
        ui.set_error("Selected agent thread is not active.".into());
        return (StateChange::redraw(), vec![]);
    };

    if !thread_can_receive_message(runtime_state) {
        let message_body = format!("@{} {}", agent.bin_name(), text.trim());
        if let Some(conversation_id) = ui.nav_level().conversation_id().map(str::to_owned) {
            super::push_pending_conversation_user_message(ui, &conversation_id, &message_body);
        }
        ui.inputs.agent.take_input();
        return (
            StateChange::redraw(),
            vec![Effect::DispatchPromptToAgent {
                agent,
                message_body,
                text,
            }],
        );
    }

    if let Some(pending) = ui.thread_panel.chat_states.get(&thread_id)
        .and_then(ChatState::active_pending_request)
        .cloned()
    {
        ui.inputs.agent.take_input();
        return (
            StateChange::redraw(),
            vec![Effect::SubmitPendingAgentRequest {
                thread_id,
                pending: pending.kind,
                text,
            }],
        );
    }

    let message_body = super::group_user_text_for_thread(ui, &thread_id, text.as_str());
    if let (Some(conversation_id), Some(body)) = (
        ui.nav_level().conversation_id().map(str::to_owned),
        message_body.as_deref(),
    ) {
        super::push_pending_conversation_user_message(ui, &conversation_id, body);
    }
    ui.inputs.agent.take_input();
    (
        StateChange::redraw(),
        vec![Effect::SendTextToThread {
            thread_id,
            text,
            message_body,
        }],
    )
}

fn thread_agent_and_state<'a>(
    ui: &'a UiState,
    thread_id: &str,
) -> Option<(minos_domain::AgentName, &'a minos_agent_runtime::ThreadState)> {
    if let Some(session) = ui
        .conversation.agent_sessions.items
        .iter()
        .find(|session| session.thread_id == thread_id)
    {
        return Some((session.agent, &session.state));
    }
    ui.thread_panel.list.items
        .iter()
        .find(|thread| thread.thread_id == thread_id)
        .map(|thread| (thread.agent, &thread.state))
}
