use super::json_helpers::{
    direct_string_by_keys, find_array_by_key, find_string_by_keys, json_value_summary,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAgentRequest {
    pub prompt: String,
    pub kind: PendingAgentRequestKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAgentRequestKind {
    CodexUserInput {
        request_id: String,
        question_ids: Vec<String>,
    },
    CodexApproval {
        request_id: String,
        method: String,
    },
    /// Grok `x.ai/exit_plan_mode` reverse-request (plan approval).
    GrokPlanApproval {
        request_id: String,
    },
    OpencodePermission {
        permission_id: String,
        approve_response: String,
        decline_response: String,
    },
    OpencodeQuestion {
        question_id: String,
        questions: Vec<PendingQuestionSpec>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingQuestionSpec {
    pub header: String,
    pub question: String,
    pub options: Vec<PendingQuestionOption>,
    pub multiple: bool,
    pub custom: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingQuestionOption {
    pub label: String,
    pub description: String,
}

impl PendingAgentRequest {
    pub fn id(&self) -> &str {
        match &self.kind {
            PendingAgentRequestKind::CodexUserInput { request_id, .. }
            | PendingAgentRequestKind::CodexApproval { request_id, .. }
            | PendingAgentRequestKind::GrokPlanApproval { request_id } => request_id,
            PendingAgentRequestKind::OpencodePermission { permission_id, .. } => permission_id,
            PendingAgentRequestKind::OpencodeQuestion { question_id, .. } => question_id,
        }
    }

    pub(super) fn from_approval_request(value: &serde_json::Value) -> Option<Self> {
        let request_id = value.get("request_id")?.as_str()?.to_owned();
        let method = value.get("method")?.as_str()?.to_owned();
        let params = value.get("params").unwrap_or(&serde_json::Value::Null);

        if method == "item/tool/requestUserInput" {
            let questions = params
                .get("questions")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let question_ids = questions
                .iter()
                .filter_map(|question| question.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let prompt = format_user_input_prompt(&questions);
            return Some(Self {
                prompt,
                kind: PendingAgentRequestKind::CodexUserInput {
                    request_id,
                    question_ids,
                },
            });
        }

        if method == "x.ai/exit_plan_mode" {
            let prompt = format_grok_plan_approval_prompt(params);
            return Some(Self {
                prompt,
                kind: PendingAgentRequestKind::GrokPlanApproval { request_id },
            });
        }

        Some(Self {
            prompt: format_approval_prompt(&method, params),
            kind: PendingAgentRequestKind::CodexApproval { request_id, method },
        })
    }

    pub(super) fn from_opencode_permission(value: &serde_json::Value) -> Option<Self> {
        if opencode_permission_is_completed(value) {
            return None;
        }

        let permission_id = opencode_permission_id(value)?;
        let title = find_string_by_keys(value, &["title", "name", "tool", "action"])
            .unwrap_or_else(|| "permission request".to_owned());
        let description =
            find_string_by_keys(value, &["description", "message", "reason"]).unwrap_or_default();
        let prompt = if description.is_empty() {
            format!("Opencode asks for permission: {title}")
        } else {
            format!("Opencode asks for permission: {title}\n{description}")
        };
        Some(Self {
            prompt,
            kind: PendingAgentRequestKind::OpencodePermission {
                permission_id,
                approve_response: find_permission_option_response(value, true)
                    .unwrap_or_else(|| "accept".to_owned()),
                decline_response: find_permission_option_response(value, false)
                    .unwrap_or_else(|| "reject".to_owned()),
            },
        })
    }

    pub(super) fn from_opencode_question(value: &serde_json::Value) -> Option<Self> {
        let properties = value.get("properties").unwrap_or(value);
        let question_id = properties
            .get("id")
            .or_else(|| properties.get("requestID"))
            .or_else(|| value.get("id"))
            .and_then(serde_json::Value::as_str)?
            .to_owned();
        let raw_questions = properties
            .get("questions")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let questions = parse_pending_questions(&raw_questions);
        let prompt = format_pending_question_prompt("Opencode asks:", &questions);
        Some(Self {
            prompt,
            kind: PendingAgentRequestKind::OpencodeQuestion {
                question_id,
                questions,
            },
        })
    }
}

fn find_permission_option_response(value: &serde_json::Value, approve: bool) -> Option<String> {
    let options = find_array_by_key(value, "options")?;
    for option in options {
        let label = find_string_by_keys(
            option,
            &[
                "kind",
                "name",
                "label",
                "title",
                "description",
                "optionId",
                "id",
            ],
        )
        .unwrap_or_default()
        .to_ascii_lowercase();
        let is_match = if approve {
            label.contains("allow")
                || label.contains("approve")
                || label.contains("accept")
                || label.contains("yes")
                || label.contains("proceed")
        } else {
            label.contains("reject")
                || label.contains("deny")
                || label.contains("decline")
                || label.contains("cancel")
                || label.contains("no")
        };
        if !is_match {
            continue;
        }
        if let Some(response) =
            find_string_by_keys(option, &["optionId", "optionID", "id", "value"])
        {
            return Some(response);
        }
    }
    None
}

pub(super) fn opencode_permission_id(value: &serde_json::Value) -> Option<String> {
    let keys = ["permissionID", "permissionId", "permission_id", "id"];
    direct_string_by_keys(value, &keys)
        .or_else(|| {
            value
                .get("properties")
                .and_then(|properties| direct_string_by_keys(properties, &keys))
        })
        .or_else(|| {
            value
                .get("permission")
                .and_then(|permission| direct_string_by_keys(permission, &keys))
                .or_else(|| {
                    value
                        .get("properties")
                        .and_then(|properties| properties.get("permission"))
                        .and_then(|permission| direct_string_by_keys(permission, &keys))
                })
        })
        .or_else(|| {
            value
                .get("permission")
                .filter(|permission| !permission.is_object())
                .and_then(json_value_summary)
                .or_else(|| {
                    value
                        .get("properties")
                        .and_then(|properties| properties.get("permission"))
                        .filter(|permission| !permission.is_object())
                        .and_then(json_value_summary)
                })
        })
        .or_else(|| find_string_by_keys(value, &["permissionID", "permissionId", "permission_id"]))
}

pub(super) fn opencode_permission_is_completed(value: &serde_json::Value) -> bool {
    let Some(status) = find_permission_status(value) else {
        return false;
    };
    matches!(
        status.to_ascii_lowercase().as_str(),
        "approved" | "accepted" | "rejected" | "declined" | "denied" | "completed"
    )
}

fn find_permission_status(value: &serde_json::Value) -> Option<String> {
    value
        .get("permission")
        .or_else(|| {
            value
                .get("properties")
                .and_then(|props| props.get("permission"))
        })
        .and_then(|permission| find_string_by_keys(permission, &["status", "state"]))
        .or_else(|| {
            value
                .get("status")
                .or_else(|| value.get("state"))
                .and_then(json_value_summary)
        })
        .or_else(|| find_string_by_keys(value, &["status", "state"]))
}

fn format_user_input_prompt(questions: &[serde_json::Value]) -> String {
    if questions.is_empty() {
        return "Agent asks for input. Type your answer in Agent Input.".into();
    }

    let parsed = parse_pending_questions(questions);
    format_pending_question_prompt("Agent asks for input:", &parsed)
}

fn format_grok_plan_approval_prompt(params: &serde_json::Value) -> String {
    let plan = params
        .get("planContent")
        .or_else(|| params.get("plan_content"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let preview: String = plan.lines().take(12).collect::<Vec<_>>().join("\n");
    if preview.is_empty() {
        "Grok plan approval: no plan content was written yet.\n\
         [a] Approve  [s] Request changes  [q] Abandon"
            .into()
    } else {
        let truncated = if plan.lines().count() > 12 {
            format!("{preview}\n…")
        } else {
            preview
        };
        format!(
            "Grok plan approval — review plan, then choose:\n\
             [a] Approve  [s] Request changes  [q] Abandon\n\n\
             {truncated}"
        )
    }
}

fn format_approval_prompt(method: &str, params: &serde_json::Value) -> String {
    let summary = match method {
        "item/commandExecution/requestApproval" => {
            find_string_by_keys(params, &["command", "cmd", "script"]).unwrap_or_default()
        }
        "item/fileChange/requestApproval" => {
            find_string_by_keys(params, &["file", "path", "file_path", "filePath"])
                .unwrap_or_default()
        }
        "session/request_permission" => {
            let tool_call = params.get("toolCall").unwrap_or(params);
            let title = find_string_by_keys(tool_call, &["title", "name"]).unwrap_or_default();
            let kind = find_string_by_keys(tool_call, &["kind"]).unwrap_or_default();
            match (title.is_empty(), kind.is_empty()) {
                (false, false) => format!("{kind}: {title}"),
                (false, true) => title,
                (true, false) => kind,
                (true, true) => {
                    find_string_by_keys(params, &["reason", "message", "title"]).unwrap_or_default()
                }
            }
        }
        _ => find_string_by_keys(params, &["reason", "message", "title"]).unwrap_or_default(),
    };
    if summary.is_empty() {
        format!("Approval required: {method}\nType yes to approve, anything else to decline.")
    } else {
        format!("Approval required: {method}\n{summary}\nType yes to approve, anything else to decline.")
    }
}

fn parse_pending_questions(questions: &[serde_json::Value]) -> Vec<PendingQuestionSpec> {
    questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let header = question
                .get("header")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned();
            let text = question
                .get("question")
                .or_else(|| question.get("text"))
                .or_else(|| question.get("prompt"))
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Question {}", index + 1));
            let options = question
                .get("options")
                .and_then(serde_json::Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| {
                            let label = option
                                .get("label")
                                .or_else(|| option.get("value"))
                                .or_else(|| option.get("id"))
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .trim()
                                .to_owned();
                            if label.is_empty() {
                                return None;
                            }
                            let description = option
                                .get("description")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .trim()
                                .to_owned();
                            Some(PendingQuestionOption { label, description })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            PendingQuestionSpec {
                header,
                question: text,
                options,
                multiple: question
                    .get("multiple")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                custom: question
                    .get("custom")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            }
        })
        .collect()
}

fn format_pending_question_prompt(prefix: &str, questions: &[PendingQuestionSpec]) -> String {
    if questions.is_empty() {
        return format!("{prefix}\nReply in Agent Input; Shift+Enter inserts a newline.");
    }

    let mut lines = vec![prefix.to_owned()];
    for (question_index, question) in questions.iter().enumerate() {
        let label = if question.header.is_empty() {
            format!("Question {}", question_index + 1)
        } else {
            question.header.clone()
        };
        lines.push(format!("- {label}: {}", question.question));
        for (option_index, option) in question.options.iter().enumerate() {
            if option.description.is_empty() {
                lines.push(format!("  {}. {}", option_index + 1, option.label));
            } else {
                lines.push(format!(
                    "  {}. {}: {}",
                    option_index + 1,
                    option.label,
                    option.description
                ));
            }
        }
        if question.multiple {
            lines.push("  Select multiple with comma-separated numbers or labels.".into());
        }
        if question.custom {
            lines.push("  Custom text is allowed.".into());
        }
    }
    lines.push("Reply in Agent Input; use one line per question.".into());
    lines.join("\n")
}

pub(super) fn opencode_question_reply_id(value: &serde_json::Value) -> Option<String> {
    let properties = value.get("properties").unwrap_or(value);
    properties
        .get("requestID")
        .or_else(|| properties.get("request_id"))
        .or_else(|| properties.get("id"))
        .or_else(|| value.get("requestID"))
        .or_else(|| value.get("request_id"))
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}
