use super::pending_request::PendingQuestionOption;
use super::tool_summary::{is_diff_like, summarize_tool_args, summarize_tool_output};
use super::*;
use minos_domain::AgentName;
use minos_ui_protocol::{MessageRole, SubagentStatus, UiEventMessage};

fn plain_parts(text: &str) -> Vec<TextPart> {
    vec![TextPart::Plain(text.into())]
}

#[test]
fn chat_state_message_started_then_text_delta() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
        message_id: "m1".into(),
        role: MessageRole::User,
        started_at_ms: 0,
    }]);
    assert!(cs.items.is_empty());
    assert!(cs.open_message_ids.contains("m1"));

    cs.apply_ui_events(vec![UiEventMessage::TextDelta {
        message_id: "m1".into(),
        text: "hello ".into(),
    }]);
    cs.apply_ui_events(vec![UiEventMessage::TextDelta {
        message_id: "m1".into(),
        text: "world".into(),
    }]);
    assert_eq!(cs.items.len(), 1);
    match &cs.items[0] {
        ChatItem::UserMessage {
            text_parts,
            is_streaming,
            ..
        } => {
            assert_eq!(*text_parts, plain_parts("hello world"));
            assert!(*is_streaming);
        }
        other => panic!("expected UserMessage, got {other:?}"),
    }
    assert!(cs.open_message_ids.contains("m1"));

    cs.apply_ui_events(vec![UiEventMessage::MessageCompleted {
        message_id: "m1".into(),
        finished_at_ms: 1,
    }]);
    match &cs.items[0] {
        ChatItem::UserMessage { is_streaming, .. } => assert!(!*is_streaming),
        other => panic!("expected UserMessage, got {other:?}"),
    }
    assert!(!cs.open_message_ids.contains("m1"));
}

#[test]
fn assistant_text_reasoning_and_tool_appear_in_arrival_order() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::ReasoningDelta {
            message_id: "m1".into(),
            text: "let me think".into(),
        },
        UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "read_file".into(),
            args_json: r#"{"path":"foo.rs"}"#.into(),
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "answer".into(),
        },
        UiEventMessage::MessageCompleted {
            message_id: "m1".into(),
            finished_at_ms: 1,
        },
    ]);

    assert!(matches!(cs.items[0], ChatItem::Reasoning { .. }));
    assert!(matches!(cs.items[1], ChatItem::ToolCall { .. }));
    assert!(matches!(cs.items[2], ChatItem::AssistantText { .. }));
    for item in &cs.items {
        match item {
            ChatItem::Reasoning { is_streaming, .. }
            | ChatItem::ToolCall { is_streaming, .. }
            | ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
            other => panic!("unexpected item {other:?}"),
        }
    }
}

#[test]
fn late_reasoning_after_tools_appends_new_item_not_above_tools() {
    let mut cs = ChatState::new("t1".into(), AgentName::Grok);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "read_file".into(),
            args_json: r#"{"path":"foo.rs"}"#.into(),
        },
        UiEventMessage::ToolCallCompleted {
            tool_call_id: "tc1".into(),
            output: "ok".into(),
            is_error: false,
        },
        UiEventMessage::ReasoningDelta {
            message_id: "m1".into(),
            text: "after tools".into(),
        },
        UiEventMessage::ReasoningDelta {
            message_id: "m1".into(),
            text: " continued".into(),
        },
    ]);

    assert_eq!(cs.items.len(), 2);
    assert!(matches!(cs.items[0], ChatItem::ToolCall { .. }));
    match &cs.items[1] {
        ChatItem::Reasoning { text, .. } => assert_eq!(text, "after tools continued"),
        other => panic!("expected trailing Reasoning, got {other:?}"),
    }
}

#[test]
fn interleaved_reasoning_opens_new_item_after_tool() {
    let mut cs = ChatState::new("t1".into(), AgentName::Gemini);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::ReasoningDelta {
            message_id: "m1".into(),
            text: "first".into(),
        },
        UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "grep".into(),
            args_json: r#"{"pattern":"x"}"#.into(),
        },
        UiEventMessage::ReasoningDelta {
            message_id: "m1".into(),
            text: "second".into(),
        },
    ]);

    assert_eq!(cs.items.len(), 3);
    match &cs.items[0] {
        ChatItem::Reasoning {
            text, is_streaming, ..
        } => {
            assert_eq!(text, "first");
            assert!(!*is_streaming);
        }
        other => panic!("expected first Reasoning, got {other:?}"),
    }
    assert!(matches!(cs.items[1], ChatItem::ToolCall { .. }));
    match &cs.items[2] {
        ChatItem::Reasoning {
            text, is_streaming, ..
        } => {
            assert_eq!(text, "second");
            assert!(*is_streaming);
        }
        other => panic!("expected second Reasoning, got {other:?}"),
    }
}

#[test]
fn intermediate_and_final_assistant_text_split_across_tools() {
    let mut cs = ChatState::new("t1".into(), AgentName::Grok);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "Looking into it.".into(),
        },
        UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "read_file".into(),
            args_json: r#"{"path":"a.rs"}"#.into(),
        },
        UiEventMessage::ToolCallCompleted {
            tool_call_id: "tc1".into(),
            output: "ok".into(),
            is_error: false,
        },
        UiEventMessage::ReasoningDelta {
            message_id: "m1".into(),
            text: "found the issue".into(),
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "Here is the fix.".into(),
        },
        UiEventMessage::MessageCompleted {
            message_id: "m1".into(),
            finished_at_ms: 1,
        },
    ]);

    assert_eq!(cs.items.len(), 4);
    match &cs.items[0] {
        ChatItem::AssistantText {
            text_parts,
            is_streaming,
            ..
        } => {
            assert_eq!(text_parts, &plain_parts("Looking into it."));
            assert!(!*is_streaming);
        }
        other => panic!("expected intermediate AssistantText, got {other:?}"),
    }
    assert!(matches!(cs.items[1], ChatItem::ToolCall { .. }));
    match &cs.items[2] {
        ChatItem::Reasoning {
            text, is_streaming, ..
        } => {
            assert_eq!(text, "found the issue");
            assert!(!*is_streaming);
        }
        other => panic!("expected Reasoning, got {other:?}"),
    }
    match &cs.items[3] {
        ChatItem::AssistantText {
            text_parts,
            is_streaming,
            ..
        } => {
            assert_eq!(text_parts, &plain_parts("Here is the fix."));
            assert!(!*is_streaming);
        }
        other => panic!("expected final AssistantText, got {other:?}"),
    }
}

#[test]
fn tool_call_placed_then_completed() {
    let mut cs = ChatState::new("t1".into(), AgentName::Claude);
    cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
        message_id: "m1".into(),
        role: MessageRole::Assistant,
        started_at_ms: 0,
    }]);
    cs.apply_ui_events(vec![UiEventMessage::ToolCallPlaced {
        message_id: "m1".into(),
        tool_call_id: "tc1".into(),
        name: "write_file".into(),
        args_json: r#"{"path":"foo.rs"}"#.into(),
    }]);
    assert_eq!(cs.items.len(), 1);
    match &cs.items[0] {
        ChatItem::ToolCall {
            name,
            args_summary,
            is_streaming,
            ..
        } => {
            assert_eq!(name, "write_file");
            assert_eq!(args_summary, "foo.rs");
            assert!(*is_streaming);
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }

    cs.apply_ui_events(vec![UiEventMessage::ToolCallCompleted {
        tool_call_id: "tc1".into(),
        output: "ok".into(),
        is_error: false,
    }]);
    match &cs.items[0] {
        ChatItem::ToolCall {
            output_summary,
            is_error,
            is_streaming,
            ..
        } => {
            assert_eq!(output_summary.as_deref(), Some("ok"));
            assert!(!*is_error);
            assert!(!*is_streaming);
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn duplicate_tool_call_placed_updates_existing_tool_block() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
        message_id: "m1".into(),
        role: MessageRole::Assistant,
        started_at_ms: 0,
    }]);
    cs.apply_ui_events(vec![UiEventMessage::ToolCallPlaced {
        message_id: "m1".into(),
        tool_call_id: "tool-1".into(),
        name: "commandExecution".into(),
        args_json: r#"{"command":"ls"}"#.into(),
    }]);
    cs.apply_ui_events(vec![UiEventMessage::ToolCallPlaced {
        message_id: "m1".into(),
        tool_call_id: "tool-1".into(),
        name: "commandExecution".into(),
        args_json: r#"{"command":"ls -la"}"#.into(),
    }]);

    assert_eq!(cs.items.len(), 1);
    match &cs.items[0] {
        ChatItem::ToolCall { args_summary, .. } => assert!(args_summary.contains("ls -la")),
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn codex_projected_events_render_final_text_and_tool_as_separate_items() {
    // Daemon projects raw agent events to UiEventMessage; TUI only applies projections.
    let mut translator = minos_ui_protocol::CodexTranslatorState::new("thr".into());
    let mut cs = ChatState::new("thr".into(), AgentName::Codex);
    for raw in [
        serde_json::json!({"method":"item/started","params":{
            "item":{"type":"agentMessage","id":"a1","text":""},
            "sessionId":"thr","turnId":"t1"
        }}),
        serde_json::json!({"method":"item/agentMessage/delta","params":{
            "itemId":"a1","delta":"partial"
        }}),
        serde_json::json!({"method":"item/started","params":{
            "item":{"type":"commandExecution","id":"cmd1","command":"ls","commandActions":[],"cwd":"/tmp","status":"inProgress"},
            "sessionId":"thr","turnId":"t1"
        }}),
        serde_json::json!({"method":"item/started","params":{
            "item":{"type":"agentMessage","id":"a2","text":""},
            "sessionId":"thr","turnId":"t1"
        }}),
        serde_json::json!({"method":"item/completed","params":{
            "item":{"type":"agentMessage","id":"a1","text":"partial final answer"},
            "sessionId":"thr","turnId":"t1","completedAtMs":2
        }}),
        serde_json::json!({"method":"item/completed","params":{
            "item":{"type":"commandExecution","id":"cmd1","command":"ls","commandActions":[],"cwd":"/tmp","status":"completed","aggregatedOutput":"ok","exitCode":0},
            "sessionId":"thr","turnId":"t1","completedAtMs":3
        }}),
    ] {
        let events = minos_ui_protocol::translate_codex(&mut translator, &raw).unwrap_or_default();
        cs.apply_ui_events(events);
    }

    assert_eq!(cs.items.len(), 2);
    match &cs.items[0] {
        ChatItem::AssistantText { text_parts, .. } => {
            assert_eq!(*text_parts, plain_parts("partial final answer"));
        }
        other => panic!("expected AssistantText, got {other:?}"),
    }
    match &cs.items[1] {
        ChatItem::ToolCall { output_summary, .. } => {
            assert_eq!(output_summary.as_deref(), Some("ok"));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn text_replace_uses_completed_agent_message_as_authoritative() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
        message_id: "m1".into(),
        role: MessageRole::Assistant,
        started_at_ms: 0,
    }]);
    cs.apply_ui_events(vec![UiEventMessage::TextDelta {
        message_id: "m1".into(),
        text: "partial ans".into(),
    }]);
    cs.apply_ui_events(vec![UiEventMessage::TextReplace {
        message_id: "m1".into(),
        text: "partial answer with final sentence".into(),
    }]);

    match &cs.items[0] {
        ChatItem::AssistantText { text_parts, .. } => {
            assert_eq!(
                *text_parts,
                plain_parts("partial answer with final sentence")
            );
        }
        other => panic!("expected AssistantText, got {other:?}"),
    }
}

#[test]
fn text_replace_without_delta_creates_streaming_item_for_open_message() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextReplace {
            message_id: "m1".into(),
            text: "authoritative answer".into(),
        },
    ]);

    assert_eq!(cs.items.len(), 1);
    match &cs.items[0] {
        ChatItem::AssistantText {
            text_parts,
            is_streaming,
            ..
        } => {
            assert_eq!(*text_parts, plain_parts("authoritative answer"));
            assert!(*is_streaming);
        }
        other => panic!("expected AssistantText, got {other:?}"),
    }
}

#[test]
fn text_replace_same_body_after_tools_freezes_mid_timeline() {
    // OpenCode finished-part snapshot equal to the frozen row → drop (no twin, no rewrite).
    let mut cs = ChatState::new("t1".into(), AgentName::Opencode);
    let mid = "msg_open_1";
    let body = "现在让我读取 workspace-store";
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: mid.into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: mid.into(),
            text: body.into(),
        },
        UiEventMessage::ToolCallPlaced {
            message_id: mid.into(),
            tool_call_id: "call_read_1".into(),
            name: "read".into(),
            args_json: "{}".into(),
        },
        UiEventMessage::TextReplace {
            message_id: mid.into(),
            text: body.into(),
        },
    ]);

    let assistant_count = cs
        .items
        .iter()
        .filter(|item| matches!(item, ChatItem::AssistantText { .. }))
        .count();
    assert_eq!(
        assistant_count, 1,
        "same-body replace after tools must not twin"
    );
    match &cs.items[0] {
        ChatItem::AssistantText { text_parts, .. } => {
            assert_eq!(*text_parts, plain_parts(body));
        }
        other => panic!("expected first item AssistantText, got {other:?}"),
    }
    assert!(matches!(cs.items[1], ChatItem::ToolCall { .. }));
    assert_eq!(cs.items.len(), 2);
}

#[test]
fn text_replace_new_body_after_tools_appends_at_end() {
    // Different body (new part / post-tool narration) → append; freeze early row.
    let mut cs = ChatState::new("t1".into(), AgentName::Opencode);
    let mid = "msg_open_1";
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: mid.into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: mid.into(),
            text: "first segment".into(),
        },
        UiEventMessage::ToolCallPlaced {
            message_id: mid.into(),
            tool_call_id: "call_read_1".into(),
            name: "read".into(),
            args_json: "{}".into(),
        },
        UiEventMessage::TextReplace {
            message_id: format!("{mid}\u{1e}prt_2"),
            text: "second segment after tools".into(),
        },
    ]);

    let assistants: Vec<_> = cs
        .items
        .iter()
        .filter(|item| matches!(item, ChatItem::AssistantText { .. }))
        .collect();
    assert_eq!(assistants.len(), 2);
    match &cs.items[0] {
        ChatItem::AssistantText { text_parts, .. } => {
            assert_eq!(*text_parts, plain_parts("first segment"));
        }
        other => panic!("expected first AssistantText, got {other:?}"),
    }
    assert!(matches!(cs.items[1], ChatItem::ToolCall { .. }));
    match &cs.items[2] {
        ChatItem::AssistantText { text_parts, .. } => {
            assert_eq!(*text_parts, plain_parts("second segment after tools"));
        }
        other => panic!("expected trailing AssistantText, got {other:?}"),
    }
}

#[test]
fn reasoning_replace_uses_completed_reasoning_as_authoritative() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
        message_id: "m1".into(),
        role: MessageRole::Assistant,
        started_at_ms: 0,
    }]);
    cs.apply_ui_events(vec![UiEventMessage::ReasoningDelta {
        message_id: "m1".into(),
        text: "old".into(),
    }]);
    cs.apply_ui_events(vec![UiEventMessage::ReasoningReplace {
        message_id: "m1".into(),
        text: "final thinking".into(),
    }]);

    match &cs.items[0] {
        ChatItem::Reasoning { text, .. } => assert_eq!(text, "final thinking"),
        other => panic!("expected Reasoning, got {other:?}"),
    }
}

#[test]
fn reasoning_replace_without_delta_creates_streaming_item_for_open_message() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::ReasoningReplace {
            message_id: "m1".into(),
            text: "final thinking".into(),
        },
    ]);

    assert_eq!(cs.items.len(), 1);
    match &cs.items[0] {
        ChatItem::Reasoning {
            text, is_streaming, ..
        } => {
            assert_eq!(text, "final thinking");
            assert!(*is_streaming);
        }
        other => panic!("expected Reasoning, got {other:?}"),
    }
}

#[test]
fn tool_arg_summary_highlights_task_and_skill_details() {
    // Grok-style bare targets (no `task=` / `skill=` labels).
    assert_eq!(
        summarize_tool_args(
            "Task",
            r#"{"description":"inspect parser","prompt":"find the failing branch"}"#
        ),
        "inspect parser"
    );
    assert_eq!(
        summarize_tool_args("skill", r#"{"skillName":"openai-docs"}"#),
        "openai-docs"
    );
}

#[test]
fn markdown_list_tool_output_is_not_summarized_as_diff() {
    let output = "- first item\n- second item";

    assert!(!is_diff_like(output));
    assert_eq!(summarize_tool_output(output), "- first item - second item");
}

#[test]
fn diff_tool_output_summarizes_changed_lines() {
    let output = "@@ -1 +1\n-old\n+new";

    assert!(is_diff_like(output));
    assert_eq!(summarize_tool_output(output), "+1/-1");
}

#[test]
fn raw_events_do_not_render_large_payloads_into_chat() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);

    cs.apply_ui_events(vec![UiEventMessage::Raw {
        kind: "tool/output".into(),
        payload_json: r#"{"content":"fn main() { println!(\"large source\"); }"}"#.into(),
    }]);

    assert!(cs.items.is_empty());
}

#[test]
fn opencode_permission_update_creates_pending_request() {
    let mut cs = ChatState::new("t1".into(), AgentName::Opencode);

    cs.apply_ui_events(vec![UiEventMessage::Raw {
        kind: "opencode/permission.updated".into(),
        payload_json: serde_json::json!({
            "type": "permission.updated",
            "properties": {
                "permission": {
                    "id": "perm-1",
                    "title": "Run shell",
                    "options": [
                        {"optionId": "allow_once", "kind": "allow"},
                        {"optionId": "reject_once", "kind": "reject"}
                    ]
                }
            }
        })
        .to_string(),
    }]);

    assert_eq!(cs.pending_requests.len(), 1);
    assert_eq!(cs.pending_requests[0].id(), "perm-1");
    assert!(cs.pending_requests[0].prompt.contains("Run shell"));
    assert_eq!(
        cs.pending_requests[0].kind,
        PendingAgentRequestKind::OpencodePermission {
            permission_id: "perm-1".into(),
            approve_response: "allow_once".into(),
            decline_response: "reject_once".into()
        }
    );
    assert_eq!(cs.items.len(), 1);
    assert!(matches!(cs.items[0], ChatItem::SystemMessage { .. }));
}

#[test]
fn opencode_permission_completion_clears_pending_request() {
    let mut cs = ChatState::new("t1".into(), AgentName::Opencode);

    cs.apply_ui_events(vec![UiEventMessage::Raw {
        kind: "opencode/permission.updated".into(),
        payload_json: serde_json::json!({
            "permissionID": "perm-1",
            "title": "Run shell",
            "options": [
                {"optionId": "allow_once", "kind": "allow"},
                {"optionId": "reject_once", "kind": "reject"}
            ]
        })
        .to_string(),
    }]);
    assert_eq!(cs.pending_requests.len(), 1);

    cs.apply_ui_events(vec![UiEventMessage::Raw {
        kind: "opencode/permission.updated".into(),
        payload_json: serde_json::json!({
            "type": "permission.updated",
            "properties": {
                "permission": {
                    "id": "perm-1",
                    "status": "rejected"
                }
            }
        })
        .to_string(),
    }]);

    assert!(cs.pending_requests.is_empty());
    assert_eq!(cs.items.len(), 1);
}

#[test]
fn grok_ask_user_question_creates_pending_request() {
    let mut cs = ChatState::new("t1".into(), AgentName::Grok);

    cs.apply_ui_events(vec![UiEventMessage::Raw {
        kind: "approval/request".into(),
        payload_json: serde_json::json!({
            "request_id": "ask-1",
            "session_id": "t1",
            "turn_id": "",
            "method": "x.ai/ask_user_question",
            "params": {
                "sessionId": "s1",
                "toolCallId": "tc1",
                "questions": [{
                    "question": "Pick a color?",
                    "options": [
                        {"label": "Red", "description": "warm"},
                        {"label": "Blue", "description": "cool"}
                    ],
                    "multi_select": false
                }]
            }
        })
        .to_string(),
    }]);

    assert_eq!(cs.pending_requests.len(), 1);
    assert_eq!(cs.pending_requests[0].id(), "ask-1");
    assert!(cs.pending_requests[0].prompt.contains("Pick a color?"));
    assert!(matches!(
        cs.pending_requests[0].kind,
        PendingAgentRequestKind::GrokUserQuestion { .. }
    ));
}

#[test]
fn opencode_question_asked_creates_pending_request_with_options() {
    let mut cs = ChatState::new("t1".into(), AgentName::Opencode);

    cs.apply_ui_events(vec![UiEventMessage::Raw {
        kind: "opencode/question.asked".into(),
        payload_json: serde_json::json!({
            "type": "question.asked",
            "properties": {
                "id": "que-1",
                "questions": [{
                    "header": "Core",
                    "question": "Pick a direction",
                    "options": [
                        {"label": "Fast", "description": "Ship quickly"},
                        {"label": "Robust", "description": "Prefer durability"}
                    ]
                }]
            }
        })
        .to_string(),
    }]);

    assert_eq!(cs.pending_requests.len(), 1);
    assert_eq!(cs.pending_requests[0].id(), "que-1");
    assert!(cs.pending_requests[0].prompt.contains("1. Fast"));
    assert_eq!(
        cs.pending_requests[0].kind,
        PendingAgentRequestKind::OpencodeQuestion {
            question_id: "que-1".into(),
            questions: vec![PendingQuestionSpec {
                header: "Core".into(),
                question: "Pick a direction".into(),
                options: vec![
                    PendingQuestionOption {
                        label: "Fast".into(),
                        description: "Ship quickly".into(),
                    },
                    PendingQuestionOption {
                        label: "Robust".into(),
                        description: "Prefer durability".into(),
                    },
                ],
                multiple: false,
                custom: false,
            }]
        }
    );
    assert!(matches!(cs.items[0], ChatItem::SystemMessage { .. }));
}

#[test]
fn new_assistant_message_finishes_previous_streaming_assistant() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);

    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "first".into(),
        },
        UiEventMessage::MessageStarted {
            message_id: "m2".into(),
            role: MessageRole::Assistant,
            started_at_ms: 1,
        },
    ]);

    assert_eq!(cs.items.len(), 1);
    match &cs.items[0] {
        ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
        other => panic!("expected AssistantText, got {other:?}"),
    }
    assert!(cs.open_message_ids.contains("m2"));
}

#[test]
fn last_completed_assistant_text_ignores_text_finished_only_by_next_message_start() {
    let mut cs = ChatState::new("t1".into(), AgentName::Opencode);

    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "intermediate".into(),
        },
        UiEventMessage::MessageStarted {
            message_id: "m2".into(),
            role: MessageRole::Assistant,
            started_at_ms: 1,
        },
    ]);

    assert_eq!(cs.last_completed_assistant_text(), None);
}

#[test]
fn error_pushes_error_item_and_finishes_streaming() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);

    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "partial".into(),
        },
        UiEventMessage::Error {
            code: "failed".into(),
            message: "tool failed".into(),
            message_id: Some("m1".into()),
        },
    ]);

    assert_eq!(cs.items.len(), 2);
    match &cs.items[0] {
        ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
        other => panic!("expected AssistantText, got {other:?}"),
    }
    match &cs.items[1] {
        ChatItem::Error { message_id, text } => {
            assert_eq!(message_id.as_deref(), Some("m1"));
            assert_eq!(text, "tool failed");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn thread_closed_pushes_system_message_and_finishes_streaming() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);

    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "partial".into(),
        },
        UiEventMessage::SessionClosed {
            session_id: "t1".into(),
            reason: minos_ui_protocol::SessionEndReason::UserStopped,
            closed_at_ms: 1,
        },
    ]);

    match &cs.items[0] {
        ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
        other => panic!("expected AssistantText, got {other:?}"),
    }
    match &cs.items[1] {
        ChatItem::SystemMessage { text } => assert!(text.contains("Thread closed")),
        other => panic!("expected SystemMessage, got {other:?}"),
    }
}

#[test]
fn last_completed_assistant_text_ignores_streaming_items() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "first".into(),
        },
        UiEventMessage::MessageCompleted {
            message_id: "m1".into(),
            finished_at_ms: 1,
        },
        UiEventMessage::MessageStarted {
            message_id: "m2".into(),
            role: MessageRole::Assistant,
            started_at_ms: 2,
        },
        UiEventMessage::TextDelta {
            message_id: "m2".into(),
            text: "second streaming".into(),
        },
    ]);

    assert_eq!(
        cs.last_completed_assistant_text(),
        Some(("m1".into(), "first".into()))
    );
}

#[test]
fn last_completed_assistant_text_falls_back_to_targeted_error_without_text() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::Error {
            code: "failed".into(),
            message: "tool failed".into(),
            message_id: Some("m1".into()),
        },
    ]);

    assert_eq!(
        cs.last_completed_assistant_text(),
        Some(("error:m1".into(), "tool failed".into()))
    );
}

#[test]
fn last_completed_assistant_text_prefers_text_before_targeted_error() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "partial answer".into(),
        },
        UiEventMessage::Error {
            code: "failed".into(),
            message: "tool failed".into(),
            message_id: Some("m1".into()),
        },
    ]);

    assert_eq!(
        cs.last_completed_assistant_text(),
        Some(("m1".into(), "partial answer".into()))
    );
}

#[test]
fn scroll_state_tracks_manual_navigation_and_bottom_following() {
    let mut cs = ChatState::new("t1".into(), AgentName::Gemini);
    cs.update_max_scroll(40);

    assert_eq!(cs.active_scroll(), 40);

    cs.scroll_up(5);
    assert!(!cs.auto_scroll);
    assert_eq!(cs.active_scroll(), 35);

    cs.scroll_down(3);
    assert_eq!(cs.active_scroll(), 38);

    cs.scroll_down(10);
    assert!(cs.auto_scroll);
    assert_eq!(cs.active_scroll(), 40);
}

#[test]
fn toggle_tool_expansion_bumps_version() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        },
        UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "bash".into(),
            args_json: r#"{"command":"ls"}"#.into(),
        },
    ]);
    let version_before = cs.version;

    assert!(cs.toggle_tool_expansion());
    assert!(cs.version > version_before);

    match &cs.items[0] {
        ChatItem::ToolCall {
            is_expanded,
            is_user_toggled,
            ..
        } => {
            assert!(!*is_expanded);
            assert_eq!(*is_user_toggled, Some(true));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn toggle_tool_expansion_only_updates_last_tool_call() {
    let mut cs = ChatState::new("t1".into(), AgentName::Codex);
    cs.apply_ui_events(vec![
        UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "bash".into(),
            args_json: r#"{"command":"one"}"#.into(),
        },
        UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tc2".into(),
            name: "bash".into(),
            args_json: r#"{"command":"two"}"#.into(),
        },
    ]);

    assert!(cs.toggle_tool_expansion());

    match (&cs.items[0], &cs.items[1]) {
        (
            ChatItem::ToolCall {
                is_user_toggled: first,
                ..
            },
            ChatItem::ToolCall {
                is_user_toggled: second,
                ..
            },
        ) => {
            assert_eq!(*first, None);
            assert_eq!(*second, Some(true));
        }
        other => panic!("expected tool calls, got {other:?}"),
    }
}

#[test]
fn subagent_spawn_and_status_update_render_parent_card() {
    let mut cs = ChatState::new("parent".into(), AgentName::Codex);
    cs.apply_ui_events(vec![UiEventMessage::SubagentSpawned {
        parent_session_id: "parent".into(),
        sub_session_id: "sub".into(),
        tool_call_id: "collab-1".into(),
        agent: AgentName::Codex,
        model: Some("gpt-5".into()),
        prompt: Some("inspect repository".into()),
        title: None,
    }]);

    match &cs.items[0] {
        ChatItem::SubagentCall {
            sub_session_id,
            status,
            is_streaming,
            ..
        } => {
            assert_eq!(sub_session_id, "sub");
            assert_eq!(*status, SubagentStatus::Running);
            assert!(*is_streaming);
        }
        other => panic!("expected SubagentCall, got {other:?}"),
    }

    cs.apply_ui_events(vec![UiEventMessage::SubagentStatusUpdated {
        sub_session_id: "sub".into(),
        status: SubagentStatus::Completed,
    }]);
    match &cs.items[0] {
        ChatItem::SubagentCall {
            status,
            is_streaming,
            ..
        } => {
            assert_eq!(*status, SubagentStatus::Completed);
            assert!(!*is_streaming);
        }
        other => panic!("expected SubagentCall, got {other:?}"),
    }
}

#[test]
fn tool_call_completed_closes_matching_subagent_card() {
    // Opencode historically emitted only ToolCallCompleted for `task`; chat_state must
    // still flip the linked SubagentCall so history replay does not leave cards "running".
    let mut cs = ChatState::new("parent".into(), AgentName::Opencode);
    cs.apply_ui_events(vec![
        UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "call_task".into(),
            name: "task".into(),
            args_json: r#"{"prompt":"audit"}"#.into(),
        },
        UiEventMessage::SubagentSpawned {
            parent_session_id: "parent".into(),
            sub_session_id: "ses_sub".into(),
            tool_call_id: "call_task".into(),
            agent: AgentName::Opencode,
            model: None,
            prompt: Some("audit".into()),
            title: None,
        },
    ]);

    cs.apply_ui_events(vec![UiEventMessage::ToolCallCompleted {
        tool_call_id: "call_task".into(),
        output: r#"<task id="ses_sub" state="completed">done</task>"#.into(),
        is_error: false,
    }]);

    let subagent = cs
        .items
        .iter()
        .find(|item| matches!(item, ChatItem::SubagentCall { .. }))
        .expect("subagent card");
    match subagent {
        ChatItem::SubagentCall {
            status,
            is_streaming,
            ..
        } => {
            assert_eq!(*status, SubagentStatus::Completed);
            assert!(!*is_streaming);
        }
        other => panic!("expected SubagentCall, got {other:?}"),
    }
}
