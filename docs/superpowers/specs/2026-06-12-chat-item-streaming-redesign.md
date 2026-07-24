# ChatItem Streaming Redesign

## Problem

The TUI agent chat display has two issues:

1. **Placeholder flash**: When a user sends a message, an empty `[Agent]` bubble appears immediately (from `MessageStarted`), before any content arrives from the agent.
2. **Monolithic message**: All agent output — text, reasoning/thinking, tool calls — is crammed into a single `RenderedMessage` per `message_id`. Reasoning and tool calls cannot be displayed as independent items in the chat flow.

## Solution

Replace `RenderedMessage` with a flat `Vec<ChatItem>` where each variant is an independent, self-contained item that only appears when it has content.

## ChatItem Model

```rust
enum ChatItem {
    UserMessage {
        message_id: String,
        text_parts: Vec<TextPart>,
        is_streaming: bool,
    },
    AssistantText {
        message_id: String,
        text_parts: Vec<TextPart>,
        is_streaming: bool,
    },
    Reasoning {
        message_id: String,
        text: String,
        is_streaming: bool,
    },
    ToolCall {
        message_id: String,
        tool_call_id: String,
        name: String,
        args_summary: String,
        args_detail: Option<String>,
        output_summary: Option<String>,
        output_detail: Option<String>,
        is_error: bool,
        is_expanded: bool,
        is_streaming: bool,
    },
    SystemMessage {
        text: String,
    },
    Error {
        message_id: Option<String>,
        text: String,
    },
}
```

Semantics:
- No placeholder: items only appear in the list when they have content. `MessageStarted` is tracked internally but does not push a visible item.
- Streaming per-item: each item tracks its own `is_streaming` independently.
- Ordering: items appear in arrival order — reasoning, tool call, text follow the order the events arrive.
- `message_id` is retained on each item so `MessageCompleted` can mark the right items done.

## ChatState

```rust
struct ChatState {
    pub session_id: String,
    pub agent: AgentName,
    pub translation_state: AgentTranslationState,
    pub items: Vec<ChatItem>,
    pub pending_requests: Vec<PendingAgentRequest>,
    open_message_ids: HashSet<String>,  // internal tracking, not rendered
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub max_scroll: u16,
    pub selection: Option<ChatSelection>,
}
```

### Event Mapping

| UiEventMessage | Old behavior | New behavior |
|---|---|---|
| `MessageStarted` | Push empty `RenderedMessage { is_streaming: true }` | Track `message_id` in `open_message_ids`. No item pushed. |
| `TextDelta` | Append text to `RenderedMessage` | Find/create `AssistantText` or `UserMessage` for that `message_id`. First delta creates the item. |
| `ReasoningDelta` | Append to `reasoning` field | Find/create `Reasoning` item. First delta creates it. |
| `ToolCallPlaced` | Append to `tool_calls` vec | Push/create `ToolCall` item. |
| `ToolCallCompleted` | Update `tool_calls` entry | Find `ToolCall` by `tool_call_id`, update output. |
| `MessageCompleted` | Set `is_streaming = false` on message | Set `is_streaming = false` on ALL items matching that `message_id`. Remove from `open_message_ids`. |
| `Error` | Set on message or push system | Push `Error` item. |
| Thread state terminal | `finish_streaming_assistant_messages()` | `finish_all_streaming()` — marks all streaming items done. |

Helper methods adapt:
- `last_completed_assistant_text()` — searches `items` for last `AssistantText` with `is_streaming == false`.
- `finish_streaming_assistant_messages()` → `finish_all_streaming()` — iterates all items.
- `toggle_tool_expansion` — matches on `ChatItem::ToolCall`.

## Rendering

`build_lines` in `chat.rs` iterates `&[ChatItem]`. Each variant renders independently:

| ChatItem | Render |
|---|---|
| `UserMessage` | `[You]` label + markdown text. Streaming cursor only if `is_streaming`. |
| `AssistantText` | `[Agent]` label + markdown text. Streaming cursor if `is_streaming`. |
| `Reasoning` | `Thinking` label + styled text. Streaming cursor if `is_streaming`. |
| `ToolCall` | `Tool name · status [args_summary]`. Expandable detail section (same as current). |
| `SystemMessage` | `[System]` label + text. |
| `Error` | Error-styled text. |

Separators (`─`) go between ALL items. Example rendering:

```
[You]
fix the bug in parser
──────────────────────────
Thinking
Let me look at the parser code...
──────────────────────────
Tool read_file · done file=src/parser.rs
──────────────────────────
[Agent]
The bug is on line 42...
──────────────────────────
Tool write_file · done file=src/parser.rs
──────────────────────────
[Agent]
Done! Fixed the off-by-one error.
```

Selection, wrapping, and scrolling reuse the existing `visual_lines`/`apply_selection` logic adapted to `ChatItem`.

## Scope

### Changed files

| File | Change |
|---|---|
| `minos-tui/src/translation.rs` | Major: `ChatItem` enum, rewrite `ChatState.apply_ui_event`, remove `RenderedMessage`, adapt helpers and tests |
| `minos-tui/src/ui/chat.rs` | Adapt `build_lines`, render, selection from `RenderedMessage` to `ChatItem` |
| `minos-tui/src/app.rs` | Rename `messages` → `items`, adapt tool expansion toggle |

### Unchanged

- `minos-ui-protocol` crate — translators and `UiEventMessage` stay the same.
- `minos-tui/src/backend/` — no changes.
- `minos-tui/src/event.rs` — no changes.
- Other TUI UI files (input_bar, status_bar, room_list, etc.) — no changes.
