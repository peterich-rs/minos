use super::*;

pub(super) fn room_input_action_needs_agent_picker_sync(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::InsertChar(_)
            | InputAction::InsertText(_)
            | InputAction::DeleteBackward
            | InputAction::DeleteForward
            | InputAction::DeleteWord
            | InputAction::DeleteNextWord
            | InputAction::DeleteToStartOfLine
            | InputAction::DeleteToEndOfLine
            | InputAction::MoveCursor(_)
            | InputAction::MoveCursorWord(_)
            | InputAction::MoveCursorLine(_)
            | InputAction::MoveToBufferStart
            | InputAction::MoveToBufferEnd
            | InputAction::NewLine
            | InputAction::AcceptMentionCompletion
    )
}

pub(super) fn format_error_chain(error: &anyhow::Error) -> String {
    let mut parts = Vec::new();
    for cause in error.chain() {
        let text = cause.to_string();
        if parts.last() != Some(&text) {
            parts.push(text);
        }
    }
    parts.join(": ")
}

#[cfg(not(test))]
pub(super) fn default_group_chat_store(workspace: &std::path::Path) -> GroupChatStore {
    match GroupChatStore::default_for_runtime(workspace) {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(
                target: "minos_tui::app",
                error = %error,
                "group chat persistence disabled"
            );
            GroupChatStore::disabled()
        }
    }
}

#[cfg(test)]
pub(super) fn default_group_chat_store(_workspace: &std::path::Path) -> GroupChatStore {
    GroupChatStore::disabled()
}

pub(super) fn group_agent_result_message_id(
    room_id: &str,
    thread_id: &str,
    message_id: &str,
) -> String {
    format!("agent-result:{room_id}:{thread_id}:{message_id}")
}

pub(super) fn codex_user_input_decision(question_ids: &[String], text: &str) -> serde_json::Value {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let mut answers = serde_json::Map::new();
    for (index, question_id) in question_ids.iter().enumerate() {
        let answer = if question_ids.len() > 1 {
            lines.get(index).copied().unwrap_or_else(|| text.trim())
        } else {
            text.trim()
        };
        answers.insert(
            question_id.clone(),
            serde_json::json!({ "answers": [answer] }),
        );
    }
    serde_json::json!({ "answers": answers })
}

pub(super) fn codex_approval_decision(method: &str, text: &str) -> serde_json::Value {
    let approved = is_affirmative(text);
    match method {
        "applyPatchApproval" | "execCommandApproval" => {
            serde_json::json!({ "decision": if approved { "approved" } else { "denied" } })
        }
        "item/permissions/requestApproval" => {
            serde_json::json!({ "permissions": {}, "scope": "turn" })
        }
        _ => serde_json::json!({ "decision": if approved { "accept" } else { "decline" } }),
    }
}

pub(super) fn opencode_permission_response(
    text: &str,
    approve_response: &str,
    decline_response: &str,
) -> String {
    if is_affirmative(text) {
        approve_response.to_owned()
    } else {
        decline_response.to_owned()
    }
}

pub(super) fn opencode_question_answers(
    questions: &[PendingQuestionSpec],
    text: &str,
) -> Vec<Vec<String>> {
    if questions.is_empty() {
        return vec![vec![text.trim().to_owned()]];
    }

    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let answer_text = if questions.len() > 1 {
                lines.get(index).copied().unwrap_or_else(|| text.trim())
            } else {
                text.trim()
            };
            parse_opencode_question_answer(question, answer_text)
        })
        .collect()
}

pub(super) fn parse_opencode_question_answer(
    question: &PendingQuestionSpec,
    text: &str,
) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let tokens = if question.multiple {
        trimmed
            .split(|ch| [',', ';'].contains(&ch))
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>()
    } else {
        vec![trimmed]
    };
    if tokens.is_empty() {
        return vec![trimmed.to_owned()];
    }

    let mut answers = Vec::new();
    for token in tokens {
        if let Some(label) = resolve_opencode_question_option(question, token) {
            answers.push(label);
        } else {
            answers.push(token.to_owned());
        }
    }
    answers
}

pub(super) fn resolve_opencode_question_option(
    question: &PendingQuestionSpec,
    token: &str,
) -> Option<String> {
    if let Ok(index) = token.parse::<usize>() {
        if (1..=question.options.len()).contains(&index) {
            return Some(question.options[index - 1].label.clone());
        }
    }

    question
        .options
        .iter()
        .find(|option| option.label.eq_ignore_ascii_case(token))
        .map(|option| option.label.clone())
}

pub(super) fn is_affirmative(text: &str) -> bool {
    let normalized = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "y" | "yes" | "approve" | "approved" | "accept" | "allow" | "ok" | "true"
    )
}
