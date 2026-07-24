use super::*;

pub(super) fn conversation_input_action_needs_agent_picker_sync(action: &InputAction) -> bool {
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
pub(super) fn default_teamwork_store() -> crate::teamwork::TeamworkStore {
    match crate::teamwork::TeamworkStore::default_for_runtime() {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(
                target: "minos_tui::app",
                error = %error,
                "teamwork persistence disabled"
            );
            crate::teamwork::TeamworkStore::disabled()
        }
    }
}

#[cfg(test)]
pub(super) fn default_teamwork_store() -> crate::teamwork::TeamworkStore {
    crate::teamwork::TeamworkStore::disabled()
}

pub(super) fn conversation_agent_result_message_id(
    conversation_id: &str,
    session_id: &str,
    message_id: &str,
) -> String {
    format!("agent-result:{conversation_id}:{session_id}:{message_id}")
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
        // ACP session/request_permission — runtime maps this to option ids
        // captured when the request was registered.
        "session/request_permission" => {
            serde_json::json!({ "approved": approved })
        }
        _ => serde_json::json!({ "decision": if approved { "accept" } else { "decline" } }),
    }
}

/// Map plan-approval overlay selection text to Grok `ExitPlanModeExtResponse`.
pub(super) fn grok_plan_approval_decision(text: &str) -> serde_json::Value {
    let token = text.trim().to_ascii_lowercase();
    let outcome = match token.as_str() {
        "approve" | "approved" | "yes" | "y" | "a" => "approved",
        "abandon" | "abandoned" | "quit" | "q" => "abandoned",
        _ => "cancelled",
    };
    serde_json::json!({ "outcome": outcome })
}

/// Map answered labels / free text to Grok `AskUserQuestionExtResponse`.
pub(super) fn grok_user_question_decision(
    questions: &[PendingQuestionSpec],
    text: &str,
) -> serde_json::Value {
    let token = text.trim().to_ascii_lowercase();
    if matches!(
        token.as_str(),
        "cancel" | "cancelled" | "no" | "n" | "skip" | ""
    ) {
        return serde_json::json!({ "outcome": "cancelled" });
    }
    let answers = opencode_question_answers(questions, text);
    let mut map = serde_json::Map::new();
    for (index, answer) in answers.into_iter().enumerate() {
        if answer.is_empty() {
            continue;
        }
        // Wire keys are question indices (Grok may omit ids on model-facing Question).
        map.insert(
            index.to_string(),
            serde_json::Value::Array(answer.into_iter().map(serde_json::Value::String).collect()),
        );
    }
    if map.is_empty() {
        return serde_json::json!({ "outcome": "cancelled" });
    }
    serde_json::json!({ "outcome": "accepted", "answers": map })
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
