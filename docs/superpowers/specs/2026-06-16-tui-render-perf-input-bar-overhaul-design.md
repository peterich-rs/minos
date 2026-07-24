# TUI Render Performance + Input Bar Overhaul

## Problem

The TUI has two categories of issues blocking daily usability:

1. **Render bottleneck**: `render_chat` calls `build_lines(all_items, width)` + `visual_lines(all_lines, width)` for the currently-visible agent chat on every redraw (whenever `handle_event` returns `true`). The same pattern exists in `group_chat.rs:34`. While redraws don't happen every tick, during active streaming (200ms ingest events) the active session rebuilds its entire line set each frame. For a session with thousands of lines, this means tens of thousands of `Line` allocations per frame. **Before implementing, add a `tracing` span or timing log in `render_chat` and `render_group_chat` to confirm which path is the dominant cost and whether `group_chat.rs` needs the same treatment.**

2. **Input bar is too basic**: The `InputState` (834 lines) supports cursor movement, word/line deletion, and `@agent` mention completion — but lacks prompt history, clipboard paste/copy, file path completion, mouse cursor positioning, a proper visual cursor, and newline insertion.

## Solution

Two workstreams in one spec:

- **Render performance**: Viewport-slicing with an incremental per-item line index. Only the visible window of lines is built and wrapped per frame.
- **Input bar overhaul**: Six new capabilities layered onto the existing `InputState`.

## Workstream A: Viewport-Slicing Render

### Architecture

The render index/cache lives in the **render layer** (`ui/chat.rs` or a new `ui/render_cache.rs`), not in `translation.rs`. This is critical: visual line counts depend on `chat.rs`'s private rendering logic (markdown headings, diff styles, tool detail expansion, separator insertion, streaming cursor, Unicode-aware wrapping). A count function in `translation.rs` would be a second rendering implementation that inevitably diverges.

Instead, the index is built by running the **same** `build_item_lines` logic and measuring the output, but without retaining the full rendered transcript. The count function calls the same render code path to guarantee consistency.

#### RenderCache

```rust
// In ui/chat.rs (or ui/render_cache.rs)
pub struct RenderCache {
    /// session id this cache was built for; prevents reusing a single active-chat cache across sessions
    indexed_session_id: Option<String>,
    /// item index -> starting absolute visual line number (after wrapping + separators)
    item_starts: Vec<usize>,
    /// total visual lines across all items (including separators)
    total_lines: usize,
    /// version tag from ChatState at cache time
    indexed_version: u64,
    /// width the cache was built for
    indexed_width: u16,
}
```

`ChatState` gains a `version: u64` field that is bumped by **every** mutation that affects rendering — not just `apply_ui_event`. See "Version Tracking" below.

The `RenderCache` itself is stored alongside the render function (passed in/out of `render_chat`), or on a wrapper struct. It is **not** stored on `ChatState` — keeping it in the render layer prevents the data layer from depending on rendering concerns.

If `UiState` stores only a single cache for the currently visible chat, the cache must compare `indexed_session_id` as well as `indexed_version` and `indexed_width`. Otherwise switching from one session to another with the same version/width can reuse the wrong `item_starts`. A `HashMap<String, RenderCache>` keyed by `session_id` is also acceptable; the simpler single-cache option is fine as long as it includes session identity.

### Counting Without Retaining Lines

`count_item_visual_lines(item, width) -> usize` calls the same sub-functions as `build_item_lines` (`push_markdown_lines`, `push_code_block`, `push_tool_detail_lines`, etc.) but with a **sink** that counts wrapped visual rows instead of retaining rendered `Line` objects. This is implemented as a `LineSink` trait:

```rust
trait LineSink {
    fn push_line(&mut self, line: Line<'static>);
}

struct CountingSink {
    width: u16,
    visual_lines: usize,
}
impl LineSink for CountingSink {
    fn push_line(&mut self, line: Line<'static>) {
        self.visual_lines += visual_lines(vec![line], self.width).len();
    }
}

struct VecSink(Vec<Line<'static>>);
impl LineSink for VecSink {
    fn push_line(&mut self, line: Line<'static>) { self.0.push(line); }
}
```

The existing `push_markdown_lines`, `push_code_block`, etc. are refactored to accept `&mut impl LineSink` instead of `&mut Vec<Line>`. The sink receives the actual `Line<'static>` that would be rendered; `CountingSink` then applies the same `visual_lines` wrapping logic and counts the wrapped visual rows. Counting raw `push_line()` calls is not sufficient because a single rendered line can wrap into many visual rows at narrow widths.

The separator is handled by the cache builder: for each item at index > 0, add 1 visual line for the separator before counting the item's own lines. The same separator-before-item convention must be used by `visible_window` and by the render loop.

### Render Path

```rust
pub fn render_chat(f: &mut Frame, area: Rect, chat: &mut ChatState, focused: bool, cache: &mut RenderCache) {
    // ... border/title setup ...

    cache.rebuild_if_stale(chat.session_id.as_str(), &chat.items, chat.version, inner.width);

    let base_row = usize::from(chat.active_scroll());
    let height = usize::from(inner.height);
    let visible = cache.visible_window(base_row, height);

    // Build lines only for visible items
    let mut all_lines = Vec::with_capacity(height + visible.item_count);
    for (idx, item) in visible.items.iter().enumerate() {
        if visible.start_item_index + idx > 0 {
            all_lines.push(separator_line(inner.width));
        }
        build_item_lines_into(&mut all_lines, item);
    }

    // Wrap into visual lines
    let mut visual = visual_lines(all_lines, inner.width);

    // Skip within first item to reach scroll position
    let skip = visible.line_offset_within_first_segment;
    let mut visible_visual_lines: Vec<VisualLine> = visual.into_iter()
        .skip(skip)
        .take(height)
        .collect();

    // Selection: apply with absolute base_row offset
    apply_selection_with_offset(visible_visual_lines.as_mut_slice(), chat.selection.as_ref(), base_row);

    let visible_lines: Vec<Line> = visible_visual_lines
        .into_iter()
        .map(|vl| vl.line)
        .collect();

    f.render_widget(Paragraph::new(visible_lines).block(block), area);
}
```

`RenderCache::visible_window(base_row, height) -> VisibleWindow` returns:
- `items: &[ChatItem]` — the items that overlap the visible window
- `start_item_index: usize` — absolute item index (for separator decision)
- `line_offset_within_first_segment: usize` — how many visual lines to skip after rendering the first visible segment. This offset includes the separator-before-item line when the first visible item has one, so scroll positions that land on or after a separator remain correct.

### Selection Compatibility

**Critical fix**: `apply_selection` currently uses `enumerate()` from row 0, but the visible slice starts at `base_row`. The fix is to pass the absolute offset:

```rust
fn apply_selection_with_offset(lines: &mut [VisualLine], selection: Option<&ChatSelection>, base_row: usize) {
    let Some(selection) = selection.filter(|s| !s.is_empty()) else { return; };
    for (local_row, visual) in lines.iter_mut().enumerate() {
        let absolute_row = base_row + local_row;
        if let Some((start_col, end_col)) = selected_cols_for_row(selection, absolute_row, &visual.text) {
            visual.line = highlight_line(std::mem::take(&mut visual.line), start_col, end_col);
        }
    }
}
```

`selected_text` similarly uses `item_starts` to find the item range containing the selection, builds only those items, and computes text with absolute row accounting.

### Version Tracking

`ChatState.version` is bumped by **all** rendering-relevant mutations. Currently, items are mutated in multiple places outside `apply_ui_event`:

1. `toggle_tool_expansion` in `app.rs:1502` — directly mutates `chat.items`
2. `finish_all_streaming()` in `translation.rs` — called from manager events
3. Any future mutator

**Solution**: Encapsulate all item mutations behind `ChatState` methods that bump version:

```rust
impl ChatState {
    pub fn toggle_tool_expansion(&mut self) {
        for item in &mut self.items {
            if let ChatItem::ToolCall { is_expanded, .. } = item {
                *is_expanded = !*is_expanded;
            }
        }
        self.version += 1;
    }

    // apply_ui_event already bumps version at entry.

    // finish_all_streaming bumps version.
}
```

Remove direct `&mut chat.items` access from `app.rs`. The `items` field becomes accessible via `&self` only (read) — mutations go through methods. This is the most robust approach: it's impossible to forget to bump the version.

### Files Changed

| File | Change |
|---|---|
| `minos-tui/src/translation.rs` | Add `version: u64` field to `ChatState`, bump in `apply_ui_event` and all item-mutating methods (`toggle_tool_expansion`, `finish_all_streaming`, etc.). Make `items` read-only externally (`pub fn items(&self) -> &[ChatItem]`). Add item mutation methods. |
| `minos-tui/src/ui/chat.rs` | Add `RenderCache` struct, `LineSink` trait, refactor `push_markdown_lines`/`push_code_block`/`push_tool_detail_lines` to use `LineSink`, add `count_item_visual_lines`, `render_chat` signature gains `&mut RenderCache`, `visible_window` method, `apply_selection_with_offset`. Adapt `selected_text` to use cache. |
| `minos-tui/src/ui/mod.rs` | Store `RenderCache` per visible chat (or a single cache for the active chat). Pass to `render_chat`. |
| `minos-tui/src/app.rs` | Replace `chat.items` direct mutation with `chat.toggle_tool_expansion()` method call. Adapt to read-only `items()` accessor. |

### Benchmarking Requirement

Before implementing the full viewport-slicing, add a `tracing` span around `render_chat` and `render_group_chat` with item count logged. Run a real session with a 5000+ line thread and confirm:
1. Which render path dominates (chat.rs vs group_chat.rs)?
2. Is the cost in `build_lines` (Line allocation) or `visual_lines` (wrapping)?

If `group_chat.rs` is the dominant cost, extend the same cache approach there (it shares the same `build_lines` → `visual_lines` → skip/take pattern).

### Testing Strategy

Unit tests (no rendering, pure logic):
- `counting_sink_matches_vec_sink_line_count` (same input through both sinks = same count)
- `render_cache_item_starts_match_count_output`
- `render_cache_rebuilds_on_version_change`
- `render_cache_rebuilds_on_width_change`
- `visible_window_returns_items_covering_scroll_range`
- `visible_window_handles_scroll_at_boundary`
- `visible_window_line_offset_skips_correctly`
- `max_scroll_matches_total_lines_minus_height`
- `apply_selection_with_offset_highlights_correct_absolute_rows`
- `selected_text_works_with_cache_when_selection_spans_items`
- `toggle_tool_expansion_bumps_version`

## Workstream B: Input Bar Overhaul

### B1: Prompt History (↑/↓)

#### Interaction Model

`↑`/`↓` navigate history **only when at the first/last visual line** of the editor (after soft-wrapping), not logical `\n` lines. This is the key subtlety: the editor wraps long lines at the render width, so "first line" means visual row 0 and "last line" means the last visual row — which may be part of a soft-wrapped logical line.

| Cursor position | `↑` | `↓` |
|---|---|---|
| First visual line (visual row 0, any col) | History previous | Move down in editor |
| Last visual line | Move up in editor | History next |
| Middle visual row | Move up/down in editor | Move up/down in editor |

This mirrors shell behavior. At visual row 0 pressing `↑` recalls the previous prompt; pressing `↓` past the last entry restores the original draft.

#### Visual Row Calculation

The current `move_up`/`move_down` operate on logical `\n` lines (`current_line_start`/`current_line_end`). For history detection we need the **visual** position after soft-wrapping. Add:

```rust
fn visual_cursor_position(content: &str, cursor_pos: usize, width: u16) -> usize {
    // Same wrapping logic as wrapped_row_for_cursor — returns the visual row
}

fn visual_row_count(content: &str, width: u16) -> usize {
    // Total visual rows after soft-wrapping at width
}
```

`↑`/`↓` handler priority chain:
1. **Active picker** (agent or path) → navigate picker candidates.
2. **Visual row 0** (for `↑`) or **visual row == last** (for `↓`) → history navigation.
3. Otherwise → in-editor vertical movement (`move_up`/`move_down`, which already handles logical lines and `preferred_column`).

#### Data Model

```rust
pub struct PromptHistory {
    entries: Vec<String>,
    cursor: Option<usize>,  // None = not browsing, Some(i) = browsing entry[i]
    draft: String,          // saved current input when browsing started
}
```

Added to `InputState`:
```rust
pub struct InputState {
    // ... existing fields ...
    pub history: PromptHistory,
}
```

#### Lifecycle

- **On successful non-empty send**: push the submitted content to `history.entries`, reset `cursor = None`.
- **Empty submissions**: do not enter history. Current `submit_room_input` / `submit_agent_input` call `take_input()` to clear whitespace-only input, so history recording must not live in a bare unconditional `take_input()`.
- **On ↑ at row 0**: if `cursor` is None, save `draft = content.clone()`, set `cursor = entries.len() - 1`. Else decrement `cursor`. Load `entries[cursor]` into content, place cursor at end.
- **On ↓ at last row**: if `cursor` is None, no-op. Else increment `cursor`. If `cursor >= entries.len()`, restore `draft`, set `cursor = None`. Else load `entries[cursor]`.
- **On Esc while browsing**: restore `draft`, set `cursor = None`.
- **History is per-input-bar** (room input and agent input have separate histories).
- **In-memory only** — not persisted to SQLite.

#### Files Changed

| File | Change |
|---|---|
| `minos-tui/src/ui/input_bar.rs` | Add `PromptHistory` struct, `history` field on `InputState`, modify `move_up`/`move_down` to return history-navigation signal, add `history_prev`/`history_next` methods. |
| `minos-tui/src/app.rs` | In `handle_key` for `↑`/`↓` on focused input: check row position, call `history_prev`/`history_next` vs `move_up`/`move_down`. |

### B2: Clipboard Paste/Copy (Extend Existing Infrastructure)

#### Existing Infrastructure (Do Not Duplicate)

The codebase already has:
- **Bracketed paste**: `EnableBracketedPaste` in `main.rs:169`, `AppEvent::Paste` in `app.rs:147`, `handle_paste` in `app.rs:588` which calls `InputState::insert_str` (`input_bar.rs:87`).
- **Copy**: `copy_to_clipboard` in `app.rs:2885` (production: `pbcopy`/`xclip`/`wl-copy`/`powershell`) and `app.rs:2876` (test: `TEST_CLIPBOARD` mutex). Called on mouse-release selection in `app.rs:1223`.
- **Paste normalization**: `normalize_pasted_text` in `app.rs:2584`.

This workstream **extends** these, not replaces them. No `arboard` dependency. No `App.clipboard` field.

#### What's Missing

1. **Ctrl+V paste from system clipboard**: Bracketed paste only works when the terminal supports it and the user's terminal has it enabled. `Ctrl+V` should explicitly read the system clipboard and paste, as a fallback / convenience.

2. **Ctrl+C explicit copy**: Currently copy only happens on mouse-release-drag selection. `Ctrl+C` when chat has a selection should explicitly copy (without requiring a mouse interaction).

#### Key Bindings (Contextual Ctrl+C)

| Context | Ctrl+C | Ctrl+V |
|---|---|---|
| Agent chat focused + has selection | Copy selection to clipboard (via existing `copy_to_clipboard`) | N/A |
| Input focused + no selection + agent running | Interrupt agent | Paste from clipboard |
| Input focused + no selection + idle | Quit TUI | Paste from clipboard |

**Scope note**: Input-bar text selection is deferred. Ctrl+C copy targets the **agent chat** selection (`ChatSelection`), which already exists. `RoomChat` currently has no `ChatSelection` model, so group chat copying is out of scope for this spec.

#### Implementation: Add `paste_from_clipboard`

Add a `paste_from_clipboard` function alongside `copy_to_clipboard`, using the same platform-specific command approach:

```rust
#[cfg(not(test))]
fn paste_from_clipboard() -> anyhow::Result<String> {
    #[cfg(target_os = "macos")]
    const COMMANDS: &[(&str, &[&str])] = &[("pbpaste", &[])];
    #[cfg(target_os = "linux")]
    const COMMANDS: &[(&str, &[&str])] = &[
        ("wl-paste", &[]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
    ];
    #[cfg(target_os = "windows")]
    const COMMANDS: &[(&str, &[&str])] =
        &[("powershell", &["-NoProfile", "-Command", "Get-Clipboard"])];
    // ... same fallback logic as copy_to_clipboard ...
}

#[cfg(test)]
fn paste_from_clipboard() -> anyhow::Result<String> {
    TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .last()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("clipboard empty"))
}
```

#### Ctrl+V Handler

```rust
KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
    match paste_from_clipboard() {
        Ok(text) => self.handle_paste(text),  // reuse existing paste path
        Err(_) => false,  // clipboard unavailable, no-op
    }
}
```

#### Ctrl+C Handler (Modified)

The existing `Ctrl+C` handler (`app.rs:550`) checks `current_thread_is_interruptible()` first. Change to check chat selection first:

```rust
KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
    // 1. If agent chat focused and has selection → copy.
    // RoomChat currently has no ChatSelection model.
    if matches!(self.ui.focus, Focus::AgentChat) {
        if let Some(chat) = self.ui.current_chat() {
            if chat.selection.is_some() {
                if let Some(text) = chat::selected_text(chat, width) {
                    let _ = copy_to_clipboard(&text);
                    self.ui.flash_copied();  // 2-second indicator
                }
                return true;
            }
        }
    }
    // 2. Existing interrupt/quit logic
    if self.current_thread_is_interruptible() { ... } else { self.quit(); }
}
```

#### Files Changed

| File | Change |
|---|---|
| `minos-tui/src/app.rs` | Add `paste_from_clipboard` (production + test stub), modify `Ctrl+C` handler to check selection first, add `Ctrl+V` handler, add `flash_copied` state |
| `minos-tui/src/ui/status_bar.rs` | Render "copied" indicator when `flash_copied` is active |
| `minos-tui/src/ui/mod.rs` | Add `flash_copied: Option<Instant>` to `UiState` |

### B3: File Path Completion (Tab)

#### Trigger

Tab key when the cursor is inside a token that looks like a path. A token qualifies as a path if it meets **any** of:
- Contains `/` (e.g., `src/ui/chat`, `./foo`, `../bar`)
- Starts with `~/`

A bare word without `/` (e.g., "bug" in "fix the bug") does **not** trigger path completion — this avoids hijacking Tab in normal prose. The user must type `./` or a `/` to opt into path mode.

Existing `@agent` mention completion takes priority — if cursor is in an `@` token, Tab is a no-op (mention uses Enter to accept).

#### Picker Unification

Replace the current `agent_picker: Option<InputAgentPickerState>` with a unified picker enum:

```rust
pub enum InputPicker {
    None,
    Agent(InputAgentPickerState),
    Path(InputPathPickerState),
}

pub struct InputPathPickerState {
    pub candidates: Vec<PathCandidate>,
    pub selected: usize,
    pub replace_range: Range<usize>,
}

pub struct PathCandidate {
    pub name: String,      // entry name for replacement
    pub is_dir: bool,
    pub full_path: String, // full relative path (for display)
}
```

`InputState.agent_picker` becomes `InputState.picker: InputPicker`. All existing `has_agent_picker`/`sync_agent_picker`/`accept_agent_completion` methods check `matches!(self.picker, InputPicker::Agent(_))`.

#### Completion Source

`App` holds `workspace_root: PathBuf` (already exists as CLI arg). On Tab:

1. Extract path token via `active_path_range(content, cursor_pos)` — reverse-scan from cursor to last whitespace.
2. Split into `(dir_prefix, partial_name)` at the last `/`.
3. `std::fs::read_dir(workspace_root.join(dir_prefix))`, filter entries where name starts with `partial_name`, sort by name, take first 8.
4. Populate `InputPicker::Path`.

Synchronous `read_dir` — fast enough for single directories (<1ms typical). If directory has >1000 entries, limit to first 8 sorted matches.

#### Tab Priority Chain

Tab currently calls `cycle_focus()` unconditionally (`app.rs:748`). The new logic must preserve this when no completion context is active. Priority:

1. **Active path picker** → Tab accepts the selected path candidate (same as Enter).
2. **Active agent picker** → Tab is a no-op. Agent mentions continue to use Enter to accept.
3. **`@` token at cursor** → Tab is a no-op (mention uses Enter to accept; Tab does nothing to avoid surprising focus-switch mid-mention).
4. **Path token at cursor (contains `/` or starts with `~/`)** → Tab opens path picker.
5. **Otherwise** → `cycle_focus()` (existing behavior preserved).

This ensures Tab never unexpectedly switches focus when the user is mid-completion.

#### Path Resolution Rules

`workspace_root.join(dir_prefix)` does not expand `~` or handle absolute paths. Define explicit rules:

| Input token | Resolution |
|---|---|
| `src/foo`, `./foo` | `workspace_root.join(token)` |
| `~/foo` | `dirs::home_dir().join(remainder)` (add `dirs` crate) |
| `/abs/path` | Allowed, resolves as-is (no workspace restriction — user may reference files outside workspace) |
| `../foo` | `workspace_root.join(token)` — `..` is resolved by `std::fs` naturally. Can escape workspace upward; this is intentional (agents can access parent dirs). |

Directory listing: `std::fs::read_dir(resolved_dir)`. If the resolved path doesn't exist or isn't readable, picker stays closed (no error — just no completion). Limit results to 8, sorted alphabetically by name. If directory has >1000 entries, still read all but cap to first 8 matches (most directories are small; the cost is one `read_dir` pass).

#### Accept Behavior

- **Tab / Enter** on a path picker: replace the token with `dir_prefix + selected.name`. If `is_dir`, append `/` and re-trigger completion (allows drilling into subdirectories with repeated Tab). If file, close picker.
- **Enter** on an agent picker: accept the agent mention.
- **Tab** on an agent picker: no-op by design.
- **↑/↓**: navigate candidates.
- **Esc**: close picker.

#### Rendering

Reuses the same inline picker area as agent picker. `render_inline_path_picker` renders candidates with directory indicator (`/` suffix) and highlights the selected one.

#### Files Changed

| File | Change |
|---|---|
| `minos-tui/src/ui/input_bar.rs` | Add `InputPicker` enum, `InputPathPickerState`, `PathCandidate`, `active_path_range`, `sync_path_picker`, `accept_path_completion`, `select_previous_path`/`select_next_path`. Refactor `agent_picker` → `picker`. |
| `minos-tui/src/app.rs` | Modify Tab handler: path picker → accept, agent picker/`@` token → no-op, path token → open picker, else → `cycle_focus()`. Modify Enter handler to check picker acceptance before sending. Add `dirs` crate for `~` expansion. |
| `minos-tui/Cargo.toml` | Add `dirs` crate |

### B4: Mouse Click to Position Cursor

#### Layout Tracking

`render_input_bar` currently computes `picker_rows`, `input_area`, `start_row` (scroll offset within the editor), and `visible_rows` as local variables. These are needed by the mouse click handler to map screen coordinates to a byte offset. `render_input_bar` must write these into a `InputLayoutMetrics` struct:

```rust
pub struct InputLayoutMetrics {
    /// The outer border Rect of the input bar
    pub outer: Rect,
    /// The inner editor area (after border + picker)
    pub editor_area: Rect,
    /// Wrapping width used for the editor content
    pub width: u16,
    /// First visible editor row (scroll offset)
    pub start_row: usize,
    /// Number of visible editor rows
    pub visible_rows: usize,
}

impl Default for InputLayoutMetrics {
    fn default() -> Self {
        Self { outer: Rect::default(), editor_area: Rect::default(), width: 1, start_row: 0, visible_rows: 1 }
    }
}
```

`UiState` gains `input_metrics: [InputLayoutMetrics; 2]` (room input, agent input), populated during `render_input_bar` via a `&mut InputLayoutMetrics` out-parameter.

```rust
pub struct UiState {
    // ... existing fields ...
    pub input_metrics: [InputLayoutMetrics; 2],
}
```

#### Click Handling

In `app.rs`, `AppEvent::Mouse(MouseEvent { kind: MouseEventKind::Down(Left), column, row, .. })`:

1. Check which input bar's `outer` Rect contains `(column, row)` via `input_metrics[0]` or `input_metrics[1]`.
2. If the click is in the picker area (above `editor_area`), treat it as a no-op or picker candidate click (deferred — for now, just focus the input).
3. If the click is within `editor_area`:
   - Focus the corresponding input.
   - Compute visual row within editor: `visual_row = (row - editor_area.y) as usize + start_row`.
   - Compute visual column: `visual_col = (column - editor_area.x) as usize`.
   - Call `byte_offset_for_visual_position(content, visual_row, visual_col, width)`.
   - Set `input.cursor_pos = offset; input.preferred_column = None;`.
4. If no input bar contains the click: existing focus-switching logic.

#### Offset Calculation

New function `byte_offset_for_visual_position(content: &str, row: usize, col: usize, width: u16) -> usize`:
- Walk `content` char by char, tracking visual row and column (same wrapping logic as `wrapped_row_for_cursor`).
- When reaching the target row and column, return the byte offset.
- If `(row, col)` is past the end of a line, clamp to line end.
- If past the end of content, clamp to `content.len()`.

#### Edge Cases

- Click beyond text end → cursor goes to end of that logical line.
- Click in the picker area → focus the input but don't move cursor (picker interaction deferred).
- `start_row` (editor scroll) is accounted for in the visual row calculation.

#### Files Changed

| File | Change |
|---|---|
| `minos-tui/src/ui/input_bar.rs` | Add `InputLayoutMetrics` struct, write metrics during `render_input_bar` via out-param. Add `byte_offset_for_visual_position`. |
| `minos-tui/src/ui/mod.rs` | Add `input_metrics: [InputLayoutMetrics; 2]` to `UiState`, pass to `render_input_bar`. |
| `minos-tui/src/app.rs` | Add mouse-click-in-input handling using `InputLayoutMetrics`. |

### B5: Visual Block/Bar Cursor

#### Problem with Current Cursor

`insert_cursor_marker` splices `CURSOR_GLYPH` (`▎`) into the content string, which:
- Changes layout (extra character width).
- Is subtle (thin bar, easy to miss).
- Breaks if cursor is at a position where the glyph wraps differently.

#### Solution: Style-Based Cursor

Remove the string-splicing approach. Instead, build the editor line as styled spans and apply a cursor style to the character at the cursor position.

#### Two Cursor Modes

```rust
pub enum CursorStyle {
    Bar,    // vertical bar at cursor position (left edge of char)
    Block,  // full cell reversed
}
```

Stored on `InputState` as `cursor_style: CursorStyle` (default: `Bar`). Toggle with `Ctrl+Alt+B`.

#### Rendering

`build_editor_lines` changes from string-splicing to span-building:

1. Build the display string **without** the glyph.
2. Compute cursor's visual `(row, col)` via `wrapped_row_for_cursor`.
3. After wrapping into `Vec<Line>`, locate the cursor's target line and apply:
   - **Bar**: insert a 1-cell-wide `Span::styled("│", REVERSED)` before the cursor character on that line.
   - **Block**: the character at cursor position gets `Style::new().add_modifier(Modifier::REVERSED)`.
4. **Empty content + focused**: render a single reversed-space cell (block cursor sitting on an empty line).

#### Width Accounting

The bar cursor occupies 1 cell, so the wrapping width for cursor lines is `width - 1` to avoid the bar pushing content. For block cursor, no width change (it overlays the existing character).

#### Files Changed

| File | change |
|---|---|
| `minos-tui/src/ui/input_bar.rs` | Add `CursorStyle` enum, `cursor_style` field, rewrite `build_editor_lines` to use span-based cursor, remove `insert_cursor_marker`, add toggle method. Add `Ctrl+Alt+B` handler. |
| `minos-tui/src/app.rs` | Handle `Ctrl+Alt+B` key. |

### B6: Soft-Wrap / Hard-Wrap Toggle (Newline Insertion)

#### Two Send Modes

| Mode | `Enter` | `Shift+Enter` | `Ctrl+J` |
|---|---|---|---|
| **Single-line (default)** | Send message | Insert `\n` | Insert `\n` |
| **Multi-line** | Insert `\n` | Send message | Insert `\n` |

Toggle: `Alt+Enter`. Current mode shown in input title: `"Agent Input [multi]"` vs `"Agent Input"`. In-memory, per session.

#### Crossterm Shift+Enter Caveat

Crossterm cannot reliably report `Shift+Enter` on all terminals — many send a plain `Enter` or an escape sequence. **`Ctrl+J` is the reliable newline key** in all modes and all terminals. Status bar shows `Ctrl+J = newline` hint.

#### Implementation

`InputState` gains `multiline: bool` (default `false`).

`Enter` handler in `app.rs`:
```rust
KeyCode::Enter => {
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        if self.ui.<focused_input>.multiline {
            return self.send_input();  // Shift+Enter sends in multi mode
        } else {
            self.ui.<focused_input>.insert_char('\n');  // Shift+Enter = newline in single mode
            return true;
        }
    }
    if self.ui.<focused_input>.multiline {
        self.ui.<focused_input>.insert_char('\n');  // Enter = newline in multi mode
    } else {
        return self.send_input();  // Enter = send in single mode
    }
}
```

`Ctrl+J` always inserts `\n` regardless of mode.

#### Files Changed

| File | Change |
|---|---|
| `minos-tui/src/ui/input_bar.rs` | Add `multiline: bool` field, `toggle_multiline` method. |
| `minos-tui/src/app.rs` | Modify `Enter` handler for mode-aware behavior, add `Ctrl+J` and `Alt+Enter` handlers. |
| `minos-tui/src/ui/mod.rs` | Show `[multi]` in input title when active. |
| `minos-tui/src/ui/status_bar.rs` | Add `Ctrl+J = newline` hint. |

## Scope

### What This Spec Covers

- Viewport-slicing render with incremental line index (Workstream A)
- Prompt history ↑/↓ (B1)
- Clipboard copy/paste via contextual Ctrl+C/Ctrl+V (B2)
- File path Tab completion (B3)
- Mouse click to position cursor (B4)
- Visual bar/block cursor (B5)
- Multi-line mode toggle (B6)

### What This Spec Does NOT Cover (Deferred)

- **Input-bar text selection** (Shift+arrow to select within input). Terminal-native Shift+drag already works. Can be added later; the `InputSelection` struct from B2 design is sketched but not implemented.
- **Adaptive light/dark theme** — separate future spec.
- **Approval UX improvement** — separate future spec.
- **Tool call spinner** — separate future spec.
- **Smooth scroll animation** (tick-based interpolation) — high cost, low payoff. The viewport-slicing already eliminates jank from full-rebuild; remaining scroll steps are instant line jumps which feel responsive.
- **Prompt history persistence** (SQLite) — in-memory only for this spec.
- **Path completion caching** — fresh `read_dir` per Tab press.
- **Fuzzy matching** for path/agent completion — prefix match only.

### New Dependencies

| Crate | Purpose |
|---|---|
| `dirs` | Home directory expansion for `~/` path completion (B3) |

No `arboard` — clipboard handled via existing `pbcopy`/`xclip`/`wl-copy` command approach.

### Changed Files Summary

| File | Workstream | Nature of Change |
|---|---|---|
| `minos-tui/Cargo.toml` | B3 | Add `dirs` crate |
| `minos-tui/src/translation.rs` | A | Add `version: u64` field to `ChatState`, bump in `apply_ui_event` + all item-mutating methods. Make `items` externally read-only. Add `toggle_tool_expansion` method. |
| `minos-tui/src/ui/chat.rs` | A | Add `RenderCache`, `LineSink` trait, refactor line-building functions to use `LineSink`, `count_item_visual_lines`, viewport-aware `render_chat` (gains `&mut RenderCache` param), `apply_selection_with_offset`, adapt `selected_text`. |
| `minos-tui/src/ui/mod.rs` | A, B4, B6 | Store `RenderCache` with session identity (or per-session caches), add `input_metrics: [InputLayoutMetrics; 2]`, `[multi]` title, `flash_copied` state. |
| `minos-tui/src/ui/input_bar.rs` | B1-B6 | `PromptHistory`, `InputPicker` enum, `InputPathPickerState`, `PathCandidate`, `InputLayoutMetrics`, `byte_offset_for_visual_position`, `visual_cursor_position`/`visual_row_count`, `CursorStyle`, `multiline`, rewrite `build_editor_lines` for style cursor. Write `InputLayoutMetrics` in `render_input_bar`. |
| `minos-tui/src/ui/status_bar.rs` | B2, B6 | "copied" flash indicator, `Ctrl+J` hint |
| `minos-tui/src/app.rs` | A, B1-B6 | Add `paste_from_clipboard` (production + test stub), modify Ctrl+C to check selection first, add Ctrl+V handler, history-aware ↑/↓ (using visual row calc), Tab priority chain, mouse click via `InputLayoutMetrics`, Ctrl+Alt+B cursor toggle, Ctrl+J / Alt+Enter newline, replace direct `chat.items` mutation with `chat.toggle_tool_expansion()` |

### Unchanged

- `minos-ui-protocol` crate — no changes.
- `minos-tui/src/backend/` — no changes.
- `minos-tui/src/event.rs` — no changes.
- `minos-tui/src/ui/group_chat.rs` — **conditionally unchanged**: the benchmarking requirement in Workstream A will determine if it needs the same `RenderCache` treatment. Initially unchanged.
- `minos-tui/src/ui/thread_list.rs` — no changes.
- `minos-tui/src/ui/room_list.rs` — no changes.
- `minos-tui/src/ui/agent_picker.rs` — no changes.
- `minos-tui/src/ui/theme.rs` — no changes (cursor styles use existing `REVERSED` modifier).

## Testing Strategy

### Unit Tests (Workstream A)

- `counting_sink_matches_vec_sink_line_count` (consistency: count vs actual line count)
- `counting_sink_counts_soft_wrapped_visual_lines`
- `render_cache_item_starts_match_count_output`
- `render_cache_rebuilds_on_version_change`
- `render_cache_rebuilds_on_width_change`
- `render_cache_rebuilds_on_session_id_change`
- `visible_window_returns_items_covering_scroll_range`
- `visible_window_handles_scroll_at_boundary`
- `visible_window_line_offset_skips_correctly`
- `visible_window_line_offset_handles_separator_boundary`
- `max_scroll_matches_total_lines_minus_height`
- `apply_selection_with_offset_highlights_correct_absolute_rows`
- `selected_text_works_with_cache_when_selection_spans_items`
- `toggle_tool_expansion_bumps_version`

### Unit Tests (Workstream B)

- `prompt_history_prev_loads_last_entry`
- `prompt_history_next_past_end_restores_draft`
- `prompt_history_esc_restores_draft`
- `prompt_history_send_pushes_entry`
- `prompt_history_ignores_empty_submission`
- `visual_cursor_position_matches_wrapped_row_for_cursor`
- `visual_row_count_counts_soft_wrapped_rows`
- `paste_from_clipboard_returns_test_clipboard_content` (test stub)
- `active_path_range_extracts_path_token`
- `sync_path_picker_filters_by_prefix`
- `accept_path_completion_replaces_token_and_appends_slash_for_dir`
- `tab_priority_chain_preserves_cycle_focus_when_no_completion`
- `tab_on_agent_picker_is_noop`
- `tab_on_path_picker_accepts_candidate`
- `byte_offset_for_visual_position_clamps_to_line_end`
- `byte_offset_for_visual_position_handles_multiline`
- `cursor_style_block_reverses_char_at_cursor`
- `cursor_style_bar_inserts_bar_span`
- `multiline_toggle_flips_enter_behavior`
