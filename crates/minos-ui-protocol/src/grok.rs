//! Grok ACP → `UiEventMessage` translator.
//!
//! Aligns with grok-build pager `AcpUpdateTracker` semantics:
//! - Parse `SessionNotification._meta` (`streamStartMs`, timestamps, promptId…)
//! - Close the open assistant text message on tool calls and stream boundaries
//! - Suppress plumbing tools (todo/wait/task-output) from the session timeline
//! - Prefer `rawInput.description` for tool display titles
//! - Richer tool content / locations / failed status extraction

use crate::error::TranslationError;
use crate::message::{
    DisplayPayload, MessageRole, SubagentStatus, ThreadEndReason, UiEventMessage,
};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use uuid::Uuid;

pub struct GrokTranslatorState {
    thread_id: String,
    #[allow(dead_code)]
    session_id: Option<String>,
    open_assistant_message_id: Option<String>,
    open_user_message_id: Option<String>,
    emitted_message_ids: HashSet<String>,
    tool_calls: HashMap<String, OpenGrokToolCall>,
    /// Grok ACP background subagents already projected as `SubagentSpawned`.
    known_subagents: HashMap<String, GrokSubagentMeta>,
    /// Tool call IDs suppressed from scrollback (todo / wait / task plumbing).
    suppressed_tools: HashSet<String>,
    /// `tool_call_update` arrived before `tool_call` (orphan race).
    orphan_updates: HashMap<String, Value>,
    /// After a tool, the next agent text chunk must open a new message_id
    /// (official pager: `current_agent_msg = None` on tool_call).
    force_new_assistant_on_text: bool,
    /// Last `_meta.streamStartMs` — change finishes the open assistant message.
    last_stream_start_ms: Option<i64>,
}

struct OpenGrokToolCall {
    #[allow(dead_code)]
    message_id: String,
    name: String,
    suppressed: bool,
}

struct GrokSubagentMeta {
    #[allow(dead_code)]
    tool_call_id: String,
    #[allow(dead_code)]
    title: Option<String>,
}

/// Parsed `SessionNotification._meta` (xAI extension fields).
#[derive(Debug, Default, Clone)]
struct NotificationMeta {
    stream_start_ms: Option<i64>,
    turn_start_ms: Option<i64>,
    agent_timestamp_ms: Option<i64>,
    prompt_id: Option<String>,
    event_id: Option<String>,
    total_tokens: Option<u64>,
    is_replay: bool,
}

impl NotificationMeta {
    fn from_params(params: &Value) -> Self {
        let Some(m) = params.get("_meta").and_then(Value::as_object) else {
            return Self::default();
        };
        Self {
            stream_start_ms: m.get("streamStartMs").and_then(Value::as_i64),
            turn_start_ms: m.get("turnStartMs").and_then(Value::as_i64),
            agent_timestamp_ms: m.get("agentTimestampMs").and_then(Value::as_i64),
            prompt_id: m.get("promptId").and_then(Value::as_str).map(str::to_owned),
            event_id: m.get("eventId").and_then(Value::as_str).map(str::to_owned),
            total_tokens: m.get("totalTokens").and_then(Value::as_u64),
            is_replay: m.get("isReplay").and_then(Value::as_bool).unwrap_or(false),
        }
    }
}

impl GrokTranslatorState {
    #[must_use]
    pub fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            session_id: None,
            open_assistant_message_id: None,
            open_user_message_id: None,
            emitted_message_ids: HashSet::new(),
            tool_calls: HashMap::new(),
            known_subagents: HashMap::new(),
            suppressed_tools: HashSet::new(),
            orphan_updates: HashMap::new(),
            force_new_assistant_on_text: false,
            last_stream_start_ms: None,
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn ensure_assistant_message(
    state: &mut GrokTranslatorState,
    started_at_ms: i64,
) -> (String, Vec<UiEventMessage>) {
    let message_id = state.open_assistant_message_id.clone().unwrap_or_else(|| {
        let id = format!("msg_{}", Uuid::new_v4());
        state.open_assistant_message_id = Some(id.clone());
        id
    });

    let mut events = Vec::new();
    if state.emitted_message_ids.insert(message_id.clone()) {
        events.push(UiEventMessage::MessageStarted {
            message_id: message_id.clone(),
            role: MessageRole::Assistant,
            started_at_ms,
        });
    }

    (message_id, events)
}

fn complete_open_assistant_message(
    state: &mut GrokTranslatorState,
    finished_at_ms: i64,
) -> Vec<UiEventMessage> {
    state
        .open_assistant_message_id
        .take()
        .map(|message_id| {
            vec![UiEventMessage::MessageCompleted {
                message_id,
                finished_at_ms,
            }]
        })
        .unwrap_or_default()
}

/// Apply `_meta` stream/turn boundaries. Returns events from closing the open message.
fn apply_notification_meta(
    state: &mut GrokTranslatorState,
    params: &Value,
) -> (NotificationMeta, Vec<UiEventMessage>) {
    let meta = NotificationMeta::from_params(params);
    let mut events = Vec::new();
    let finish_at = meta.agent_timestamp_ms.unwrap_or_else(now_ms);

    if let Some(new_start) = meta.stream_start_ms {
        if state
            .last_stream_start_ms
            .is_some_and(|prev| prev != new_start)
        {
            // New LLM stream iteration — finish prior agent text (official pager).
            events.extend(complete_open_assistant_message(state, finish_at));
            state.force_new_assistant_on_text = false;
        }
        state.last_stream_start_ms = Some(new_start);
    }

    // Do NOT emit Raw meta on every notification — Grok stamps _meta on nearly
    // every session/update. Flooding Raw events balloons ingest frames and
    // Desktop live-merge/re-render, which breaks session scroll for Grok only.
    // Segmentation uses streamStartMs above; other meta fields are unused.
    let _ = (
        meta.prompt_id.as_ref(),
        meta.agent_timestamp_ms,
        meta.total_tokens,
        meta.event_id.as_ref(),
        meta.turn_start_ms,
        meta.is_replay,
    );

    (meta, events)
}

fn begin_text_segment(
    state: &mut GrokTranslatorState,
    meta: &NotificationMeta,
) -> (String, Vec<UiEventMessage>) {
    let started = meta.agent_timestamp_ms.unwrap_or(0);
    let mut events = Vec::new();
    if state.force_new_assistant_on_text {
        events.extend(complete_open_assistant_message(
            state,
            meta.agent_timestamp_ms.unwrap_or_else(now_ms),
        ));
        state.force_new_assistant_on_text = false;
    }
    let (mid, start_events) = ensure_assistant_message(state, started);
    events.extend(start_events);
    (mid, events)
}

/// Plumbing / internal tools that the official pager keeps out of scrollback.
fn is_suppressed_tool(title: &str, kind: &str, raw_input: Option<&Value>) -> bool {
    let t = title.trim();
    let lower = t.to_ascii_lowercase();
    if matches!(
        t,
        "TodoWrite"
            | "todo_write"
            | "TodoRead"
            | "update_goal"
            | "UpdateGoal"
            | "spawn_subagent"
            | "task"
            | "Task"
            | "get_command_or_subagent_output"
            | "kill_command_or_subagent"
            | "wait_commands_or_subagents"
            | "get_task_output"
            | "kill_task"
            | "wait_tasks"
            | "get_task_or_subagent_output"
            | "kill_task_or_subagent"
            | "wait_tasks_or_subagents"
            | "AwaitShell"
            | "Await"
            | "scheduler_create"
            | "scheduler_delete"
            | "scheduler_list"
    ) {
        return true;
    }
    if lower.starts_with("await:")
        || lower.starts_with("sleep ")
        || lower.starts_with("wait tasks:")
        || lower.starts_with("kill task:")
        || lower == "todowrite"
        || lower.contains("todo_write")
    {
        return true;
    }
    if kind.eq_ignore_ascii_case("think") {
        return true;
    }
    if let Some(raw) = raw_input {
        if let Some(variant) = raw.get("variant").and_then(Value::as_str) {
            if matches!(
                variant,
                "TaskOutput" | "KillTask" | "WaitTasks" | "Task" | "TodoWrite"
            ) {
                return true;
            }
        }
    }
    false
}

fn tool_raw_input(update: &Value) -> Option<&Value> {
    update
        .get("rawInput")
        .or_else(|| update.get("raw_input"))
        .or_else(|| {
            update
                .get("_meta")
                .and_then(|m| m.get("rawInput").or_else(|| m.get("raw_input")))
        })
}

fn tool_description(update: &Value) -> Option<String> {
    tool_raw_input(update)
        .and_then(|r| r.get("description").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            update
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
}

fn tool_path_hint(update: &Value) -> Option<String> {
    // Prefer structured locations[], then rawInput path/file_path/target.
    if let Some(locs) = update.get("locations").and_then(Value::as_array) {
        for loc in locs {
            if let Some(p) = loc
                .get("path")
                .or_else(|| loc.get("uri"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(p.to_owned());
            }
        }
    }
    let raw = tool_raw_input(update)?;
    for key in [
        "path",
        "file_path",
        "filePath",
        "target_file",
        "targetFile",
        "command",
    ] {
        if let Some(p) = raw
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(p.to_owned());
        }
    }
    None
}

fn normalize_tool_kind(kind: &str) -> String {
    match kind.to_ascii_lowercase().as_str() {
        "read" | "read_file" | "readfile" => "read".into(),
        "edit" | "write" | "diff" | "search_replace" | "apply_patch" => "edit".into(),
        "execute" | "terminal" | "bash" | "shell" | "run" => "execute".into(),
        "search" | "grep" | "glob" => "search".into(),
        "fetch" | "web_fetch" | "webfetch" => "web_fetch".into(),
        "web_search" | "websearch" => "web_search".into(),
        "list" | "list_dir" | "listdir" => "list".into(),
        "" => "other".into(),
        other => other.to_owned(),
    }
}

/// Human-facing tool name: prefer description, else path, else title.
fn tool_display_name(update: &Value) -> String {
    let kind_raw = update
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("other");
    let kind = normalize_tool_kind(kind_raw);
    let title = update
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let subject = tool_description(update)
        .or_else(|| tool_path_hint(update))
        .unwrap_or_else(|| title.to_owned());
    if subject.is_empty() {
        kind
    } else {
        format!("{kind}: {subject}")
    }
}

/// Project tool `content` + `rawOutput` into UI-facing text.
///
/// Grok ACP is a dual channel (see grok-build `acp_tool_update` + pager tracker):
/// - `content` often carries **model-oriented** shapes (N→ line markers, raw
///   ANSI bash bytes, "found N matches" stubs, Diff blocks).
/// - `raw_output` is the typed `ToolOutput` the official pager prefers for UI
///   (plain `FileContent.raw_output`, ListDir listing, Grep file_matches,
///   Bash.output_for_prompt, EditsApplied, …).
///
/// Prefer structured `raw_output` projection first (pager parity), then content
/// Diff/text, and never dump whole ToolOutput JSON as the tool body.
///
/// Always strip ANSI at the end so `[31m` / `[90m` never reach UIs.
fn extract_tool_result_text(update: &Value) -> String {
    let text = extract_tool_result_text_raw(update);
    crate::strip_ansi_escapes(&text)
}

fn extract_tool_result_text_raw(update: &Value) -> String {
    let raw = update.get("rawOutput").or_else(|| update.get("raw_output"));

    // 1) Typed raw_output (pager path).
    if let Some(raw) = raw {
        if let Some(projected) = project_raw_output_text(raw) {
            return projected;
        }
    }

    // 2) ACP content blocks (Diff + text). Used when raw is absent / unknown.
    if let Some(content) = update.get("content") {
        let from_content = extract_tool_content_text(content);
        if !from_content.is_empty() {
            return from_content;
        }
    }

    // 3) String raw only — never `Value::to_string()` whole objects.
    if let Some(raw) = raw {
        if let Some(s) = raw.as_str() {
            return s.to_owned();
        }
    }

    String::new()
}

fn extract_tool_content_text(content: &Value) -> String {
    if let Some(arr) = content.as_array() {
        let mut parts: Vec<String> = Vec::new();
        for item in arr {
            let ty = item.get("type").and_then(Value::as_str).unwrap_or("");
            if ty.eq_ignore_ascii_case("diff") {
                if let Some(patch) = project_acp_diff_item(item) {
                    if !patch.is_empty() {
                        parts.push(patch);
                    }
                }
                continue;
            }
            // ACP: { type: "content", content: { type: "text", text } }
            if let Some(t) = item
                .get("content")
                .and_then(|inner| inner.get("text"))
                .and_then(Value::as_str)
                .or_else(|| item.get("text").and_then(Value::as_str))
                .or_else(|| item.get("content").and_then(Value::as_str))
            {
                if !t.is_empty() {
                    parts.push(t.to_owned());
                }
            }
        }
        return parts.join("\n");
    }
    content.as_str().map(str::to_owned).unwrap_or_default()
}

fn project_acp_diff_item(item: &Value) -> Option<String> {
    let path = item
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let new_text = item
        .get("newText")
        .or_else(|| item.get("new_text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let old_text = item
        .get("oldText")
        .or_else(|| item.get("old_text"))
        .and_then(Value::as_str)
        .unwrap_or("");

    // Prefer structured edit details in _meta when present (Grok embeds them).
    if let Some(meta) = item.get("_meta").or_else(|| item.get("meta")) {
        if let Some(details) = meta_edit_details(meta) {
            if !details.is_empty() {
                return Some(unified_diff_from_details(path, &details));
            }
        }
    }

    Some(unified_diff_from_strings(path, old_text, new_text, 1, 1))
}

fn meta_edit_details(meta: &Value) -> Option<Vec<EditDetail>> {
    let details = meta.get("details").and_then(Value::as_array).or_else(|| {
        // Sometimes meta is the SearchReplaceEditContextInformation object itself.
        meta.as_array()
    })?;
    let mut out = Vec::new();
    for d in details {
        out.push(EditDetail {
            old_string: d
                .get("old_string")
                .or_else(|| d.get("oldString"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            new_string: d
                .get("new_string")
                .or_else(|| d.get("newString"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            old_line: d
                .get("old_line")
                .or_else(|| d.get("oldLine"))
                .and_then(Value::as_u64)
                .unwrap_or(1) as usize,
            new_line: d
                .get("new_line")
                .or_else(|| d.get("newLine"))
                .and_then(Value::as_u64)
                .unwrap_or(1) as usize,
            context_before: d
                .get("context_before")
                .or_else(|| d.get("contextBefore"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            context_after: d
                .get("context_after")
                .or_else(|| d.get("contextAfter"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            line_prefix: d
                .get("line_prefix")
                .or_else(|| d.get("linePrefix"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[derive(Debug, Clone)]
struct EditDetail {
    old_string: String,
    new_string: String,
    old_line: usize,
    new_line: usize,
    context_before: String,
    context_after: String,
    line_prefix: String,
}

fn project_raw_output_text(raw: &Value) -> Option<String> {
    // ToolOutput is internally tagged: { "type": "SearchReplace", "EditsApplied": {...} }
    let ty = raw.get("type").and_then(Value::as_str).unwrap_or("");

    // ── File edits ──────────────────────────────────────────────────────
    if ty == "SearchReplace" || raw.get("EditsApplied").is_some() {
        if let Some(applied) = raw.get("EditsApplied") {
            return project_edits_applied(applied);
        }
        return search_replace_error_text(raw);
    }

    if ty == "ApplyPatch" {
        if let Some(success) = raw.get("Success") {
            return project_apply_patch_success(success);
        }
        for key in ["ParseError", "ApplicationError", "EmptyPatch"] {
            if let Some(msg) = raw.get(key).and_then(Value::as_str) {
                return Some(msg.to_owned());
            }
        }
    }

    // ── Read: prefer plain raw_output (no N→), densify numbers for gutter ─
    if ty == "ReadFile" || raw.get("FileContent").is_some() {
        if let Some(fc) = raw.get("FileContent") {
            return project_read_file_content(fc);
        }
        for key in [
            "FileNotFound",
            "IsADirectory",
            "PermissionDenied",
            "FileTooLarge",
            "FileReadError",
            "ImageSizeError",
        ] {
            if let Some(msg) = raw.get(key).and_then(Value::as_str) {
                return Some(msg.to_owned());
            }
        }
        // Image/PDF: no useful text body
        if raw.get("ImageContent").is_some() || raw.get("PdfPageImages").is_some() {
            return Some(String::new());
        }
    }

    // ── ListDir: content only lives in raw_output ───────────────────────
    if ty == "ListDir" {
        if let Some(c) = raw.get("Content") {
            if let Some(listing) = c.get("content").and_then(Value::as_str) {
                return Some(listing.to_owned());
            }
        }
        for key in [
            "NotFound",
            "IsAFile",
            "NotADirectory",
            "PermissionDenied",
            "Error",
        ] {
            if let Some(msg) = raw.get(key).and_then(Value::as_str) {
                return Some(msg.to_owned());
            }
        }
    }

    // ── Grep: content is only "found N matches"; real hits in raw ───────
    if ty == "GrepSearch" {
        return project_grep_search(raw);
    }

    // ── Bash: prefer pre-stripped output_for_prompt over raw ANSI bytes ─
    if ty == "Bash" {
        if let Some(s) = raw
            .get("output_for_prompt")
            .or_else(|| raw.get("outputForPrompt"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_owned());
        }
        // Fallback: try output as string (rare) or utf8 from byte array
        if let Some(s) = raw.get("output").and_then(Value::as_str) {
            return Some(s.to_owned());
        }
        if let Some(arr) = raw.get("output").and_then(Value::as_array) {
            let bytes: Vec<u8> = arr
                .iter()
                .filter_map(Value::as_u64)
                .map(|n| n as u8)
                .collect();
            if !bytes.is_empty() {
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }

    // ── Web ─────────────────────────────────────────────────────────────
    if ty == "WebSearch" {
        if let Some(content) = raw.get("content").and_then(Value::as_str) {
            let mut out = content.to_owned();
            if let Some(cites) = raw.get("citations").and_then(Value::as_array) {
                let links: Vec<&str> = cites.iter().filter_map(Value::as_str).collect();
                if !links.is_empty() {
                    out.push_str("\n\n");
                    for (i, u) in links.iter().enumerate() {
                        let _ = writeln!(out, "[{}] {}", i + 1, u);
                    }
                }
            }
            return Some(out);
        }
    }

    if ty == "WebFetch" {
        if let Some(c) = raw.get("Content") {
            if let Some(text) = c.get("content").and_then(Value::as_str) {
                return Some(text.to_owned());
            }
        }
        if let Some(domain) = raw.get("DomainNotAllowed").and_then(Value::as_str) {
            return Some(format!(
                "Error: domain {domain} is not in the allowed domains list"
            ));
        }
        if let Some(obj) = raw.get("CrossHostRedirect") {
            let from = obj
                .get("original_host")
                .or_else(|| obj.get("originalHost"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            let to = obj
                .get("redirect_url")
                .or_else(|| obj.get("redirectUrl"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            return Some(format!("Error: cross-host redirect from {from} to {to}"));
        }
        if let Some(obj) = raw.get("Error") {
            if let Some(msg) = obj.get("message").and_then(Value::as_str) {
                let url = obj.get("url").and_then(Value::as_str);
                return Some(match url {
                    Some(u) => format!("Error fetching URL {u}: {msg}"),
                    None => format!("Error: {msg}"),
                });
            }
            if let Some(msg) = obj.as_str() {
                return Some(msg.to_owned());
            }
        }
    }

    // ── MCP / Dynamic / Text ────────────────────────────────────────────
    if ty == "MCP" {
        // MCPOutput wraps details; try common shapes
        if let Some(s) = raw
            .get("OkayOutput")
            .or_else(|| raw.get("Error"))
            .and_then(Value::as_str)
        {
            return Some(maybe_pretty_json_str(s));
        }
        if let Some(details) = raw.get("output").or_else(|| raw.get("details")) {
            if let Some(s) = details
                .get("OkayOutput")
                .or_else(|| details.get("Error"))
                .and_then(Value::as_str)
            {
                return Some(maybe_pretty_json_str(s));
            }
        }
    }

    if ty == "Text" {
        if let Some(s) = raw.get("text").and_then(Value::as_str) {
            return Some(s.to_owned());
        }
    }

    if ty == "Dynamic" {
        if let Some(v) = raw.get("value") {
            return Some(maybe_pretty_json_value(v));
        }
    }

    if ty == "SearchTool" {
        if let Some(s) = raw.get("content").and_then(Value::as_str) {
            return Some(s.to_owned());
        }
    }

    if ty == "CodexGrepFiles" {
        if let Some(m) = raw.get("Matches") {
            if let Some(s) = m.get("content").and_then(Value::as_str) {
                return Some(s.to_owned());
            }
        }
        for key in ["NoMatches", "Error"] {
            if let Some(msg) = raw.get(key).and_then(Value::as_str) {
                return Some(msg.to_owned());
            }
        }
    }

    if ty == "BackgroundTaskStarted" {
        if let Some(s) = raw.get("summary").and_then(Value::as_str) {
            return Some(s.to_owned());
        }
    }

    if ty == "Skill" {
        if let Some(s) = raw
            .get("tool_result")
            .or_else(|| raw.get("toolResult"))
            .and_then(Value::as_str)
        {
            return Some(s.to_owned());
        }
    }

    // Media tools: path / message prose
    if matches!(
        ty,
        "ImageGen" | "ImageToVideo" | "ReferenceToVideo" | "ImageEdit"
    ) {
        if let Some(s) = raw
            .get("path")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return Some(format!("saved: {s}"));
        }
        if let Some(s) = raw
            .get("uploaded_url")
            .or_else(|| raw.get("uploadedUrl"))
            .and_then(Value::as_str)
        {
            return Some(format!("uploaded: {s}"));
        }
    }

    // Generic prose fields (ListDir-like nested tools, write confirmations).
    if let Some(s) = raw
        .get("tool_output_for_prompt_concise")
        .or_else(|| raw.get("toolOutputForPromptConcise"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(s.to_owned());
    }
    if let Some(s) = raw
        .get("tool_output_for_prompt")
        .or_else(|| raw.get("toolOutputForPrompt"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(s.to_owned());
    }

    None
}

/// Grok `FileContent`: UI uses plain `raw_output` (pager does too). Densify
/// `{offset+1+i}→line` so Desktop/TUI gutters show real file line numbers
/// without the sparse decade markers from model-facing `content`.
fn project_read_file_content(fc: &Value) -> Option<String> {
    let plain = fc
        .get("raw_output")
        .or_else(|| fc.get("rawOutput"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            // Fall back to model content with arrows stripped if raw missing.
            fc.get("content")
                .and_then(Value::as_str)
                .map(strip_arrow_line_markers)
        })?;

    let offset = fc.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let start = offset.saturating_add(1);

    // Always densify so gutters are correct for offset reads.
    let lines = split_diff_lines(&plain);
    if lines.is_empty() {
        return Some(format!("{start}→"));
    }
    let mut out = String::with_capacity(plain.len() + lines.len() * 8);
    for (i, line) in lines.iter().enumerate() {
        let n = start + i;
        let _ = writeln!(out, "{n}→{line}");
    }
    // Keep trailing newline consistency with source.
    if !plain.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

/// Strip sparse/dense `N→` markers, expanding unnumbered lines sequentially.
fn strip_arrow_line_markers(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut next: Option<usize> = None;
    for line in lines {
        if let Some((num, rest)) = line.split_once('→') {
            if !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(n) = num.parse::<usize>() {
                    out.push(rest.to_owned());
                    next = Some(n + 1);
                    continue;
                }
            }
        }
        if next.is_some() {
            out.push(line.to_owned());
            if let Some(n) = next.as_mut() {
                *n += 1;
            }
        } else {
            out.push(line.to_owned());
        }
    }
    out.join("\n")
}

fn project_grep_search(raw: &Value) -> Option<String> {
    if let Some(files) = raw.get("file_matches").or_else(|| raw.get("fileMatches")) {
        if let Some(arr) = files.as_array() {
            if !arr.is_empty() {
                let mut out = String::new();
                for f in arr {
                    let path = f.get("path").and_then(Value::as_str).unwrap_or("?");
                    let matches = f.get("matches").and_then(Value::as_array);
                    if let Some(matches) = matches {
                        for m in matches {
                            let ln = m
                                .get("line_number")
                                .or_else(|| m.get("lineNumber"))
                                .and_then(Value::as_u64)
                                .unwrap_or(0);
                            let content = m.get("content").and_then(Value::as_str).unwrap_or("");
                            let _ = writeln!(out, "{path}:{ln}:{content}");
                        }
                    } else {
                        out.push_str(path);
                        out.push('\n');
                    }
                }
                if !out.is_empty() {
                    return Some(out);
                }
            }
        }
    }

    // Decode stdout bytes when present (may include <workspace_result> wrapper).
    if let Some(arr) = raw.get("stdout").and_then(Value::as_array) {
        let bytes: Vec<u8> = arr
            .iter()
            .filter_map(Value::as_u64)
            .map(|n| n as u8)
            .collect();
        if !bytes.is_empty() {
            let s = String::from_utf8_lossy(&bytes);
            return Some(strip_workspace_result_wrapper(&s));
        }
    }
    if let Some(s) = raw.get("stdout").and_then(Value::as_str) {
        return Some(strip_workspace_result_wrapper(s));
    }

    let count = raw
        .get("match_count")
        .or_else(|| raw.get("matchCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(format!("found {count} matches"))
}

/// Grep model prompt wraps hits in `<workspace_result …>…</workspace_result>`.
fn strip_workspace_result_wrapper(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(start) = trimmed.find('>') {
        if trimmed.starts_with("<workspace_result") {
            let inner = &trimmed[start + 1..];
            if let Some(end) = inner.rfind("</workspace_result>") {
                return inner[..end].trim().to_owned();
            }
        }
    }
    s.to_owned()
}

fn maybe_pretty_json_str(s: &str) -> String {
    let t = s.trim();
    if t.starts_with('{') || t.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Value>(t) {
            return serde_json::to_string_pretty(&v).unwrap_or_else(|_| s.to_owned());
        }
    }
    s.to_owned()
}

fn maybe_pretty_json_value(v: &Value) -> String {
    match v {
        Value::String(s) => maybe_pretty_json_str(s),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn search_replace_error_text(raw: &Value) -> Option<String> {
    for key in [
        "FileNotFound",
        "FileAlreadyExists",
        "MultipleMatchesFound",
        "InvalidInput",
        "FilenameTooLong",
    ] {
        if let Some(msg) = raw.get(key).and_then(Value::as_str) {
            return Some(msg.to_owned());
        }
    }
    if let Some(obj) = raw.get("NoMatchesFound") {
        if let Some(msg) = obj.get("message").and_then(Value::as_str) {
            return Some(msg.to_owned());
        }
        if let Some(msg) = obj.as_str() {
            return Some(msg.to_owned());
        }
    }
    None
}

fn project_edits_applied(applied: &Value) -> Option<String> {
    let path = applied
        .get("absolute_path")
        .or_else(|| applied.get("absolutePath"))
        .and_then(Value::as_str)
        .unwrap_or("");

    // edits: { details: [...] }
    if let Some(edits) = applied.get("edits") {
        if let Some(details) = meta_edit_details(edits) {
            let patch = unified_diff_from_details(path, &details);
            if !patch.is_empty() {
                return Some(patch);
            }
        }
    }

    if let Some(patch) = applied.get("patch").and_then(Value::as_str) {
        if !patch.trim().is_empty() {
            return Some(patch.to_owned());
        }
    }

    let old = applied
        .get("old_string")
        .or_else(|| applied.get("oldString"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let new = applied
        .get("new_string")
        .or_else(|| applied.get("newString"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if !old.is_empty() || !new.is_empty() {
        let patch = unified_diff_from_strings(path, old, new, 1, 1);
        if !patch.is_empty() {
            return Some(patch);
        }
    }

    // Fall back to concise/prose confirmation (not the full EditsApplied object).
    if let Some(s) = applied
        .get("tool_output_for_prompt_concise")
        .or_else(|| applied.get("toolOutputForPromptConcise"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(s.to_owned());
    }
    if let Some(s) = applied
        .get("tool_output_for_prompt")
        .or_else(|| applied.get("toolOutputForPrompt"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(s.to_owned());
    }
    None
}

fn project_apply_patch_success(success: &Value) -> Option<String> {
    let files = success.get("files").and_then(Value::as_array)?;
    let mut patches = Vec::new();
    for f in files {
        let path = f
            .get("path")
            .or_else(|| f.get("move_to").or_else(|| f.get("moveTo")))
            .and_then(Value::as_str)
            .unwrap_or("");
        let old = f
            .get("old_text")
            .or_else(|| f.get("oldText"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let new = f
            .get("new_text")
            .or_else(|| f.get("newText"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let patch = unified_diff_from_strings(path, old, new, 1, 1);
        if !patch.is_empty() {
            patches.push(patch);
        }
    }
    if !patches.is_empty() {
        return Some(patches.join("\n"));
    }
    success
        .get("tool_output_for_prompt")
        .or_else(|| success.get("toolOutputForPrompt"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn display_diff_path(path: &str) -> String {
    let p = path.trim();
    if p.is_empty() {
        return "file".into();
    }
    // Strip leading ./ for cleaner headers; keep absolute paths as-is for uniqueness.
    p.trim_start_matches("./").to_owned()
}

fn split_diff_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    // Drop the empty trailing segment created by a final newline so
    // `"a\nb\n"` → ["a", "b"] rather than ["a", "b", ""].
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// Build a unified patch from old/new text with a shared-prefix/suffix line diff.
fn unified_diff_from_strings(
    path: &str,
    old: &str,
    new: &str,
    old_start: usize,
    new_start: usize,
) -> String {
    if old == new {
        return String::new();
    }
    let path = display_diff_path(path);
    let old_lines = split_diff_lines(old);
    let new_lines = split_diff_lines(new);

    // Common prefix / suffix so small mid-file edits don't dump entire blocks as
    // pure delete+insert when old/new are full-file snapshots.
    let mut pref = 0usize;
    while pref < old_lines.len() && pref < new_lines.len() && old_lines[pref] == new_lines[pref] {
        pref += 1;
    }
    let mut suf = 0usize;
    while suf < (old_lines.len() - pref)
        && suf < (new_lines.len() - pref)
        && old_lines[old_lines.len() - 1 - suf] == new_lines[new_lines.len() - 1 - suf]
    {
        suf += 1;
    }

    let context_before = 3usize.min(pref);
    let context_after = 3usize.min(suf);
    let old_hunk_start = old_start.saturating_add(pref.saturating_sub(context_before));
    let new_hunk_start = new_start.saturating_add(pref.saturating_sub(context_before));
    let old_change = &old_lines[pref..old_lines.len() - suf];
    let new_change = &new_lines[pref..new_lines.len() - suf];
    let before = &old_lines[pref.saturating_sub(context_before)..pref];
    let after = if suf == 0 {
        &old_lines[old_lines.len()..]
    } else {
        &old_lines[old_lines.len() - suf..old_lines.len() - suf + context_after]
    };

    let old_count = before.len() + old_change.len() + after.len();
    let new_count = before.len() + new_change.len() + after.len();

    let mut out = String::new();
    let _ = write!(out, "--- a/{path}\n+++ b/{path}\n");
    let _ = writeln!(
        out,
        "@@ -{},{} +{},{} @@",
        if old_count == 0 {
            0
        } else {
            old_hunk_start.max(1)
        },
        old_count,
        if new_count == 0 {
            0
        } else {
            new_hunk_start.max(1)
        },
        new_count
    );
    for line in before {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    for line in old_change {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in new_change {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    for line in after {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn unified_diff_from_details(path: &str, details: &[EditDetail]) -> String {
    if details.is_empty() {
        return String::new();
    }
    let path = display_diff_path(path);
    let mut out = String::new();
    let _ = write!(out, "--- a/{path}\n+++ b/{path}\n");

    for d in details {
        let before_lines = split_diff_lines(&d.context_before);
        let after_lines = split_diff_lines(&d.context_after);
        let mut old_body = d.old_string.clone();
        let mut new_body = d.new_string.clone();
        // Align mid-line matches with the leading indent Grok captured.
        if !d.line_prefix.is_empty() {
            if !old_body.is_empty() && !old_body.starts_with(&d.line_prefix) {
                old_body = format!("{}{old_body}", d.line_prefix);
            }
            if !new_body.is_empty() && !new_body.starts_with(&d.line_prefix) {
                new_body = format!("{}{new_body}", d.line_prefix);
            }
        }
        let old_lines = split_diff_lines(&old_body);
        let new_lines = split_diff_lines(&new_body);

        let old_count = before_lines.len() + old_lines.len() + after_lines.len();
        let new_count = before_lines.len() + new_lines.len() + after_lines.len();
        let old_start = if old_count == 0 { 0 } else { d.old_line.max(1) };
        let new_start = if new_count == 0 { 0 } else { d.new_line.max(1) };

        let _ = writeln!(
            out,
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@"
        );
        for line in &before_lines {
            out.push(' ');
            out.push_str(line);
            out.push('\n');
        }
        for line in &old_lines {
            out.push('-');
            out.push_str(line);
            out.push('\n');
        }
        for line in &new_lines {
            out.push('+');
            out.push_str(line);
            out.push('\n');
        }
        for line in &after_lines {
            out.push(' ');
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn compact_tool_args_json(update: &Value) -> String {
    // Prefer a compact projection: kind/title/description/path/rawInput slice.
    let mut obj = serde_json::Map::new();
    if let Some(k) = update.get("kind") {
        obj.insert("kind".into(), k.clone());
    }
    if let Some(t) = update.get("title") {
        obj.insert("title".into(), t.clone());
    }
    if let Some(d) = tool_description(update) {
        obj.insert("description".into(), Value::String(d));
    }
    if let Some(p) = tool_path_hint(update) {
        obj.insert("path".into(), Value::String(p));
    }
    if let Some(locs) = update.get("locations") {
        obj.insert("locations".into(), locs.clone());
    }
    if let Some(raw) = tool_raw_input(update) {
        obj.insert("rawInput".into(), raw.clone());
    }
    if let Some(st) = update.get("status") {
        obj.insert("status".into(), st.clone());
    }
    // Keep full update only when compact view is empty.
    if obj.len() <= 1 {
        return serde_json::to_string(update).unwrap_or_else(|_| "{}".into());
    }
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| "{}".into())
}

/// Only emit turn-activity for suppressed tools (otherwise scroll/timeline noise).
fn suppressed_activity_raw(activity: &str, extra: Value) -> UiEventMessage {
    let mut map = serde_json::Map::new();
    map.insert("activity".into(), Value::String(activity.into()));
    map.insert("suppressed".into(), Value::Bool(true));
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            map.insert(k.clone(), v.clone());
        }
    }
    UiEventMessage::Raw {
        kind: "grok/turn_activity".into(),
        payload_json: Value::Object(map).to_string(),
    }
}

#[allow(clippy::too_many_lines)]
pub fn translate(
    state: &mut GrokTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    // Minos synthetic envelopes shared with Codex (approval overlay path).
    if let Some(method) = raw.get("method").and_then(Value::as_str) {
        match method {
            "approval/request" | "approval/timeout" => {
                let params = raw.get("params").cloned().unwrap_or(Value::Null);
                return Ok(vec![UiEventMessage::Raw {
                    kind: method.to_string(),
                    payload_json: serde_json::to_string(&params).unwrap_or_default(),
                }]);
            }
            _ => {}
        }
    }

    let kind =
        raw.get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslationError::Malformed {
                reason: "missing kind field".into(),
            })?;

    match kind {
        "user_message" => Ok(translate_user_message(state, raw)),
        "acp_notification" => translate_acp_notification(state, raw),
        "acp_server_request" => translate_acp_server_request(state, raw),
        "acp_prompt_response" => Ok(translate_acp_prompt_response(state, raw)),
        "acp_error" => Ok(vec![UiEventMessage::Error {
            code: raw
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("grok_acp_error")
                .to_string(),
            message: raw
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Grok ACP reported an error")
                .to_string(),
            message_id: state
                .open_assistant_message_id
                .clone()
                .or_else(|| state.open_user_message_id.clone()),
        }]),
        "acp_closed" => {
            let finished = now_ms();
            let mut events = complete_open_assistant_message(state, finished);
            state.tool_calls.clear();
            state.suppressed_tools.clear();
            state.orphan_updates.clear();
            state.force_new_assistant_on_text = false;
            state.last_stream_start_ms = None;
            events.push(UiEventMessage::ThreadClosed {
                thread_id: state.thread_id.clone(),
                reason: ThreadEndReason::AgentDone,
                closed_at_ms: finished,
            });
            Ok(events)
        }
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("grok/{other}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

fn translate_user_message(state: &mut GrokTranslatorState, raw: &Value) -> Vec<UiEventMessage> {
    let message_id = raw
        .get("messageId")
        .and_then(Value::as_str)
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_string);
    if !state.emitted_message_ids.insert(message_id.clone()) {
        return vec![];
    }
    let finished = raw
        .get("createdAtMs")
        .and_then(Value::as_i64)
        .unwrap_or_else(now_ms);
    let mut events = complete_open_assistant_message(state, finished);
    state.force_new_assistant_on_text = false;
    state.open_user_message_id = Some(message_id.clone());
    let text = raw
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    events.push(UiEventMessage::MessageStarted {
        message_id: message_id.clone(),
        role: MessageRole::User,
        started_at_ms: raw.get("createdAtMs").and_then(Value::as_i64).unwrap_or(0),
    });
    if !text.is_empty() {
        events.push(UiEventMessage::TextDelta {
            message_id,
            text: DisplayPayload::inline(text),
        });
    }
    events
}

fn translate_acp_prompt_response(
    state: &mut GrokTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let stop_reason = raw
        .get("stopReason")
        .and_then(Value::as_str)
        .unwrap_or("end_turn");
    let finished = now_ms();
    match stop_reason {
        "end_turn" => {
            state.tool_calls.clear();
            state.suppressed_tools.clear();
            state.orphan_updates.clear();
            state.force_new_assistant_on_text = false;
            complete_open_assistant_message(state, finished)
        }
        "cancelled" => {
            let mut events = complete_open_assistant_message(state, finished);
            state.tool_calls.clear();
            state.suppressed_tools.clear();
            state.orphan_updates.clear();
            state.force_new_assistant_on_text = false;
            events.push(UiEventMessage::ThreadClosed {
                thread_id: state.thread_id.clone(),
                reason: ThreadEndReason::UserStopped,
                closed_at_ms: finished,
            });
            events
        }
        other => vec![UiEventMessage::Error {
            code: format!("grok_stop_reason_{other}"),
            message: format!("Grok stopped with reason: {other}"),
            message_id: state.open_assistant_message_id.clone(),
        }],
    }
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::unnecessary_wraps)]
fn translate_acp_notification(
    state: &mut GrokTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let params = raw.get("params").cloned().unwrap_or(Value::Null);
    let (meta, mut prefix_events) = apply_notification_meta(state, &params);
    let update = params.get("update").cloned().unwrap_or(Value::Null);
    let session_update = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or("");

    let body_events = match session_update {
        "agent_message_chunk" | "agent_thought_chunk" => {
            translate_agent_or_thought_chunk(state, &update, session_update, &meta)
        }
        "tool_call" => translate_tool_call(state, &update, &meta),
        "tool_call_update" => translate_tool_call_update(state, &update),
        "plan" => vec![UiEventMessage::Raw {
            kind: "grok/plan".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }],
        "thought" => {
            // Legacy / alternate thought shape.
            let content = update.get("content").cloned().unwrap_or(Value::Null);
            let text = content
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                vec![]
            } else {
                let (mid, mut events) = begin_text_segment(state, &meta);
                events.push(UiEventMessage::ReasoningDelta {
                    message_id: mid,
                    text: DisplayPayload::inline(text),
                });
                events
            }
        }
        "current_mode_update" => vec![UiEventMessage::Raw {
            kind: "grok/mode_change".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }],
        "available_commands_update" => vec![UiEventMessage::Raw {
            kind: "grok/commands_update".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }],
        "session_info_update" => vec![UiEventMessage::Raw {
            kind: "grok/session_info".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }],
        "user_message_chunk" => {
            // Echo of user prompt over ACP — Minos already records user_message.
            vec![]
        }
        "subagent_progress" => translate_subagent_progress(state, &update),
        "subagent_finished" => translate_subagent_finished(state, &update),
        "" => vec![],
        other => vec![UiEventMessage::Raw {
            kind: format!("grok/acp/{other}"),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }],
    };

    prefix_events.extend(body_events);
    Ok(prefix_events)
}

fn translate_agent_or_thought_chunk(
    state: &mut GrokTranslatorState,
    update: &Value,
    session_update: &str,
    meta: &NotificationMeta,
) -> Vec<UiEventMessage> {
    let content = update.get("content").cloned().unwrap_or(Value::Null);
    let content_type = content
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("text");
    let text = content
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if text.is_empty() {
        return vec![];
    }

    // Whitespace-only first chunk with no open message — ignore (official pager).
    if state.open_assistant_message_id.is_none()
        && state.force_new_assistant_on_text
        && text.trim().is_empty()
    {
        return vec![];
    }

    let is_thought = session_update == "agent_thought_chunk"
        || content_type == "thought"
        || (session_update != "agent_message_chunk" && content_type != "text");

    if is_thought && session_update == "agent_thought_chunk" {
        // Thought does not force a new assistant text segment the way tools do,
        // but does not close agent text either until stream boundary / tool.
        let (mid, mut events) =
            ensure_assistant_message(state, meta.agent_timestamp_ms.unwrap_or(0));
        events.push(UiEventMessage::ReasoningDelta {
            message_id: mid,
            text: DisplayPayload::inline(text),
        });
        return events;
    }

    if session_update == "agent_message_chunk" && content_type == "text" {
        let (mid, mut events) = begin_text_segment(state, meta);
        events.push(UiEventMessage::TextDelta {
            message_id: mid,
            text: DisplayPayload::inline(text),
        });
        return events;
    }

    // Fallback thought-like content.
    let (mid, mut events) = ensure_assistant_message(state, meta.agent_timestamp_ms.unwrap_or(0));
    events.push(UiEventMessage::ReasoningDelta {
        message_id: mid,
        text: DisplayPayload::inline(text),
    });
    events
}

fn translate_tool_call(
    state: &mut GrokTranslatorState,
    update: &Value,
    meta: &NotificationMeta,
) -> Vec<UiEventMessage> {
    let tool_call_id = update
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if tool_call_id.is_empty() {
        return vec![];
    }

    let title = update
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let kind = update
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("other")
        .to_string();
    let raw_input = tool_raw_input(update).cloned();
    let suppressed = is_suppressed_tool(&title, &kind, raw_input.as_ref());
    let display_name = tool_display_name(update);
    let finished = meta.agent_timestamp_ms.unwrap_or_else(now_ms);

    // Official: tool_call clears current agent text entry.
    let mut events = complete_open_assistant_message(state, finished);
    state.force_new_assistant_on_text = true;

    if suppressed {
        state.suppressed_tools.insert(tool_call_id.clone());
        state.tool_calls.insert(
            tool_call_id.clone(),
            OpenGrokToolCall {
                message_id: String::new(),
                name: display_name.clone(),
                suppressed: true,
            },
        );
        let wait = waiting_reason_for_suppressed(&title, raw_input.as_ref());
        events.push(suppressed_activity_raw(
            &wait,
            serde_json::json!({
                "toolCallId": tool_call_id,
                "title": title,
            }),
        ));
        // Task/spawn still surface as subagent when metadata present.
        if title.eq_ignore_ascii_case("spawn_subagent")
            || title.eq_ignore_ascii_case("task")
            || title == "Task"
        {
            if let Some(raw) = raw_input.as_ref() {
                if let Some(sub_id) = raw
                    .get("subagent_id")
                    .or_else(|| raw.get("task_id"))
                    .and_then(Value::as_str)
                {
                    let desc = tool_description(update);
                    if let Some(spawned) = maybe_spawn_subagent(
                        state,
                        sub_id.to_owned(),
                        tool_call_id,
                        desc.clone(),
                        desc,
                    ) {
                        events.push(spawned);
                    }
                }
            }
        }
        return events;
    }

    // Orphan update already completed this tool — place as completed immediately.
    if let Some(orphan) = state.orphan_updates.remove(&tool_call_id) {
        let (mid, start_events) =
            ensure_assistant_message(state, meta.agent_timestamp_ms.unwrap_or(0));
        events.extend(start_events);
        state.tool_calls.insert(
            tool_call_id.clone(),
            OpenGrokToolCall {
                message_id: mid.clone(),
                name: display_name.clone(),
                suppressed: false,
            },
        );
        events.push(UiEventMessage::ToolCallPlaced {
            message_id: mid,
            tool_call_id: tool_call_id.clone(),
            name: display_name,
            args_json: DisplayPayload::inline(compact_tool_args_json(update)),
        });
        // Orphans only carry terminal statuses (completed/failed/cancelled) —
        // derive error flag from the orphan payload so failed tools are not
        // mislabeled as successful on replay.
        let orphan_is_error = orphan
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|s| s == "failed" || s == "cancelled");
        events.extend(complete_tool_from_update(
            state,
            &tool_call_id,
            &orphan,
            orphan_is_error,
        ));
        // `complete_tool_from_update` already closed the open assistant message
        // and set force_new_assistant_on_text; nothing else to do here.
        return events;
    }

    // Already-completed tool_call (status on place).
    let status = update
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("pending");
    let (mid, start_events) = ensure_assistant_message(state, meta.agent_timestamp_ms.unwrap_or(0));
    events.extend(start_events);
    state.tool_calls.insert(
        tool_call_id.clone(),
        OpenGrokToolCall {
            message_id: mid.clone(),
            name: display_name.clone(),
            suppressed: false,
        },
    );
    events.push(UiEventMessage::ToolCallPlaced {
        message_id: mid,
        tool_call_id: tool_call_id.clone(),
        name: display_name,
        args_json: DisplayPayload::inline(compact_tool_args_json(update)),
    });

    if matches!(status, "completed" | "failed") {
        let content = extract_tool_result_text(update);
        let is_error = status == "failed";
        let open = state.tool_calls.remove(&tool_call_id);
        events.push(UiEventMessage::ToolCallCompleted {
            tool_call_id: tool_call_id.clone(),
            output: DisplayPayload::inline(content.clone()),
            is_error,
        });
        if open
            .as_ref()
            .is_some_and(|t| t.name.to_ascii_lowercase().contains("spawn_subagent"))
            || content.contains("subagent_id:")
        {
            if let Some(spawned) = maybe_spawn_from_tool_output(state, &tool_call_id, &content) {
                events.push(spawned);
            }
        }
        let _ = complete_open_assistant_message(state, finished);
        state.force_new_assistant_on_text = true;
    }

    events
}

fn waiting_reason_for_suppressed(title: &str, raw_input: Option<&Value>) -> String {
    let variant = raw_input
        .and_then(|r| r.get("variant"))
        .and_then(Value::as_str);
    if matches!(
        title,
        "get_command_or_subagent_output" | "get_task_output" | "get_task_or_subagent_output"
    ) || variant == Some("TaskOutput")
    {
        return "waiting_task_output".into();
    }
    if matches!(
        title,
        "wait_commands_or_subagents" | "wait_tasks" | "wait_tasks_or_subagents"
    ) || title.starts_with("Wait tasks:")
        || variant == Some("WaitTasks")
    {
        return "waiting_tasks_complete".into();
    }
    if matches!(title, "Await" | "AwaitShell")
        || title.starts_with("Await:")
        || title.starts_with("Sleep ")
    {
        return "waiting_sleep".into();
    }
    if matches!(title, "spawn_subagent" | "task" | "Task") || variant == Some("Task") {
        return "waiting_subagent".into();
    }
    "waiting_model".into()
}

fn translate_tool_call_update(
    state: &mut GrokTranslatorState,
    update: &Value,
) -> Vec<UiEventMessage> {
    let tool_call_id = update
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if tool_call_id.is_empty() {
        return vec![];
    }

    let status = update
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // Suppressed tools: clear bookkeeping on terminal status, no UI tool events.
    if state.suppressed_tools.contains(&tool_call_id)
        || state
            .tool_calls
            .get(&tool_call_id)
            .is_some_and(|t| t.suppressed)
    {
        if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
            state.suppressed_tools.remove(&tool_call_id);
            let open = state.tool_calls.remove(&tool_call_id);
            let content = extract_tool_result_text(update);
            // spawn_subagent may only complete via update.
            if open
                .as_ref()
                .is_some_and(|t| t.name.to_ascii_lowercase().contains("spawn_subagent"))
                || content.contains("subagent_id:")
            {
                if let Some(spawned) = maybe_spawn_from_tool_output(state, &tool_call_id, &content)
                {
                    return vec![spawned];
                }
            }
        }
        return vec![];
    }

    if !matches!(status.as_str(), "completed" | "failed" | "cancelled") {
        // In-progress streaming output — stash as raw for debugging only when useful.
        return vec![];
    }

    if !state.tool_calls.contains_key(&tool_call_id) {
        // Orphan: remember until tool_call arrives.
        state.orphan_updates.insert(tool_call_id, update.clone());
        return vec![];
    }

    complete_tool_from_update(
        state,
        &tool_call_id,
        update,
        status == "failed" || status == "cancelled",
    )
}

fn complete_tool_from_update(
    state: &mut GrokTranslatorState,
    tool_call_id: &str,
    update: &Value,
    is_error: bool,
) -> Vec<UiEventMessage> {
    let content = extract_tool_result_text(update);

    let open = state.tool_calls.remove(tool_call_id);
    let mut events = vec![UiEventMessage::ToolCallCompleted {
        tool_call_id: tool_call_id.to_owned(),
        output: DisplayPayload::inline(content.clone()),
        is_error,
    }];
    if open
        .as_ref()
        .is_some_and(|t| t.name.to_ascii_lowercase().contains("spawn_subagent"))
        || content.contains("subagent_id:")
    {
        if let Some(spawned) = maybe_spawn_from_tool_output(state, tool_call_id, &content) {
            events.push(spawned);
        }
    }
    // Close tool shell message so next agent text is a fresh segment.
    events.extend(complete_open_assistant_message(state, now_ms()));
    state.force_new_assistant_on_text = true;
    events
}

fn child_session_id(update: &Value) -> Option<String> {
    update
        .get("child_session_id")
        .or_else(|| update.get("subagent_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn maybe_spawn_subagent(
    state: &mut GrokTranslatorState,
    sub_thread_id: String,
    tool_call_id: String,
    title: Option<String>,
    prompt: Option<String>,
) -> Option<UiEventMessage> {
    if state.known_subagents.contains_key(&sub_thread_id) {
        return None;
    }
    state.known_subagents.insert(
        sub_thread_id.clone(),
        GrokSubagentMeta {
            tool_call_id: tool_call_id.clone(),
            title: title.clone(),
        },
    );
    Some(UiEventMessage::SubagentSpawned {
        parent_thread_id: state.thread_id.clone(),
        sub_thread_id,
        tool_call_id,
        agent: AgentName::Grok,
        model: None,
        prompt,
        title,
    })
}

fn maybe_spawn_from_tool_output(
    state: &mut GrokTranslatorState,
    tool_call_id: &str,
    content: &str,
) -> Option<UiEventMessage> {
    let sub_id = content.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("subagent_id:")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    })?;
    let description = content.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("description:")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    });
    maybe_spawn_subagent(
        state,
        sub_id,
        tool_call_id.to_owned(),
        description.clone(),
        description,
    )
}

fn translate_subagent_progress(
    state: &mut GrokTranslatorState,
    update: &Value,
) -> Vec<UiEventMessage> {
    let Some(sub_id) = child_session_id(update) else {
        return Vec::new();
    };
    let title = update
        .get("description")
        .or_else(|| update.get("title"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut events = Vec::new();
    if let Some(spawned) = maybe_spawn_subagent(state, sub_id.clone(), String::new(), title, None) {
        events.push(spawned);
    }
    events.push(UiEventMessage::SubagentStatusUpdated {
        sub_thread_id: sub_id,
        status: SubagentStatus::Running,
    });
    // Compact progress metrics only (no per-chunk activity spam).
    events.push(UiEventMessage::Raw {
        kind: "grok/acp/subagent_progress".into(),
        payload_json: serde_json::to_string(update).unwrap_or_default(),
    });
    events
}

fn translate_subagent_finished(
    state: &mut GrokTranslatorState,
    update: &Value,
) -> Vec<UiEventMessage> {
    let Some(sub_id) = child_session_id(update) else {
        return Vec::new();
    };
    let title = update
        .get("description")
        .or_else(|| update.get("title"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut events = Vec::new();
    if let Some(spawned) = maybe_spawn_subagent(state, sub_id.clone(), String::new(), title, None) {
        events.push(spawned);
    }
    let failed = update
        .get("error_count")
        .and_then(Value::as_u64)
        .is_some_and(|n| n > 0)
        || update
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    events.push(UiEventMessage::SubagentStatusUpdated {
        sub_thread_id: sub_id,
        status: if failed {
            SubagentStatus::Failed
        } else {
            SubagentStatus::Completed
        },
    });
    events.push(UiEventMessage::Raw {
        kind: "grok/acp/subagent_finished".into(),
        payload_json: serde_json::to_string(update).unwrap_or_default(),
    });
    events
}

#[allow(clippy::unnecessary_wraps)]
fn translate_acp_server_request(
    state: &mut GrokTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let method = raw.get("method").and_then(Value::as_str).unwrap_or("");

    match method {
        "session/request_permission" => {
            let params = raw.get("params").cloned().unwrap_or(Value::Null);
            let tool_call = params.get("toolCall").cloned().unwrap_or(Value::Null);
            let tool_call_id = tool_call
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let display_name = if tool_call_id.is_empty() {
                String::new()
            } else {
                tool_display_name(&tool_call)
            };

            if tool_call_id.is_empty() {
                Ok(vec![UiEventMessage::Raw {
                    kind: "grok/permission_request".into(),
                    payload_json: serde_json::to_string(raw).unwrap_or_default(),
                }])
            } else {
                // Do not complete the open text mid for permission — user still sees context.
                let (mid, mut events) = ensure_assistant_message(state, 0);
                state.tool_calls.insert(
                    tool_call_id.clone(),
                    OpenGrokToolCall {
                        message_id: mid.clone(),
                        name: display_name.clone(),
                        suppressed: false,
                    },
                );
                events.push(UiEventMessage::ToolCallPlaced {
                    message_id: mid,
                    tool_call_id,
                    name: display_name,
                    args_json: DisplayPayload::inline(compact_tool_args_json(&tool_call)),
                });
                Ok(events)
            }
        }
        "fs/read_text_file" | "fs/write_text_file" => Ok(vec![UiEventMessage::Raw {
            kind: format!("grok/{method}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
        _ => Ok(vec![UiEventMessage::Raw {
            kind: format!("grok/server_request/{method}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::*;

    fn val(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    fn assistant_id(events: &[UiEventMessage]) -> String {
        events
            .iter()
            .find_map(|event| match event {
                UiEventMessage::MessageStarted {
                    message_id,
                    role: MessageRole::Assistant,
                    ..
                } => Some(message_id.clone()),
                _ => None,
            })
            .expect("assistant MessageStarted")
    }

    #[test]
    fn acp_notification_agent_message_chunk_text() {
        let mut s = GrokTranslatorState::new("thr_x".into());
        let raw = val(
            r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello"}}}}"#,
        );
        let out = translate(&mut s, &raw).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::MessageStarted {
                role: MessageRole::Assistant,
                ..
            }
        )));
        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::TextDelta { text, .. } if text == "Hello")));
    }

    #[test]
    fn agent_msg_resets_after_tool() {
        // Mirrors grok-build AcpUpdateTracker::agent_msg_resets_after_tool.
        let mut s = GrokTranslatorState::new("thr".into());
        let before = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Before tool"}}}}"#,
            ),
        )
        .unwrap();
        let mid1 = assistant_id(&before);

        let tool = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc1","title":"file.rs","kind":"read","status":"completed","content":[]}}}"#,
            ),
        )
        .unwrap();
        assert!(tool.iter().any(
            |e| matches!(e, UiEventMessage::MessageCompleted { message_id, .. } if message_id == &mid1)
        ));
        assert!(tool
            .iter()
            .any(|e| matches!(e, UiEventMessage::ToolCallPlaced { tool_call_id, .. } if tool_call_id == "tc1")));

        let after = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"After tool"}}}}"#,
            ),
        )
        .unwrap();
        let mid2 = assistant_id(&after);
        assert_ne!(mid1, mid2);
        assert!(after.iter().any(
            |e| matches!(e, UiEventMessage::TextDelta { message_id, text, .. } if message_id == &mid2 && text == "After tool")
        ));
    }

    #[test]
    fn stream_start_ms_change_closes_open_assistant_message() {
        let mut s = GrokTranslatorState::new("thr".into());
        let first = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"_meta":{"streamStartMs":1000,"agentTimestampMs":1100},"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"stream A"}}}}"#,
            ),
        )
        .unwrap();
        let mid_a = assistant_id(&first);

        let second = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"_meta":{"streamStartMs":2000,"agentTimestampMs":2100},"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"stream B"}}}}"#,
            ),
        )
        .unwrap();
        assert!(second.iter().any(
            |e| matches!(e, UiEventMessage::MessageCompleted { message_id, .. } if message_id == &mid_a)
        ));
        let mid_b = second
            .iter()
            .rev()
            .find_map(|event| match event {
                UiEventMessage::MessageStarted {
                    message_id,
                    role: MessageRole::Assistant,
                    ..
                } => Some(message_id.clone()),
                _ => None,
            })
            .unwrap();
        assert_ne!(mid_a, mid_b);
        // Meta is applied for segmentation only — no Raw flood.
        assert!(!second.iter().any(
            |e| matches!(e, UiEventMessage::Raw { kind, .. } if kind == "grok/notification_meta")
        ));
    }

    #[test]
    fn plumbing_tools_are_suppressed() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"todo1","title":"TodoWrite","kind":"other","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        assert!(!out
            .iter()
            .any(|e| matches!(e, UiEventMessage::ToolCallPlaced { .. })));
        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::Raw { kind, payload_json }
                if kind == "grok/turn_activity" && payload_json.contains("suppressed"))));
    }

    #[test]
    fn tool_display_prefers_raw_input_description() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc1","title":"bash","kind":"execute","status":"pending","rawInput":{"description":"Run unit tests","command":"cargo test"}}}}"#,
            ),
        )
        .unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::ToolCallPlaced { name, .. } if name.contains("Run unit tests")
        )));
    }

    #[test]
    fn orphan_tool_update_then_tool_call_completes() {
        let mut s = GrokTranslatorState::new("thr".into());
        let early = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"late1","status":"completed","content":[{"content":{"type":"text","text":"ok"}}]}}}"#,
            ),
        )
        .unwrap();
        assert!(
            early.is_empty()
                || !early
                    .iter()
                    .any(|e| matches!(e, UiEventMessage::ToolCallCompleted { .. }))
        );

        let placed = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"late1","title":"Read","kind":"read","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        assert!(placed.iter().any(|e| matches!(
            e,
            UiEventMessage::ToolCallPlaced { tool_call_id, .. } if tool_call_id == "late1"
        )));
        assert!(placed.iter().any(|e| matches!(
            e,
            UiEventMessage::ToolCallCompleted { tool_call_id, is_error: false, .. } if tool_call_id == "late1"
        )));
    }

    #[test]
    fn orphan_failed_tool_update_then_tool_call_marks_is_error() {
        // Orphan update with status:failed arrives before tool_call. The replay
        // path must carry is_error:true — not the hardcoded false from before.
        let mut s = GrokTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"late_fail","status":"failed","content":[{"content":{"type":"text","text":"boom"}}]}}}"#,
            ),
        )
        .unwrap();

        let placed = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"late_fail","title":"bash","kind":"execute","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        assert!(placed.iter().any(|e| matches!(
            e,
            UiEventMessage::ToolCallCompleted { tool_call_id, is_error: true, .. } if tool_call_id == "late_fail"
        )));
    }

    #[test]
    fn failed_tool_update_sets_is_error() {
        let mut s = GrokTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc_fail","title":"bash","kind":"execute","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"tc_fail","status":"failed","content":[{"content":{"type":"text","text":"boom"}}]}}}"#,
            ),
        )
        .unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::ToolCallCompleted { tool_call_id, is_error: true, .. } if tool_call_id == "tc_fail"
        )));
    }

    #[test]
    fn internal_user_message_text() {
        let mut s = GrokTranslatorState::new("thr_x".into());
        let out = translate(
            &mut s,
            &val(r#"{"kind":"user_message","messageId":"u1","text":"你好","threadId":"thr_x"}"#),
        )
        .unwrap();

        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::MessageStarted {
                message_id,
                role: MessageRole::User,
                ..
            } if message_id == "u1"
        )));
        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::TextDelta { message_id, text } if message_id == "u1" && text == "你好")));
    }

    #[test]
    fn codex_json_rpc_event_is_not_accepted_for_grok() {
        let mut s = GrokTranslatorState::new("thr_x".into());
        let err = translate(
            &mut s,
            &val(
                r#"{"method":"item/agentMessage/delta","params":{"itemId":"a1","delta":"Hello"}}"#,
            ),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            TranslationError::Malformed { reason } if reason == "missing kind field"
        ));
    }

    #[test]
    fn acp_prompt_response_completes_open_assistant_message() {
        let mut s = GrokTranslatorState::new("thr_x".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello"}}}}"#,
            ),
        )
        .unwrap();

        let out = translate(
            &mut s,
            &val(r#"{"kind":"acp_prompt_response","stopReason":"end_turn"}"#),
        )
        .unwrap();

        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::MessageCompleted { .. })));
    }

    #[test]
    fn subagent_progress_emits_spawned_and_status() {
        let mut s = GrokTranslatorState::new("parent-thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"subagent_progress","child_session_id":"child-1","description":"Explore lifecycle"}}}"#,
            ),
        )
        .unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::SubagentSpawned {
                parent_thread_id,
                sub_thread_id,
                agent: AgentName::Grok,
                title: Some(t),
                ..
            } if parent_thread_id == "parent-thr"
                && sub_thread_id == "child-1"
                && t == "Explore lifecycle"
        )));
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::SubagentStatusUpdated {
                sub_thread_id,
                status: SubagentStatus::Running,
            } if sub_thread_id == "child-1"
        )));
        let out2 = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"subagent_progress","child_session_id":"child-1"}}}"#,
            ),
        )
        .unwrap();
        assert!(!out2
            .iter()
            .any(|e| matches!(e, UiEventMessage::SubagentSpawned { .. })));
    }

    #[test]
    fn subagent_finished_emits_completed_status() {
        let mut s = GrokTranslatorState::new("parent-thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"subagent_finished","child_session_id":"child-2","output":"done"}}}"#,
            ),
        )
        .unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::SubagentSpawned {
                sub_thread_id,
                agent: AgentName::Grok,
                ..
            } if sub_thread_id == "child-2"
        )));
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::SubagentStatusUpdated {
                sub_thread_id,
                status: SubagentStatus::Completed,
            } if sub_thread_id == "child-2"
        )));
    }

    #[test]
    fn acp_notification_thought_emits_reasoning_delta() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"thought","content":{"type":"text","text":"thinking..."}}}}"#,
            ),
        )
        .unwrap();
        assert!(out.iter().any(
            |e| matches!(e, UiEventMessage::ReasoningDelta { text, .. } if text == "thinking...")
        ));
    }

    #[test]
    fn acp_notification_agent_thought_chunk_emits_reasoning_delta() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"thinking from gemini"}}}}"#,
            ),
        )
        .unwrap();

        let assistant_id = assistant_id(&out);
        assert!(out.iter().any(
            |event| matches!(event, UiEventMessage::ReasoningDelta { message_id, text } if message_id == &assistant_id && text == "thinking from gemini")
        ));
    }

    #[test]
    fn acp_notification_tool_call_placed() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc_1","title":"Edit file","kind":"edit","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::ToolCallPlaced { tool_call_id, name, .. } if tool_call_id == "tc_1" && name.contains("Edit file")))
        );
    }

    #[test]
    fn acp_notification_tool_call_without_open_message_starts_assistant_message() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc_1","title":"Read file","kind":"read","status":"pending"}}}"#,
            ),
        )
        .unwrap();

        let assistant_id = assistant_id(&out);
        assert!(out.iter().any(
            |event| matches!(event, UiEventMessage::ToolCallPlaced { message_id, tool_call_id, .. } if message_id == &assistant_id && tool_call_id == "tc_1")
        ));
    }

    #[test]
    fn cancelled_prompt_then_continue_tool_call_uses_new_assistant_message() {
        let mut s = GrokTranslatorState::new("thr".into());
        let first = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"partial"}}}}"#,
            ),
        )
        .unwrap();
        let first_assistant_id = assistant_id(&first);

        let cancelled = translate(
            &mut s,
            &val(r#"{"kind":"acp_prompt_response","stopReason":"cancelled"}"#),
        )
        .unwrap();
        assert!(cancelled.iter().any(
            |event| matches!(event, UiEventMessage::MessageCompleted { message_id, .. } if message_id == &first_assistant_id)
        ));

        let _ = translate(
            &mut s,
            &val(r#"{"kind":"user_message","messageId":"u_continue","text":"continue"}"#),
        )
        .unwrap();
        let after_continue_tool = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc_after_continue","title":"Read file","kind":"read","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        let new_assistant_id = assistant_id(&after_continue_tool);

        assert_ne!(new_assistant_id, first_assistant_id);
        assert!(after_continue_tool.iter().any(
            |event| matches!(event, UiEventMessage::ToolCallPlaced { message_id, tool_call_id, .. } if message_id == &new_assistant_id && tool_call_id == "tc_after_continue")
        ));
    }

    #[test]
    fn acp_notification_tool_call_update_completed() {
        let mut s = GrokTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc_1","title":"Edit","kind":"edit","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"completed","content":null}}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::ToolCallCompleted { tool_call_id, is_error: false, .. } if tool_call_id == "tc_1"))
        );
    }

    #[test]
    fn acp_closed_emits_thread_closed() {
        let mut s = GrokTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}}}"#,
            ),
        )
        .unwrap();
        let out = translate(&mut s, &val(r#"{"kind":"acp_closed"}"#)).unwrap();
        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::MessageCompleted { .. })));
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::ThreadClosed {
                reason: ThreadEndReason::AgentDone,
                ..
            }
        )));
    }

    #[test]
    fn acp_notification_plan_emits_raw() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"plan","entries":[{"content":"Step 1","priority":"high","status":"pending"}]}}}"#,
            ),
        )
        .unwrap();
        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::Raw { kind, .. } if kind == "grok/plan")));
    }

    #[test]
    fn acp_server_request_permission_emits_tool_call_placed() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_server_request","method":"session/request_permission","params":{"toolCall":{"toolCallId":"tc_perm","title":"Run shell","kind":"terminal","status":"pending"},"options":[]}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::ToolCallPlaced { tool_call_id, name, .. } if tool_call_id == "tc_perm" && name.contains("Run shell")))
        );
    }

    #[test]
    fn unknown_session_update_falls_through_to_raw() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"custom_event","data":"something"}}}"#,
            ),
        )
        .unwrap();
        assert!(out.iter().any(
            |e| matches!(e, UiEventMessage::Raw { kind, .. } if kind == "grok/acp/custom_event")
        ));
    }

    #[test]
    fn approval_request_envelope_becomes_raw_for_overlay() {
        let mut s = GrokTranslatorState::new("thr_x".into());
        let raw = val(
            r#"{"method":"approval/request","params":{"request_id":"perm-1","thread_id":"thr_x","turn_id":"","method":"session/request_permission","params":{"toolCall":{"title":"ls","kind":"other"}}}}"#,
        );
        let out = translate(&mut s, &raw).unwrap();
        assert!(matches!(
            &out[0],
            UiEventMessage::Raw { kind, payload_json }
                if kind == "approval/request" && payload_json.contains("perm-1")
        ));
    }

    fn tool_completed_output(events: &[UiEventMessage]) -> String {
        events
            .iter()
            .find_map(|e| match e {
                UiEventMessage::ToolCallCompleted { output, .. } => Some(output.render_preview()),
                _ => None,
            })
            .expect("ToolCallCompleted")
    }

    #[test]
    fn search_replace_diff_content_projects_to_unified_patch() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(r#"{
                  "kind":"acp_notification",
                  "params":{
                    "update":{
                      "sessionUpdate":"tool_call",
                      "toolCallId":"tc_edit",
                      "title":"search_replace",
                      "kind":"edit",
                      "status":"completed",
                      "content":[{
                        "type":"diff",
                        "path":"/repo/docs/architecture-desktop.md",
                        "oldText":"line one\nline two\n",
                        "newText":"line one\nline two plus\n",
                        "_meta":{
                          "details":[{
                            "old_string":"line two\n",
                            "new_string":"line two plus\n",
                            "old_line":2,
                            "new_line":2,
                            "context_before":"line one\n",
                            "context_after":"",
                            "line_prefix":""
                          }]
                        }
                      }]
                    }
                  }
                }"#),
        )
        .unwrap();
        let text = tool_completed_output(&out);
        assert!(
            text.contains("--- a/") && text.contains("+++ b/"),
            "expected unified headers, got: {text}"
        );
        assert!(text.contains("@@ "), "expected hunk header, got: {text}");
        assert!(
            text.contains("-line two"),
            "expected delete line, got: {text}"
        );
        assert!(
            text.contains("+line two plus"),
            "expected add line, got: {text}"
        );
        assert!(
            !text.contains("EditsApplied"),
            "must not dump structured raw JSON: {text}"
        );
    }

    #[test]
    fn search_replace_raw_output_only_projects_to_unified_patch() {
        // Content empty → fall back to raw_output ToolOutput::SearchReplace::EditsApplied
        // without dumping the whole JSON object (the user-reported bug).
        let mut s = GrokTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(r#"{
                  "kind":"acp_notification",
                  "params":{
                    "update":{
                      "sessionUpdate":"tool_call",
                      "toolCallId":"tc_raw2",
                      "title":"search_replace",
                      "kind":"edit",
                      "status":"pending"
                    }
                  }
                }"#),
        )
        .unwrap();
        let out = translate(
            &mut s,
            &val(
                r#"{
                  "kind":"acp_notification",
                  "params":{
                    "update":{
                      "sessionUpdate":"tool_call_update",
                      "toolCallId":"tc_raw2",
                      "status":"completed",
                      "content":[],
                      "rawOutput":{
                        "type":"SearchReplace",
                        "EditsApplied":{
                          "absolute_path":"/repo/docs/architecture-desktop.md",
                          "old_string":"alpha\n",
                          "new_string":"beta\n",
                          "tool_output_for_prompt":"The file architecture-desktop.md has been updated successfully.",
                          "tool_output_for_prompt_concise":"The file architecture-desktop.md has been updated.",
                          "edits":{
                            "details":[{
                              "old_string":"alpha\n",
                              "new_string":"beta\n",
                              "old_line":188,
                              "new_line":188,
                              "context_before":"before\n",
                              "context_after":"after\n",
                              "line_prefix":""
                            }]
                          }
                        }
                      }
                    }
                  }
                }"#,
            ),
        )
        .unwrap();
        let text = tool_completed_output(&out);
        assert!(
            text.contains("--- a/repo/docs/architecture-desktop.md")
                || text.contains("architecture-desktop.md")
        );
        assert!(text.contains("-alpha"), "got: {text}");
        assert!(text.contains("+beta"), "got: {text}");
        assert!(
            text.contains(" before") || text.contains("before"),
            "got: {text}"
        );
        assert!(
            !text.contains("EditsApplied"),
            "must not dump raw JSON: {text}"
        );
        assert!(
            !text.contains("tool_output_for_prompt"),
            "must not dump prompt fields as body: {text}"
        );
    }

    #[test]
    fn search_replace_new_file_diff_projects_additions() {
        let mut s = GrokTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(r#"{
                  "kind":"acp_notification",
                  "params":{
                    "update":{
                      "sessionUpdate":"tool_call",
                      "toolCallId":"tc_new",
                      "title":"write",
                      "kind":"edit",
                      "status":"completed",
                      "content":[{
                        "type":"diff",
                        "path":"/repo/src/new.rs",
                        "oldText":null,
                        "newText":"fn main() {}\n"
                      }]
                    }
                  }
                }"#),
        )
        .unwrap();
        let text = tool_completed_output(&out);
        assert!(text.contains("+fn main() {}"), "got: {text}");
        assert!(text.contains("@@ "), "got: {text}");
    }

    #[test]
    fn apply_patch_raw_output_projects_multi_file_patches() {
        let mut s = GrokTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(r#"{
                  "kind":"acp_notification",
                  "params":{
                    "update":{
                      "sessionUpdate":"tool_call",
                      "toolCallId":"tc_patch",
                      "title":"apply_patch",
                      "kind":"edit",
                      "status":"pending"
                    }
                  }
                }"#),
        )
        .unwrap();
        let out = translate(
            &mut s,
            &val(r#"{
                  "kind":"acp_notification",
                  "params":{
                    "update":{
                      "sessionUpdate":"tool_call_update",
                      "toolCallId":"tc_patch",
                      "status":"completed",
                      "content":[],
                      "rawOutput":{
                        "type":"ApplyPatch",
                        "Success":{
                          "files":[
                            {
                              "path":"/repo/a.rs",
                              "action":"modified",
                              "old_text":"a\n",
                              "new_text":"A\n"
                            },
                            {
                              "path":"/repo/b.rs",
                              "action":"added",
                              "new_text":"b\n"
                            }
                          ],
                          "tool_output_for_prompt":"ok"
                        }
                      }
                    }
                  }
                }"#),
        )
        .unwrap();
        let text = tool_completed_output(&out);
        assert!(text.contains("a.rs"), "got: {text}");
        assert!(text.contains("b.rs"), "got: {text}");
        assert!(text.contains("-a") && text.contains("+A"), "got: {text}");
        assert!(text.contains("+b"), "got: {text}");
    }

    #[test]
    fn extract_tool_result_never_dumps_edits_applied_json() {
        // Unit-level: the raw object alone must not appear as tool output text.
        let update = val(r#"{
              "content": [],
              "rawOutput": {
                "type": "SearchReplace",
                "EditsApplied": {
                  "absolute_path": "/tmp/x.md",
                  "old_string": "old\n",
                  "new_string": "new\n",
                  "tool_output_for_prompt": "updated",
                  "edits": {
                    "details": [{
                      "old_string": "old\n",
                      "new_string": "new\n",
                      "old_line": 1,
                      "new_line": 1,
                      "context_before": "",
                      "context_after": "",
                      "line_prefix": ""
                    }]
                  }
                }
              }
            }"#);
        let text = extract_tool_result_text(&update);
        assert!(!text.starts_with('{'), "got: {text}");
        assert!(!text.contains("EditsApplied"), "got: {text}");
        assert!(
            text.contains("-old") && text.contains("+new"),
            "got: {text}"
        );
    }

    #[test]
    fn unified_diff_from_strings_common_prefix_suffix() {
        let patch = unified_diff_from_strings("f.rs", "a\nb\nc\nd\n", "a\nB\nc\nd\n", 1, 1);
        assert!(patch.contains("-b") && patch.contains("+B"), "got: {patch}");
        // Context lines for a/c around the change
        assert!(
            patch.contains(" a") || patch.contains("\n a\n"),
            "got: {patch}"
        );
    }

    #[test]
    fn bash_tool_output_strips_ansi_color_codes() {
        // Grok ACP content is raw process bytes with SGR; UI must not show [31m.
        let mut s = GrokTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(r#"{
                  "kind":"acp_notification",
                  "params":{
                    "update":{
                      "sessionUpdate":"tool_call",
                      "toolCallId":"tc_bash",
                      "title":"npm test",
                      "kind":"execute",
                      "status":"pending"
                    }
                  }
                }"#),
        )
        .unwrap();
        // Real ESC bytes (not the JSON \u001b literal form alone).
        let update = serde_json::json!({
            "kind": "acp_notification",
            "params": {
                "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tc_bash",
                    "status": "completed",
                    "content": [{
                        "type": "content",
                        "content": {
                            "type": "text",
                            "text": "\u{1b}[31m✖ fail\u{1b}[39m \u{1b}[90m(12ms)\u{1b}[39m"
                        }
                    }]
                }
            }
        });
        let out = translate(&mut s, &update).unwrap();
        let text = tool_completed_output(&out);
        assert!(!text.contains('['), "ansi leftovers: {text:?}");
        assert!(text.contains("✖ fail"), "got: {text:?}");
        assert!(text.contains("(12ms)"), "got: {text:?}");
    }

    #[test]
    fn read_file_prefers_raw_output_over_arrow_content() {
        // content has sparse N→ markers for the model; pager uses raw_output.
        let update = serde_json::json!({
            "content": [{
                "type": "content",
                "content": { "type": "text", "text": "880→    }\n  )\n890→  foo\n" }
            }],
            "rawOutput": {
                "type": "ReadFile",
                "FileContent": {
                    "content": "880→    }\n  )\n890→  foo\n",
                    "absolute_path": "/repo/SessionsView.tsx",
                    "offset": 879,
                    "raw_output": "    }\n  )\n  foo\n",
                    "total_lines": 1400
                }
            }
        });
        let text = extract_tool_result_text(&update);
        // Dense numbering from offset 879 → start line 880
        assert!(text.contains("880→    }"), "got: {text:?}");
        assert!(text.contains("881→  )"), "got: {text:?}");
        assert!(text.contains("882→  foo"), "got: {text:?}");
        // No sparse decade-only markers left without intermediate numbers.
        assert!(
            !text.contains("890→"),
            "should densify, not keep decade: {text:?}"
        );
    }

    #[test]
    fn list_dir_projects_listing_from_raw_output() {
        let update = serde_json::json!({
            "content": [],
            "rawOutput": {
                "type": "ListDir",
                "Content": {
                    "content": "src/\nCargo.toml\nREADME.md\n",
                    "absolute_root_path": "/repo"
                }
            }
        });
        let text = extract_tool_result_text(&update);
        assert!(text.contains("Cargo.toml"), "got: {text:?}");
        assert!(!text.contains("absolute_root_path"), "got: {text:?}");
    }

    #[test]
    fn grep_projects_file_matches_not_stub_count() {
        let update = serde_json::json!({
            "content": [{
                "type": "content",
                "content": { "type": "text", "text": "found 2 matches" }
            }],
            "rawOutput": {
                "type": "GrepSearch",
                "stdout": [],
                "stderr": [],
                "exit_code": 0,
                "match_count": 2,
                "file_matches": [{
                    "path": "src/a.rs",
                    "matches": [
                        { "line_number": 10, "content": "fn foo() {}" },
                        { "line_number": 20, "content": "fn bar() {}" }
                    ]
                }]
            }
        });
        let text = extract_tool_result_text(&update);
        assert!(text.contains("src/a.rs:10:fn foo() {}"), "got: {text:?}");
        assert!(text.contains("src/a.rs:20:fn bar() {}"), "got: {text:?}");
        assert!(
            !text.starts_with("found 2 matches"),
            "should not use stub: {text:?}"
        );
    }

    #[test]
    fn bash_prefers_output_for_prompt_over_ansi_content() {
        let update = serde_json::json!({
            "content": [{
                "type": "content",
                "content": {
                    "type": "text",
                    "text": "\u{1b}[31mraw ansi\u{1b}[39m"
                }
            }],
            "rawOutput": {
                "type": "Bash",
                "output": [],
                "output_for_prompt": "exit: 0\nclean output\n",
                "exit_code": 0
            }
        });
        let text = extract_tool_result_text(&update);
        assert!(text.contains("clean output"), "got: {text:?}");
        assert!(!text.contains("raw ansi"), "got: {text:?}");
        assert!(!text.contains('['), "got: {text:?}");
    }

    #[test]
    fn never_dumps_typed_tool_output_json_objects() {
        for raw in [
            serde_json::json!({"type":"ListDir","Content":{"content":"a\n","absolute_root_path":"/x"}}),
            serde_json::json!({"type":"SearchReplace","EditsApplied":{
                "absolute_path":"/x","old_string":"a\n","new_string":"b\n",
                "tool_output_for_prompt":"ok",
                "edits":{"details":[{"old_string":"a\n","new_string":"b\n","old_line":1,"new_line":1,
                    "context_before":"","context_after":"","line_prefix":""}]}
            }}),
            serde_json::json!({"type":"ReadFile","FileContent":{
                "content":"1→hi\n","absolute_path":"/x","raw_output":"hi\n","total_lines":1
            }}),
        ] {
            let update = serde_json::json!({ "content": [], "rawOutput": raw });
            let text = extract_tool_result_text(&update);
            assert!(!text.trim_start().starts_with('{'), "dumped JSON: {text:?}");
            assert!(!text.contains("\"type\""), "dumped JSON: {text:?}");
        }
    }
}
