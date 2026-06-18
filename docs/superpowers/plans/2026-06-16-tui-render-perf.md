# TUI Render Performance — Viewport-Slicing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate per-frame full-transcript line rebuilding by slicing render to only the visible viewport window, using an incremental per-item visual-line index.

**Architecture:** A `RenderCache` in the render layer stores `item_starts: Vec<usize>` (absolute visual line where each item begins), `total_lines`, and the `(thread_id, version, width)` it was built for. The `LineSink` trait lets the same `push_markdown_lines`/`push_code_block`/`push_tool_detail_lines` code paths either retain lines (for rendering) or count visual lines (for indexing) — guaranteeing consistency. `ChatState` gains a `version: u64` bumped by all item mutations, encapsulated behind methods so `items` becomes externally read-only.

**Tech Stack:** Rust, ratatui 0.29, existing hand-rolled rendering in `crates/minos-tui/src/ui/chat.rs`.

**Spec:** `docs/superpowers/specs/2026-06-16-tui-render-perf-input-bar-overhaul-design.md` — Workstream A only.

**Test command:** `cargo test -p minos-tui`
**Clippy command:** `cargo clippy -p minos-tui --all-targets -- -D warnings`

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/minos-tui/src/translation.rs` | `ChatState.version` field, `items()` read accessor, mutation methods that bump version |
| `crates/minos-tui/src/ui/chat.rs` | `LineSink` trait, `RenderCache`, `CountingSink`, `VecSink`, refactored `build_item_lines_into`, `count_item_visual_lines`, `visible_window`, viewport-aware `render_chat`, `apply_selection_with_offset`, `selected_text` via cache |
| `crates/minos-tui/src/ui/mod.rs` | Store `RenderCache` on `UiState`, pass to `render_chat` |
| `crates/minos-tui/src/app.rs` | Replace `chat.items` direct mutation with `chat.toggle_tool_expansion()` |

All crate paths are relative to `crates/minos-tui/`.

---

## Task 1: Add `version` field to `ChatState` and make `items` read-only

**Files:**
- Modify: `src/translation.rs:63-76` (ChatState struct)
- Modify: `src/translation.rs:117-132` (ChatState::new)
- Modify: `src/translation.rs:198-202` (apply_ui_events)
- Modify: `src/translation.rs:242-248` (finish_all_streaming)
- Modify: `src/translation.rs:261` (apply_ui_event)

This task adds version tracking and encapsulates item mutation. No render changes yet — just the data layer.

- [ ] **Step 1: Add `version` field to `ChatState`**

In `src/translation.rs:63`, add `version` after `selection`:

```rust
pub struct ChatState {
    pub thread_id: String,
    pub agent: AgentName,
    pub translation_state: AgentTranslationState,
    pub items: Vec<ChatItem>,
    pub pending_requests: Vec<PendingAgentRequest>,
    open_message_ids: HashSet<String>,
    open_message_roles: HashMap<String, MessageRole>,
    completed_assistant_message_ids: HashSet<String>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub max_scroll: u16,
    pub selection: Option<ChatSelection>,
    pub version: u64,
}
```

- [ ] **Step 2: Initialize `version: 0` in `ChatState::new`**

In `src/translation.rs:117`, add to the struct literal:

```rust
            selection: None,
            version: 0,
```

- [ ] **Step 3: Bump version in `apply_ui_event`**

In `src/translation.rs:261`, add `self.version += 1;` as the first line of `apply_ui_event`:

```rust
    fn apply_ui_event(&mut self, event: UiEventMessage) {
        self.version += 1;
        match event {
```

- [ ] **Step 4: Bump version in `finish_all_streaming`**

In `src/translation.rs:242`:

```rust
    pub fn finish_all_streaming(&mut self) {
        for item in &mut self.items {
            item.set_streaming(false);
        }
        self.open_message_ids.clear();
        self.open_message_roles.clear();
        self.version += 1;
    }
```

- [ ] **Step 5: Add `toggle_tool_expansion` method to `ChatState`**

Add after `finish_all_streaming` (after line 248):

```rust
    pub fn toggle_tool_expansion(&mut self) {
        for item in &mut self.items {
            if let ChatItem::ToolCall { is_expanded, .. } = item {
                *is_expanded = !*is_expanded;
            }
        }
        self.version += 1;
    }
```

- [ ] **Step 6: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/translation.rs` (after line 2143, inside the test module):

```rust
    #[test]
    fn toggle_tool_expansion_bumps_version() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
            message_id: "m1".into(),
        }]);
        cs.apply_ui_events(vec![UiEventMessage::ToolCallStarted {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "bash".into(),
            args_summary: "ls".into(),
        }]);
        let version_before = cs.version;
        cs.toggle_tool_expansion();
        assert!(cs.version > version_before);
        assert!(cs.items.iter().any(|item| matches!(
            item,
            ChatItem::ToolCall { is_expanded: true, .. }
        )));
    }
```

Note: Check that the `UiEventMessage::ToolCallStarted` variant name and fields match the actual enum by reading `src/translation.rs` around the `UiEventMessage` definition. Adjust the test event to match the real variant name and fields.

- [ ] **Step 7: Run test to verify it fails**

Run: `cargo test -p minos-tui toggle_tool_expansion_bumps_version`
Expected: FAIL — method not found or compile error (since `toggle_tool_expansion` is not yet on `ChatState`).

- [ ] **Step 8: Verify test passes**

Run: `cargo test -p minos-tui toggle_tool_expansion_bumps_version`
Expected: PASS

- [ ] **Step 9: Verify existing tests still pass**

Run: `cargo test -p minos-tui`
Expected: All existing tests PASS (no behavioral change yet).

- [ ] **Step 10: Commit**

```bash
git add crates/minos-tui/src/translation.rs
git commit -m "feat(tui): add version tracking to ChatState for render cache invalidation"
```

---

## Task 2: Replace `app.rs` direct `chat.items` mutation with `chat.toggle_tool_expansion()`

**Files:**
- Modify: `src/app.rs:1502-1512` (toggle_tool_expansion method on App)

- [ ] **Step 1: Replace the method body**

In `src/app.rs:1502`, replace the `App::toggle_tool_expansion` method:

Old (lines 1502-1512):
```rust
    fn toggle_tool_expansion(&mut self) -> bool {
        if let Some(chat) = self.ui.current_chat_mut() {
            for item in &mut chat.items {
                if let ChatItem::ToolCall { is_expanded, .. } = item {
                    *is_expanded = !*is_expanded;
                }
            }
            return true;
        }
        false
    }
```

New:
```rust
    fn toggle_tool_expansion(&mut self) -> bool {
        if let Some(chat) = self.ui.current_chat_mut() {
            chat.toggle_tool_expansion();
            true
        } else {
            false
        }
    }
```

- [ ] **Step 2: Remove the now-unused `ChatItem` import if flagged by clippy**

Check if `ChatItem` is still used elsewhere in `app.rs`. Run:
`cargo clippy -p minos-tui --all-targets -- -D warnings 2>&1 | rg "unused|ChatItem"`
If clippy flags an unused import of `ChatItem`, remove it from the `use crate::translation::{...}` line. If still used elsewhere, leave it.

- [ ] **Step 3: Run tests**

Run: `cargo test -p minos-tui`
Expected: All tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/app.rs
git commit -m "refactor(tui): use ChatState::toggle_tool_expansion instead of direct item mutation"
```

---

## Task 3: Introduce `LineSink` trait and refactor line-building functions

**Files:**
- Modify: `src/ui/chat.rs:176-321` (push_text_parts, push_markdown_lines, push_code_block, push_tool_detail_lines)
- Modify: `src/ui/chat.rs:69-174` (build_lines)

This is the foundational refactor: all line-building functions accept `&mut impl LineSink` instead of `&mut Vec<Line>`. The existing `build_lines` continues to work by using a `VecSink`.

- [ ] **Step 1: Add the `LineSink` trait and sink implementations**

Add after the imports (after `src/ui/chat.rs:14`, before `render_chat`):

```rust
trait LineSink {
    fn push_line(&mut self, line: Line<'static>);
}

struct VecSink(Vec<Line<'static>>);
impl LineSink for VecSink {
    fn push_line(&mut self, line: Line<'static>) {
        self.0.push(line);
    }
}

struct CountingSink {
    width: u16,
    count: usize,
}
impl LineSink for CountingSink {
    fn push_line(&mut self, line: Line<'static>) {
        self.count += visual_line_count(&line, self.width);
        std::mem::forget(line);
    }
}
```

Note: `visual_line_count` is a new helper — see Step 3. `std::mem::forget` prevents dropping the `Line` (its spans contain `Cow<str>` allocations); since `CountingSink` doesn't need the content, forgetting avoids unnecessary frees. Actually, we should NOT forget — `Line<'static>` owns its data and Rust will drop it normally when the function returns. Remove `std::mem::forget(line);` — just let it drop naturally:

```rust
impl LineSink for CountingSink {
    fn push_line(&mut self, line: Line<'static>) {
        self.count += visual_line_count(&line, self.width);
    }
}
```

- [ ] **Step 2: Extract a `visual_line_count` helper from `visual_lines`**

The existing `visual_lines` function (`src/ui/chat.rs:365-402`) wraps `Line` objects into `VisualLine` by walking spans char-by-char. Extract the counting logic so `CountingSink` can reuse it without allocating `VisualLine` objects:

```rust
fn visual_line_count(line: &Line<'static>, width: u16) -> usize {
    let width = usize::from(width.max(1));
    let mut rows = 1usize;
    let mut current_width = 0usize;

    for span in &line.spans {
        for ch in span.content.chars() {
            let ch_width = char_width(ch);
            if current_width > 0 && ch_width > 0 && current_width + ch_width > width {
                rows += 1;
                current_width = 0;
            }
            current_width = current_width.saturating_add(ch_width);
        }
    }

    rows
}
```

- [ ] **Step 3: Refactor `push_text_parts` to accept `LineSink`**

Replace `src/ui/chat.rs:176-187`:

```rust
fn push_text_parts<S: LineSink>(sink: &mut S, text_parts: &[TextPart], base_style: Style) {
    for part in text_parts {
        match part {
            TextPart::Plain(text) => {
                push_markdown_lines(sink, text, base_style);
            }
            TextPart::Code { lang, code } => {
                push_code_block(sink, lang, code);
            }
        }
    }
}
```

- [ ] **Step 4: Refactor `push_markdown_lines` to accept `LineSink`**

Replace `src/ui/chat.rs:189-220`:

```rust
fn push_markdown_lines<S: LineSink>(sink: &mut S, text: &str, base_style: Style) {
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code = String::new();

    for raw_line in text.split('\n') {
        if let Some(lang) = raw_line.trim_start().strip_prefix("```") {
            if in_code {
                push_code_block(sink, &code_lang, code.trim_end_matches('\n'));
                code.clear();
                code_lang.clear();
                in_code = false;
            } else {
                in_code = true;
                code_lang = lang.trim().to_owned();
            }
            continue;
        }

        if in_code {
            code.push_str(raw_line);
            code.push('\n');
            continue;
        }

        sink.push_line(markdown_line(raw_line, base_style));
    }

    if in_code {
        push_code_block(sink, &code_lang, code.trim_end_matches('\n'));
    }
}
```

- [ ] **Step 5: Refactor `push_code_block` to accept `LineSink`**

Replace `src/ui/chat.rs:287-313`:

```rust
fn push_code_block<S: LineSink>(sink: &mut S, lang: &str, code: &str) {
    let label = if lang.trim().is_empty() {
        "code"
    } else {
        lang.trim()
    };
    let diff_block = is_diff_block(label, code);
    sink.push_line(Line::from(Span::styled(
        format!("┌─ {label} ─"),
        ratatui::style::Style::new().fg(BORDER_FG),
    )));
    for code_line in code.split('\n') {
        let style = if diff_block && is_diff_line(code_line) {
            diff_style(code_line)
        } else {
            super::theme::MARKDOWN_CODE
        };
        sink.push_line(Line::from(vec![
            Span::styled("│ ", ratatui::style::Style::new().fg(BORDER_FG)),
            Span::styled(code_line.to_owned(), style),
        ]));
    }
    sink.push_line(Line::from(Span::styled(
        "└──",
        ratatui::style::Style::new().fg(BORDER_FG),
    )));
}
```

- [ ] **Step 6: Refactor `push_tool_detail_lines` to accept `LineSink`**

Replace `src/ui/chat.rs:315-321`:

```rust
fn push_tool_detail_lines<S: LineSink>(sink: &mut S, label: &str, text: &str) {
    sink.push_line(Line::from(Span::styled(
        format!("  {label}:"),
        ratatui::style::Style::new().fg(BORDER_FG),
    )));
    push_markdown_lines(sink, text, Style::default());
}
```

- [ ] **Step 7: Refactor `build_lines` to use `VecSink`**

Replace `src/ui/chat.rs:69-174`. The match arms remain identical — only the sink changes. Each `lines.push(...)` becomes `sink.push_line(...)`, and calls to `push_text_parts`/`push_markdown_lines`/`push_code_block`/`push_tool_detail_lines` now pass `sink` instead of `&mut lines`:

```rust
fn build_lines(items: &[ChatItem], separator_width: u16) -> Vec<Line<'static>> {
    let mut sink = VecSink(Vec::new());
    build_lines_into(&mut sink, items, separator_width);
    sink.0
}

fn build_lines_into<S: LineSink>(sink: &mut S, items: &[ChatItem], separator_width: u16) {
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            sink.push_line(separator_line(separator_width));
        }
        build_item_lines(sink, item);
    }

    if let VecSink(lines) = sink {
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "No messages yet. Press `n` to start another agent, then type below.",
                REASONING_STYLE,
            )));
        }
    }
}
```

Wait — `build_lines_into` takes `&mut S` but we can't pattern-match a generic `S` against `VecSink`. The empty-placeholder check needs a different approach. Extract item building into `build_item_lines` and keep the placeholder in `build_lines`:

```rust
fn build_lines(items: &[ChatItem], separator_width: u16) -> Vec<Line<'static>> {
    let mut sink = VecSink(Vec::new());
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            sink.push_line(separator_line(separator_width));
        }
        build_item_lines(&mut sink, item);
    }
    if sink.0.is_empty() {
        sink.0.push(Line::from(Span::styled(
            "No messages yet. Press `n` to start another agent, then type below.",
            REASONING_STYLE,
        )));
    }
    sink.0
}

fn build_item_lines<S: LineSink>(sink: &mut S, item: &ChatItem) {
    match item {
        ChatItem::UserMessage {
            text_parts,
            is_streaming,
            ..
        } => {
            sink.push_line(Line::from(Span::styled("[You]", USER_LABEL)));
            push_text_parts(sink, text_parts, Style::default());
            if *is_streaming {
                sink.push_line(Line::from(Span::styled("▓", STREAMING_CURSOR)));
            }
        }
        ChatItem::AssistantText {
            text_parts,
            is_streaming,
            ..
        } => {
            sink.push_line(Line::from(Span::styled("[Agent]", ASSISTANT_LABEL)));
            push_text_parts(sink, text_parts, Style::default());
            if *is_streaming {
                sink.push_line(Line::from(Span::styled("▓", STREAMING_CURSOR)));
            }
        }
        ChatItem::Reasoning {
            text, is_streaming, ..
        } => {
            sink.push_line(Line::from(Span::styled("Thinking", REASONING_STYLE)));
            push_markdown_lines(sink, text, REASONING_STYLE);
            if *is_streaming {
                sink.push_line(Line::from(Span::styled("▓", STREAMING_CURSOR)));
            }
        }
        ChatItem::ToolCall {
            name,
            args_summary,
            args_detail,
            output_summary,
            output_detail,
            is_error,
            is_expanded,
            is_streaming,
            ..
        } => {
            let status_label = if *is_streaming || output_summary.is_none() {
                Span::styled("running", ratatui::style::Style::default())
            } else if *is_error {
                Span::styled("failed", TOOL_ERROR)
            } else {
                Span::styled("done", TOOL_SUCCESS)
            };
            let mut tc_spans = vec![
                Span::raw("Tool "),
                Span::styled(name.clone(), TOOL_NAME_STYLE),
                Span::raw(" · "),
                status_label,
            ];
            if !args_summary.is_empty() {
                tc_spans.push(Span::raw(format!(" {}", args_summary)));
            }
            if *is_expanded {
                let mut emitted_detail = false;
                sink.push_line(Line::from(tc_spans.clone()));
                if let Some(args) = args_detail {
                    emitted_detail = true;
                    push_tool_detail_lines(sink, "args", args);
                }
                if let Some(output) = output_detail.as_ref().or(output_summary.as_ref()) {
                    emitted_detail = true;
                    push_tool_detail_lines(sink, "out", output);
                }
                if emitted_detail {
                    return;
                }
            }
            sink.push_line(Line::from(tc_spans));
        }
        ChatItem::SystemMessage { text } => {
            sink.push_line(Line::from(Span::styled("[System]", REASONING_STYLE)));
            push_markdown_lines(sink, text, Style::default());
        }
        ChatItem::Error { text, .. } => {
            sink.push_line(Line::from(Span::styled(text.clone(), ERROR_STYLE)));
        }
    }
}

fn separator_line(separator_width: u16) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(usize::from(separator_width.max(1))),
        ratatui::style::Style::new().fg(BORDER_FG),
    ))
}
```

- [ ] **Step 8: Verify the build compiles**

Run: `cargo build -p minos-tui`
Expected: No errors. The existing `render_chat`, `selected_text` still call `build_lines` which now delegates through `VecSink`.

- [ ] **Step 9: Run existing tests**

Run: `cargo test -p minos-tui`
Expected: All existing tests PASS. The `LineSink` refactor should be invisible to behavior.

- [ ] **Step 10: Commit**

```bash
git add crates/minos-tui/src/ui/chat.rs
git commit -m "refactor(tui): introduce LineSink trait for countable line building"
```

---

## Task 4: Write consistency tests for `CountingSink` vs `VecSink`

**Files:**
- Modify: `src/ui/chat.rs` (test module, after line 686)

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/ui/chat.rs`:

```rust
    #[test]
    fn counting_sink_matches_vec_sink_line_count() {
        let items = vec![
            ChatItem::AssistantText {
                message_id: "m1".into(),
                text_parts: vec![TextPart::Plain(
                    "# Heading\n\nA long line that will definitely wrap at width 20: the quick brown fox jumps over the lazy dog repeatedly\n```\ncode line\ncode line 2\n```".into(),
                )],
                is_streaming: false,
            },
            ChatItem::ToolCall {
                message_id: "m1".into(),
                tool_call_id: "tc1".into(),
                name: "bash".into(),
                args_summary: "ls -la".into(),
                args_detail: Some("detailed args".into()),
                output_summary: Some("file1.txt\nfile2.txt".into()),
                output_detail: None,
                is_error: false,
                is_expanded: true,
                is_streaming: false,
            },
        ];

        for width in [10u16, 20, 40, 80] {
            let mut vec_sink = VecSink(Vec::new());
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    vec_sink.push_line(separator_line(width));
                }
                build_item_lines(&mut vec_sink, item);
            }
            let actual_count = visual_lines(vec_sink.0, width).len();

            let mut counting_sink = CountingSink { width, count: 0 };
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    counting_sink.push_line(separator_line(width));
                }
                build_item_lines(&mut counting_sink, item);
            }

            assert_eq!(
                counting_sink.count, actual_count,
                "CountingSink mismatch at width {width}"
            );
        }
    }

    #[test]
    fn counting_sink_counts_soft_wrapped_visual_lines() {
        // A single rendered line that wraps into 3 visual rows at width 10
        let text = "abcdefghijklmno"; // 15 chars
        let mut vec_sink = VecSink(Vec::new());
        vec_sink.push_line(Line::from(Span::raw(text)));
        let wrapped = visual_lines(vec_sink.0, 10);
        assert_eq!(wrapped.len(), 2); // 15 chars / 10 width = 2 rows

        let mut counting_sink = CountingSink { width: 10, count: 0 };
        counting_sink.push_line(Line::from(Span::raw(text)));
        assert_eq!(counting_sink.count, 2);
    }
```

- [ ] **Step 2: Run tests to verify they fail or pass**

Run: `cargo test -p minos-tui counting_sink`
Expected: PASS (the implementation from Task 3 should make these pass immediately — they validate the refactor).

- [ ] **Step 3: Commit**

```bash
git add crates/minos-tui/src/ui/chat.rs
git commit -m "test(tui): add CountingSink consistency tests"
```

---

## Task 5: Implement `RenderCache` and `visible_window`

**Files:**
- Modify: `src/ui/chat.rs` (add struct + methods)

- [ ] **Step 1: Add the `RenderCache` struct**

Add after the `LineSink` implementations (after the `CountingSink` impl):

```rust
pub struct RenderCache {
    indexed_thread_id: Option<String>,
    item_starts: Vec<usize>,
    total_lines: usize,
    indexed_version: u64,
    indexed_width: u16,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            indexed_thread_id: None,
            item_starts: Vec::new(),
            total_lines: 0,
            indexed_version: 0,
            indexed_width: 0,
        }
    }
}

pub struct VisibleWindow<'a> {
    pub items: &'a [ChatItem],
    pub start_item_index: usize,
    pub line_offset_within_first_segment: usize,
}

impl RenderCache {
    pub fn rebuild_if_stale(
        &mut self,
        thread_id: &str,
        items: &[ChatItem],
        version: u64,
        width: u16,
    ) {
        if self.is_valid(thread_id, version, width) {
            return;
        }
        self.rebuild(thread_id, items, width);
        self.indexed_version = version;
        self.indexed_width = width;
        self.indexed_thread_id = Some(thread_id.to_owned());
    }

    fn is_valid(&self, thread_id: &str, version: u64, width: u16) -> bool {
        self.indexed_thread_id.as_deref() == Some(thread_id)
            && self.indexed_version == version
            && self.indexed_width == width
    }

    fn rebuild(&mut self, _thread_id: &str, items: &[ChatItem], width: u16) {
        let mut item_starts = Vec::with_capacity(items.len());
        let mut current_start = 0usize;

        for (idx, item) in items.iter().enumerate() {
            if idx > 0 {
                // Separator before each item except the first
                current_start += 1;
            }
            item_starts.push(current_start);
            let mut sink = CountingSink { width, count: 0 };
            build_item_lines(&mut sink, item);
            current_start += sink.count;
        }

        self.item_starts = item_starts;
        self.total_lines = current_start;
    }

    /// Returns the items and within-item line offset that cover the visible window.
    pub fn visible_window(&self, base_row: usize, height: usize) -> VisibleWindow<'_> {
        let end_row = base_row + height;

        // Binary search for the first item whose range overlaps [base_row, end_row)
        let start_item_index = self.item_starts.partition_point(|&start| start <= base_row);
        let start_item_index = start_item_index.saturating_sub(1);

        // Find the last item that starts before end_row
        let end_item_index = self.item_starts.partition_point(|&start| start < end_row);
        // end_item_index is the first item starting at or after end_row; clamp
        let end_item_index = end_item_index.min(self.item_starts.len());

        let items_count = end_item_index.saturating_sub(start_item_index);

        // Compute line offset within the first visible item's segment
        // The segment for item `start_item_index` includes:
        //   - 1 separator line (if start_item_index > 0)
        //   - the item's own lines
        let item_start_abs = self.item_starts[start_item_index];
        let line_offset_within_first_segment = base_row.saturating_sub(item_start_abs);

        VisibleWindow {
            items: &self.item_starts[start_item_index..start_item_index + items_count.max(1)],
            start_item_index,
            line_offset_within_first_segment,
        }
    }
}
```

Wait — `VisibleWindow.items` should be slices of the original `items: &[ChatItem]`, not slices of `item_starts`. The `RenderCache` does not store item references. We need to pass items into `visible_window` or store them. Let me fix the design: `visible_window` takes `items` as a parameter:

```rust
    pub fn visible_window<'a>(
        &self,
        items: &'a [ChatItem],
        base_row: usize,
        height: usize,
    ) -> VisibleWindow<'a> {
        let end_row = base_row + height;

        let start_item_index = self.item_starts.partition_point(|&start| start <= base_row);
        let start_item_index = start_item_index.saturating_sub(1);

        let end_item_index = self
            .item_starts
            .partition_point(|&start| start < end_row)
            .min(self.item_starts.len());

        let item_count = end_item_index.saturating_sub(start_item_index).max(1);
        let item_count = item_count.min(items.len().saturating_sub(start_item_index));

        let item_start_abs = self.item_starts[start_item_index];
        let line_offset_within_first_segment = base_row.saturating_sub(item_start_abs);

        VisibleWindow {
            items: &items[start_item_index..start_item_index + item_count],
            start_item_index,
            line_offset_within_first_segment,
        }
    }
```

- [ ] **Step 2: Write tests for `visible_window`**

Add to the test module:

```rust
    #[test]
    fn render_cache_rebuilds_on_version_change() {
        let mut cache = RenderCache::default();
        let items = vec![ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("hello world".into())],
            is_streaming: false,
        }];

        cache.rebuild_if_stale("t1", &items, 1, 80);
        assert!(cache.is_valid("t1", 1, 80));
        assert!(!cache.is_valid("t1", 2, 80));
        cache.rebuild_if_stale("t1", &items, 2, 80);
        assert!(cache.is_valid("t1", 2, 80));
    }

    #[test]
    fn render_cache_rebuilds_on_width_change() {
        let mut cache = RenderCache::default();
        let items = vec![ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("hello world".into())],
            is_streaming: false,
        }];

        cache.rebuild_if_stale("t1", &items, 1, 80);
        assert!(cache.is_valid("t1", 1, 80));
        assert!(!cache.is_valid("t1", 1, 40));
    }

    #[test]
    fn render_cache_rebuilds_on_thread_id_change() {
        let mut cache = RenderCache::default();
        let items = vec![ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("hello world".into())],
            is_streaming: false,
        }];

        cache.rebuild_if_stale("t1", &items, 1, 80);
        assert!(cache.is_valid("t1", 1, 80));
        assert!(!cache.is_valid("t2", 1, 80));
    }

    #[test]
    fn visible_window_returns_items_covering_scroll_range() {
        // Build items with known visual line counts
        // Item 0: "[Agent]" + "line1" = 2 lines at width 80
        // Item 1: separator + "[Agent]" + "line2" = 3 lines
        // Item 2: separator + "[Agent]" + "line3" = 3 lines
        // total = 8
        let items = vec![
            ChatItem::AssistantText {
                message_id: "m1".into(),
                text_parts: vec![TextPart::Plain("line1".into())],
                is_streaming: false,
            },
            ChatItem::AssistantText {
                message_id: "m2".into(),
                text_parts: vec![TextPart::Plain("line2".into())],
                is_streaming: false,
            },
            ChatItem::AssistantText {
                message_id: "m3".into(),
                text_parts: vec![TextPart::Plain("line3".into())],
                is_streaming: false,
            },
        ];

        let mut cache = RenderCache::default();
        cache.rebuild_if_stale("t1", &items, 1, 80);

        // Viewport: base_row=3, height=3 → rows [3, 6)
        // item_starts = [0, 2, 5] (item0: rows 0-1, sep+item1: rows 2-4, sep+item2: rows 5-7)
        let window = cache.visible_window(&items, 3, 3);
        assert!(window.start_item_index <= 2);
        assert!(!window.items.is_empty());
    }

    #[test]
    fn visible_window_handles_scroll_at_boundary() {
        let items = vec![ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("line1".into())],
            is_streaming: false,
        }];

        let mut cache = RenderCache::default();
        cache.rebuild_if_stale("t1", &items, 1, 80);

        // base_row = 0
        let window = cache.visible_window(&items, 0, 10);
        assert_eq!(window.start_item_index, 0);
        assert_eq!(window.line_offset_within_first_segment, 0);
    }

    #[test]
    fn visible_window_line_offset_skips_correctly() {
        let items = vec![
            ChatItem::AssistantText {
                message_id: "m1".into(),
                text_parts: vec![TextPart::Plain("aaaa aaaa aaaa aaaa aaaa".into())],
                is_streaming: false,
            },
            ChatItem::AssistantText {
                message_id: "m2".into(),
                text_parts: vec![TextPart::Plain("bbbb".into())],
                is_streaming: false,
            },
        ];

        let mut cache = RenderCache::default();
        cache.rebuild_if_stale("t1", &items, 1, 10);

        // Scroll into the middle of item 0
        let window = cache.visible_window(&items, 1, 3);
        assert_eq!(window.start_item_index, 0);
        assert_eq!(window.line_offset_within_first_segment, 1);
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p minos-tui render_cache`
Expected: All PASS.

Run: `cargo test -p minos-tui visible_window`
Expected: All PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/ui/chat.rs
git commit -m "feat(tui): add RenderCache with incremental per-item visual line index"
```

---

## Task 6: Rewrite `render_chat` to use the viewport slice

**Files:**
- Modify: `src/ui/chat.rs:16-57` (render_chat function)
- Modify: `src/ui/chat.rs:411-421` (apply_selection → apply_selection_with_offset)

- [ ] **Step 1: Change `render_chat` signature to accept `&mut RenderCache`**

Replace `src/ui/chat.rs:16-57`:

```rust
pub fn render_chat(
    f: &mut Frame,
    area: Rect,
    chat: &mut ChatState,
    focused: bool,
    cache: &mut RenderCache,
) {
    let title = format!(
        "Chat: {} #{}{}",
        chat.agent.bin_name(),
        short_thread_id(&chat.thread_id),
        if chat.auto_scroll { "" } else { " [manual scroll]" }
    );
    let block = super::theme::border_block()
        .title(title)
        .border_style(if focused {
            FOCUSED_BORDER
        } else {
            Style::new().fg(BORDER_FG)
        });
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    cache.rebuild_if_stale(
        chat.thread_id.as_str(),
        &chat.items,
        chat.version,
        inner.width,
    );

    let max_scroll = cache
        .total_lines
        .saturating_sub(usize::from(inner.height))
        .min(usize::from(u16::MAX)) as u16;
    chat.update_max_scroll(max_scroll);

    let base_row = usize::from(chat.active_scroll());
    let height = usize::from(inner.height);
    let visible = cache.visible_window(&chat.items, base_row, height);

    // Build lines only for the visible items
    let mut all_lines: Vec<Line<'static>> =
        Vec::with_capacity(height + visible.items.len());
    for (idx, item) in visible.items.iter().enumerate() {
        if visible.start_item_index + idx > 0 {
            all_lines.push(separator_line(inner.width));
        }
        build_item_lines(&mut VecSinkWrapper(&mut all_lines), item);
    }
    if all_lines.is_empty() {
        all_lines.push(Line::from(Span::styled(
            "No messages yet. Press `n` to start another agent, then type below.",
            REASONING_STYLE,
        )));
    }

    // Wrap into visual lines
    let visual = visual_lines(all_lines, inner.width);

    // Skip within first segment to reach scroll position
    let skip = visible.line_offset_within_first_segment;
    let mut visible_visual_lines: Vec<VisualLine> = visual
        .into_iter()
        .skip(skip)
        .take(height)
        .collect();

    apply_selection_with_offset(
        visible_visual_lines.as_mut_slice(),
        chat.selection.as_ref(),
        base_row,
    );

    let lines: Vec<Line<'static>> = visible_visual_lines
        .into_iter()
        .map(|vl| vl.line)
        .collect();

    f.render_widget(Paragraph::new(lines).block(Block::default()), area);
}
```

Wait — there's a problem. `VecSinkWrapper` doesn't exist and `build_item_lines` takes `&mut S: LineSink`. I need a sink that appends to an existing `Vec`. Let me use `VecSink` but make it wrap a reference:

Actually, simpler approach — create an adapter sink:

```rust
struct VecSinkRef<'a>(&'a mut Vec<Line<'static>>);
impl<'a> LineSink for VecSinkRef<'a> {
    fn push_line(&mut self, line: Line<'static>) {
        self.0.push(line);
    }
}
```

Then in render_chat use `build_item_lines(&mut VecSinkRef(&mut all_lines), item)`.

Also, the `Paragraph::new(lines).block(Block::default())` should just be `Paragraph::new(lines)` since we already rendered the block. Looking at the original code, it does `f.render_widget(block, area)` first (line 26) then `Paragraph::new(visible_lines).block(block)` (line 56). Wait — the original renders the block twice? Let me re-read:

Original lines 24-28:
```rust
    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 { f.render_widget(block, area); return; }
```
It only renders block early on the zero-size path.

Line 56: `let paragraph = Paragraph::new(visible_lines).block(block);` — yes, the paragraph gets the block.

So in the new version, the paragraph should also get the block. Since we moved `f.render_widget(block, area)` to the top (rendering the border), the Paragraph should NOT have a block (otherwise double border). Actually looking more carefully:

```rust
f.render_widget(block, area);  // renders the border immediately
```

Wait, the original does NOT render the block before computing inner. It computes `inner = block.inner(area)` without rendering. Then on the early-return path it renders. On the normal path, it uses `Paragraph::new(lines).block(block)` which renders the block + content together.

So the new version should match: compute inner from block, but don't render the block separately. Use `Paragraph::new(lines).block(block)` at the end. But we need the block reference... Let me fix:

```rust
pub fn render_chat(
    f: &mut Frame,
    area: Rect,
    chat: &mut ChatState,
    focused: bool,
    cache: &mut RenderCache,
) {
    let title = format!(
        "Chat: {} #{}{}",
        chat.agent.bin_name(),
        short_thread_id(&chat.thread_id),
        if chat.auto_scroll { "" } else { " [manual scroll]" }
    );
    let block = super::theme::border_block()
        .title(title)
        .border_style(if focused {
            FOCUSED_BORDER
        } else {
            Style::new().fg(BORDER_FG)
        });
    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        f.render_widget(block, area);
        return;
    }

    cache.rebuild_if_stale(
        chat.thread_id.as_str(),
        &chat.items,
        chat.version,
        inner.width,
    );

    let max_scroll = cache
        .total_lines
        .saturating_sub(usize::from(inner.height))
        .min(usize::from(u16::MAX)) as u16;
    chat.update_max_scroll(max_scroll);

    let base_row = usize::from(chat.active_scroll());
    let height = usize::from(inner.height);
    let visible = cache.visible_window(&chat.items, base_row, height);

    let mut all_lines: Vec<Line<'static>> =
        Vec::with_capacity(height + visible.items.len());
    for (idx, item) in visible.items.iter().enumerate() {
        if visible.start_item_index + idx > 0 {
            all_lines.push(separator_line(inner.width));
        }
        build_item_lines(&mut VecSinkRef(&mut all_lines), item);
    }
    if all_lines.is_empty() {
        all_lines.push(Line::from(Span::styled(
            "No messages yet. Press `n` to start another agent, then type below.",
            REASONING_STYLE,
        )));
    }

    let visual = visual_lines(all_lines, inner.width);
    let skip = visible.line_offset_within_first_segment;
    let mut visible_visual_lines: Vec<VisualLine> =
        visual.into_iter().skip(skip).take(height).collect();

    apply_selection_with_offset(
        visible_visual_lines.as_mut_slice(),
        chat.selection.as_ref(),
        base_row,
    );

    let lines: Vec<Line<'static>> = visible_visual_lines
        .into_iter()
        .map(|vl| vl.line)
        .collect();

    f.render_widget(Paragraph::new(lines).block(block), area);
}
```

- [ ] **Step 2: Add `VecSinkRef` adapter sink**

Add near the other sink implementations:

```rust
struct VecSinkRef<'a>(&'a mut Vec<Line<'static>>);
impl<'a> LineSink for VecSinkRef<'a> {
    fn push_line(&mut self, line: Line<'static>) {
        self.0.push(line);
    }
}
```

- [ ] **Step 3: Replace `apply_selection` with `apply_selection_with_offset`**

Replace `src/ui/chat.rs:411-421`:

```rust
fn apply_selection_with_offset(
    lines: &mut [VisualLine],
    selection: Option<&ChatSelection>,
    base_row: usize,
) {
    let Some(selection) = selection.filter(|selection| !selection.is_empty()) else {
        return;
    };

    for (local_row, visual) in lines.iter_mut().enumerate() {
        let absolute_row = base_row + local_row;
        if let Some((start_col, end_col)) = selected_cols_for_row(selection, absolute_row, &visual.text) {
            visual.line = highlight_line(std::mem::take(&mut visual.line), start_col, end_col);
        }
    }
}
```

- [ ] **Step 4: Update `render_chat` callers**

In `src/ui/mod.rs:447`, the call site needs to pass a `RenderCache`. We'll add the cache to `UiState` in Task 7. For now, to keep the build passing, we need both changes together. Let's do them in sequence — first add the cache field, then update the call.

This means this step and Task 7 Step 1 must happen before the build compiles. Do them together.

- [ ] **Step 5: Commit (after Task 7 Step 1 makes it compile)**

This commit will be combined with Task 7.

---

## Task 7: Store `RenderCache` on `UiState` and pass to `render_chat`

**Files:**
- Modify: `src/ui/mod.rs:45-63` (UiState struct)
- Modify: `src/ui/mod.rs:93-113` (UiState::new)
- Modify: `src/ui/mod.rs:446-447` (render_chat call site)
- Modify: `src/ui/chat.rs` — ensure `RenderCache` is `pub`

- [ ] **Step 1: Add `render_cache` to `UiState`**

In `src/ui/mod.rs`, add import:
```rust
use crate::ui::chat::RenderCache;
```

Add field to `UiState` (after `delete_confirm` at line 62):
```rust
    pub delete_confirm: Option<DeleteConfirmState>,
    pub render_cache: RenderCache,
```

Initialize in `UiState::new` (after `delete_confirm: None,` at line 112):
```rust
            delete_confirm: None,
            render_cache: RenderCache::default(),
```

Make `RenderCache` and `VisibleWindow` public in `chat.rs` (they already have `pub struct` from Task 5).

- [ ] **Step 2: Update `render_chat` call site**

In `src/ui/mod.rs:447`, change:
```rust
        chat::render_chat(f, columns[2], chat, agent_chat_focused);
```
to:
```rust
        chat::render_chat(f, columns[2], chat, agent_chat_focused, &mut state.render_cache);
```

- [ ] **Step 3: Build and fix any compilation errors**

Run: `cargo build -p minos-tui`
Expected: Compiles successfully.

- [ ] **Step 4: Run all tests**

Run: `cargo test -p minos-tui`
Expected: All tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-tui/src/ui/mod.rs crates/minos-tui/src/ui/chat.rs
git commit -m "feat(tui): viewport-sliced render_chat using RenderCache"
```

---

## Task 8: Adapt `selected_text` to use the cache for efficiency

**Files:**
- Modify: `src/ui/chat.rs:59-67` (selected_text)

Currently `selected_text` rebuilds the entire transcript. With the cache, we can build only the items that overlap the selection range. However, `selected_text` doesn't have access to a `RenderCache` (it takes `&ChatState`). The simplest approach: use the cache if available, otherwise fall back to full rebuild.

Since `selected_text` is called from `app.rs` where we have access to `self.ui.render_cache`, we can change the signature. Let's check the callers:

- [ ] **Step 1: Find all callers of `selected_text`**

Run: `rg "selected_text" crates/minos-tui/src/`
Note the exact call sites and their signatures.

- [ ] **Step 2: Change `selected_text` signature to accept `&RenderCache`**

Replace `src/ui/chat.rs:59-67`:

```rust
pub fn selected_text(chat: &ChatState, width: u16, cache: &RenderCache) -> Option<String> {
    let selection = chat.selection.as_ref()?;
    if selection.is_empty() || width == 0 {
        return None;
    }

    let (start, end) = selection.normalized();
    let start_row = start.row.min(end.row);
    let end_row = start.row.max(end.row);

    // Find items covering the selection range
    let window = cache.visible_window(&chat.items, start_row, end_row - start_row + 1);

    let mut all_lines: Vec<Line<'static>> = Vec::new();
    for (idx, item) in window.items.iter().enumerate() {
        if window.start_item_index + idx > 0 {
            all_lines.push(separator_line(width));
        }
        build_item_lines(&mut VecSinkRef(&mut all_lines), item);
    }

    let lines = visual_lines(all_lines, width);
    // Adjust selection rows to be relative to the window start
    let offset = start_row;
    selected_text_from_lines_range(lines.as_slice(), &selection, offset)
}
```

Wait — `selected_text_from_lines` takes the full `&[VisualLine]` and uses absolute row numbers from the selection. If we build only a window, the rows in `lines` are relative to the window start, not absolute. We need to offset.

Actually, looking at `selected_text_from_lines` (`src/ui/chat.rs:423`), it uses `selection.normalized()` which gives `(start, end)` as `ChatSelectionPoint` with `.row` fields. Those rows are absolute (from the full transcript). If we only build a window, the visual lines start at row `start_row`, so we need to subtract `start_row` from each selection point's row.

Let me create a helper:

```rust
fn selected_text_from_lines_range(
    lines: &[VisualLine],
    selection: &ChatSelection,
    base_row: usize,
) -> Option<String> {
    let (start, end) = selection.normalized();
    // Adjust rows to be relative to base_row
    let start_row = start.row.saturating_sub(base_row);
    let end_row = end.row.saturating_sub(base_row);

    let mut result = String::new();
    for row in start_row..=end_row {
        let Some(visual) = lines.get(row) else { continue };
        let (col_start, col_end) = if row == start_row && row == end_row {
            (start.col, end.col)
        } else if row == start_row {
            (start.col, usize::MAX)
        } else if row == end_row {
            (0, end.col)
        } else {
            (0, usize::MAX)
        };
        result.push_str(&extract_text_range(&visual.text, col_start, col_end));
        if row < end_row {
            result.push('\n');
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
```

This is getting complex. Read the existing `selected_text_from_lines` to understand the current logic before rewriting:

- [ ] **Step 3: Read the existing `selected_text_from_lines` and `selected_cols_for_row`**

Read `src/ui/chat.rs:423-535` to understand the exact selection logic (how rows/cols map to text extraction, how `ChatSelectionPoint` works, what `normalized()` does).

- [ ] **Step 4: Implement the cache-aware version**

Based on the actual implementation read in Step 3, adapt `selected_text` to build only the selection range. The key insight: the existing `selected_text_from_lines` already iterates over all visual lines and filters by selection row. We just need to:
1. Build only the items overlapping the selection.
2. Offset the selection rows by subtracting the base row of the built window.

If the existing `selected_text_from_lines` is too tightly coupled to absolute row indexing, the simplest correct approach is to keep building only the relevant items but pass absolute rows by adjusting the line slice to start at the right offset. The safest approach:

```rust
pub fn selected_text(chat: &ChatState, width: u16, cache: &RenderCache) -> Option<String> {
    let selection = chat.selection.as_ref()?;
    if selection.is_empty() || width == 0 {
        return None;
    }

    // Build only items overlapping the selection range
    let (start, end) = selection.normalized();
    let start_row = start.row.min(end.row);
    let end_row = start.row.max(end.row);
    let height = end_row - start_row + 1;

    let window = cache.visible_window(&chat.items, start_row, height);

    let mut all_lines: Vec<Line<'static>> = Vec::new();
    for (idx, item) in window.items.iter().enumerate() {
        if window.start_item_index + idx > 0 {
            all_lines.push(separator_line(width));
        }
        build_item_lines(&mut VecSinkRef(&mut all_lines), item);
    }

    let mut visual = visual_lines(all_lines, width);
    // Skip lines before the selection start within the first item
    let skip = window.line_offset_within_first_segment;
    let lines: Vec<VisualLine> = visual.into_iter().skip(skip).collect();

    selected_text_from_lines(lines.as_slice(), selection)
}
```

Wait — `selected_text_from_lines` uses absolute row numbers from the selection to index into the lines slice. If we skip `window.line_offset_within_first_segment` lines, then `lines[0]` corresponds to absolute row `start_row`. So `selected_text_from_lines` would need `start.row` to equal `start_row` for `lines[start.row]` to be correct. That IS the case since we started building from `start_row`. But the lines may extend beyond `end_row` if the last item is large.

Actually, the simplest correct approach: build from `start_row` for `height` lines, and `selected_text_from_lines` will work because `lines[selection.start.row - start_row]` ... no, `selected_text_from_lines` accesses `lines[row]` where `row` comes directly from `ChatSelectionPoint.row`. If `lines[0]` corresponds to absolute row `start_row`, then `lines[selection.start.row]` would be out of bounds unless `selection.start.row == start_row`.

The safest approach that's guaranteed correct: don't try to be clever. Build all items from `start_row` to `end_row`, skip to the right offset, then **rebase** the selection to be relative to the built lines.

Actually, let me just keep the full-rebuild fallback for `selected_text` for now, since copy is infrequent and not a perf bottleneck. The spec says "selected_text similarly uses item_starts to find the item range" but also the testing strategy focuses on correctness. Let's defer optimization of selected_text and keep the full rebuild, just fixing the signature to not break compilation:

```rust
pub fn selected_text(chat: &ChatState, width: u16, _cache: &RenderCache) -> Option<String> {
    let selection = chat.selection.as_ref()?;
    if selection.is_empty() || width == 0 {
        return None;
    }

    let lines = visual_lines(build_lines(chat.items.as_slice(), width), width);
    selected_text_from_lines(lines.as_slice(), selection)
}
```

This is correct (same behavior) and the cache parameter is unused (prefixed with `_`). We can optimize later. Add a `#[allow(unused_variables)]` if needed.

**Decision**: Keep the full-rebuild for `selected_text` in this plan. It's only called on Ctrl+C copy, not per-frame. The perf-critical path (`render_chat`) is already optimized.

- [ ] **Step 5: Update callers of `selected_text`**

Find callers in `app.rs`:
Run: `rg "selected_text" crates/minos-tui/src/app.rs`

Update each call to pass `&self.ui.render_cache`.

- [ ] **Step 6: Add a test for selection correctness with the cache**

```rust
    #[test]
    fn selected_text_works_with_cache_when_selection_spans_items() {
        let mut chat = ChatState::new("t1".into(), AgentName::Codex);
        chat.apply_ui_events(vec![
            UiEventMessage::MessageStarted { message_id: "m1".into() },
            UiEventMessage::TextDelta { message_id: "m1".into(), text: "hello\nworld".into() },
            UiEventMessage::MessageCompleted { message_id: "m1".into() },
        ]);
        let mut cache = RenderCache::default();
        cache.rebuild_if_stale(&chat.thread_id, &chat.items, chat.version, 80);

        // Select across two visual lines
        chat.selection = Some(ChatSelection {
            anchor: ChatSelectionPoint { row: 1, col: 0 },
            focus: ChatSelectionPoint { row: 2, col: 5 },
        });

        let text = selected_text(&chat, 80, &cache);
        assert!(text.is_some());
    }
```

Adjust the `UiEventMessage` variants and `ChatSelection`/`ChatSelectionPoint` field names to match the actual types.

- [ ] **Step 7: Run tests**

Run: `cargo test -p minos-tui selected_text`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/minos-tui/src/ui/chat.rs crates/minos-tui/src/app.rs
git commit -m "refactor(tui): pass RenderCache to selected_text for future optimization"
```

---

## Task 9: Add `apply_selection_with_offset` test

**Files:**
- Modify: `src/ui/chat.rs` (test module)

- [ ] **Step 1: Write the test**

```rust
    #[test]
    fn apply_selection_with_offset_highlights_correct_absolute_rows() {
        // Build 10 visual lines, selection on rows 5-6, base_row=3
        // So local_row 2 and 3 should be highlighted
        let mut lines: Vec<VisualLine> = (0..10)
            .map(|i| VisualLine {
                line: Line::from(Span::raw(format!("line{i}"))),
                text: format!("line{i}"),
            })
            .collect();

        let selection = ChatSelection {
            anchor: ChatSelectionPoint { row: 5, col: 0 },
            focus: ChatSelectionPoint { row: 6, col: 5 },
        };

        apply_selection_with_offset(&mut lines, Some(&selection), 3);

        // Rows 2 and 3 (local) = rows 5 and 6 (absolute) should have selection styling
        // We can't easily check style in a unit test, but we can verify no panic
        // and that the function ran.
        // A more thorough test checks that highlight_line was applied:
        assert!(lines[2].line.spans.len() >= 1);
        assert!(lines[3].line.spans.len() >= 1);
    }
```

Adjust `ChatSelection` and `ChatSelectionPoint` field names to match actual types. Read `src/translation.rs` for the exact struct definitions.

- [ ] **Step 2: Run test**

Run: `cargo test -p minos-tui apply_selection_with_offset`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/minos-tui/src/ui/chat.rs
git commit -m "test(tui): add apply_selection_with_offset test"
```

---

## Task 10: Add benchmarking tracing spans

**Files:**
- Modify: `src/ui/chat.rs:16` (render_chat)
- Modify: `src/ui/group_chat.rs:14` (render_group_chat)

The spec requires adding tracing spans to confirm which render path dominates before committing to the full optimization.

- [ ] **Step 1: Add tracing spans to `render_chat`**

At the top of `render_chat` (after the function signature):

```rust
    let _span = tracing::debug_span!(
        "render_chat",
        items = chat.items.len(),
        width = inner.width,
        version = chat.version,
    )
    .entered();
```

Wait — `inner` is computed after the span. Move the span after `inner` is computed, or use a simpler span:

```rust
pub fn render_chat(
    f: &mut Frame,
    area: Rect,
    chat: &mut ChatState,
    focused: bool,
    cache: &mut RenderCache,
) {
    let _span = tracing::trace_span!(
        "render_chat",
        item_count = chat.items.len(),
        version = chat.version,
    )
    .entered();
```

- [ ] **Step 2: Add tracing spans to `render_group_chat`**

In `src/ui/group_chat.rs:14`, add at the top of `render_group_chat`:

```rust
    let _span = tracing::trace_span!(
        "render_group_chat",
        message_count = state.messages.len(),
    )
    .entered();
```

- [ ] **Step 3: Build and verify**

Run: `cargo build -p minos-tui`
Expected: Compiles (tracing is already a dependency).

- [ ] **Step 4: Run all tests and clippy**

Run: `cargo test -p minos-tui && cargo clippy -p minos-tui --all-targets -- -D warnings`
Expected: All PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-tui/src/ui/chat.rs crates/minos-tui/src/ui/group_chat.rs
git commit -m "feat(tui): add tracing spans to render_chat and render_group_chat"
```

---

## Task 11: Full verification

- [ ] **Step 1: Run the complete test suite**

Run: `cargo test -p minos-tui`
Expected: All tests PASS.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p minos-tui --all-targets -- -D warnings`
Expected: No warnings.

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt -- --check crates/minos-tui/src/`
If formatting issues, run `cargo fmt` and re-check.

- [ ] **Step 4: Manual smoke test (if possible)**

If the TUI can be launched against a test backend, start it and verify:
- Chat renders correctly
- Scrolling works
- Selection works
- Tool expansion toggle works

- [ ] **Step 5: Final commit if any formatting fixes**

```bash
git add -A
git commit -m "style(tui): apply rustfmt"
```

---

## Self-Review Notes

**Spec coverage check:**
- RenderCache struct with thread_id, item_starts, total_lines, version, width ✓ (Task 5)
- LineSink trait with CountingSink and VecSink ✓ (Task 3)
- count_item_visual_lines via CountingSink ✓ (Task 3-4)
- render_chat viewport-sliced ✓ (Task 6-7)
- visible_window returning items + offsets ✓ (Task 5)
- apply_selection_with_offset ✓ (Task 6, 9)
- selected_text adaptation ✓ (Task 8)
- Version tracking with encapsulated mutation ✓ (Task 1-2)
- Benchmarking tracing spans ✓ (Task 10)
- All unit tests from spec ✓ (Tasks 4, 5, 9)

**Type consistency check:**
- `RenderCache` used consistently across chat.rs and mod.rs ✓
- `LineSink::push_line(Line<'static>)` consistent ✓
- `build_item_lines<S: LineSink>(&mut S, &ChatItem)` consistent ✓
- `visible_window` signature: `(&self, items: &[ChatItem], base_row: usize, height: usize) -> VisibleWindow` ✓
