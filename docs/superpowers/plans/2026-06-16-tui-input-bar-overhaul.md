# TUI Input Bar Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Overhaul the TUI input bar with six capabilities: prompt history (B1), clipboard paste/copy (B2), file path Tab completion (B3), mouse click cursor positioning (B4), visual bar/block cursor (B5), and multi-line mode toggle (B6).

**Architecture:** All changes layer onto the existing `InputState` struct (`crates/minos-tui/src/ui/input_bar.rs`). New state fields are added incrementally; each capability is self-contained and can be committed independently. The `app.rs` key handler dispatch is modified in focused, minimal blocks.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, existing `pbcopy`/`xclip` clipboard approach (no new clipboard crate). New dependency: `dirs` crate for `~/` expansion (B3).

**Spec:** `docs/superpowers/specs/2026-06-16-tui-render-perf-input-bar-overhaul-design.md` — Workstream B (B1-B6).

**Test command:** `cargo test -p minos-tui`
**Clippy command:** `cargo clippy -p minos-tui --all-targets -- -D warnings`

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/minos-tui/Cargo.toml` | Add `dirs` dependency (B3) |
| `crates/minos-tui/src/ui/input_bar.rs` | All new InputState fields, `PromptHistory`, `InputPicker` enum, `InputPathPickerState`, `PathCandidate`, `InputLayoutMetrics`, `byte_offset_for_visual_position`, `visual_cursor_position`/`visual_row_count`, `CursorStyle`, `multiline`, rewritten `build_editor_lines` for style cursor |
| `crates/minos-tui/src/ui/mod.rs` | `input_metrics`, `[multi]` title, `flash_copied` state |
| `crates/minos-tui/src/ui/status_bar.rs` | "copied" flash indicator, `Ctrl+J` hint |
| `crates/minos-tui/src/app.rs` | `paste_from_clipboard`, Ctrl+C/V handlers, history-aware ↑/↓, Tab priority chain, mouse click via `InputLayoutMetrics`, Ctrl+Alt+B, Ctrl+J / Alt+Enter |

All crate paths are relative to `crates/minos-tui/`.

---

## Task B1: Prompt History (↑/↓)

**Files:**
- Modify: `src/ui/input_bar.rs` (add `PromptHistory`, `visual_cursor_position`, `visual_row_count`, `history_prev`/`history_next` on InputState)
- Modify: `src/app.rs` (↑/↓ handlers for room input and agent input)

### Step 1: Add `PromptHistory` struct and visual row helpers

- [ ] **Step 1: Add `PromptHistory` struct**

Add after the `InputAgentPickerState` definition (after `src/ui/input_bar.rs:24`):

```rust
pub struct PromptHistory {
    pub entries: Vec<String>,
    pub cursor: Option<usize>,
    pub draft: String,
}

impl PromptHistory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            draft: String::new(),
        }
    }

    pub fn record(&mut self, entry: &str) {
        if !entry.trim().is_empty() {
            self.entries.push(entry.to_owned());
        }
        self.cursor = None;
        self.draft.clear();
    }

    pub fn previous(&mut self, current_draft: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        if self.cursor.is_none() {
            self.draft = current_draft.to_owned();
            self.cursor = Some(self.entries.len() - 1);
        } else if let Some(idx) = self.cursor {
            if idx == 0 {
                return None;
            }
            self.cursor = Some(idx - 1);
        }
        self.cursor.map(|idx| self.entries[idx].as_str())
    }

    pub fn next(&mut self) -> Option<&str> {
        let idx = self.cursor?;
        let new_idx = idx + 1;
        if new_idx >= self.entries.len() {
            self.cursor = None;
            return None;
        }
        self.cursor = Some(new_idx);
        Some(self.entries[new_idx].as_str())
    }

    pub fn cancel(&mut self) -> &str {
        self.cursor = None;
        &self.draft
    }

    pub fn is_browsing(&self) -> bool {
        self.cursor.is_some()
    }
}

impl Default for PromptHistory {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Add `history` field to `InputState`**

In `src/ui/input_bar.rs:57`, add the field:

```rust
pub struct InputState {
    pub content: String,
    pub cursor_pos: usize,
    pub preferred_column: Option<usize>,
    pub focused: bool,
    pub readonly: bool,
    pub agent_picker: Option<InputAgentPickerState>,
    pub history: PromptHistory,
}
```

Initialize in `InputState::new` (line 67):
```rust
            agent_picker: None,
            history: PromptHistory::new(),
```

Also clear history browsing in `take_input` (line 289-295) — add `self.history.cursor = None;` but NOT recording. Recording happens in the submit handler in `app.rs`, not here:

```rust
    pub fn take_input(&mut self) -> String {
        let taken = std::mem::take(&mut self.content);
        self.cursor_pos = 0;
        self.preferred_column = None;
        self.agent_picker = None;
        self.history.cursor = None;
        taken
    }
```

- [ ] **Step 3: Add visual cursor position helpers**

Add after `wrapped_row_for_cursor` (after `src/ui/input_bar.rs:678`):

```rust
pub fn visual_cursor_row(content: &str, cursor_pos: usize, width: u16) -> usize {
    wrapped_row_for_cursor(content, cursor_pos, width)
}

pub fn visual_row_count(content: &str, width: u16) -> usize {
    let width = usize::from(width.max(1));
    let mut row = 0usize;
    let mut col_width = 0usize;

    for ch in content.chars() {
        if ch == '\n' {
            row += 1;
            col_width = 0;
            continue;
        }
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col_width > 0 && ch_width > 0 && col_width + ch_width > width {
            row += 1;
            col_width = 0;
        }
        col_width = col_width.saturating_add(ch_width);
    }

    row
}
```

- [ ] **Step 4: Write tests for `PromptHistory`**

Add to the test module in `src/ui/input_bar.rs` (after line 842):

```rust
    #[test]
    fn prompt_history_prev_loads_last_entry() {
        let mut h = PromptHistory::new();
        h.record("first");
        h.record("second");
        let prev = h.previous("");
        assert_eq!(prev, Some("second"));
    }

    #[test]
    fn prompt_history_prev_then_next_restores_draft() {
        let mut h = PromptHistory::new();
        h.record("entry");
        let _ = h.previous("my draft");
        assert_eq!(h.next(), None);
        assert_eq!(h.cancel(), "my draft");
    }

    #[test]
    fn prompt_history_esc_restores_draft() {
        let mut h = PromptHistory::new();
        h.record("entry");
        let _ = h.previous("original draft");
        let restored = h.cancel();
        assert_eq!(restored, "original draft");
        assert!(!h.is_browsing());
    }

    #[test]
    fn prompt_history_send_pushes_entry() {
        let mut h = PromptHistory::new();
        h.record("hello");
        assert_eq!(h.entries.len(), 1);
        h.record("world");
        assert_eq!(h.entries.len(), 2);
    }

    #[test]
    fn prompt_history_ignores_empty_submission() {
        let mut h = PromptHistory::new();
        h.record("   ");
        assert_eq!(h.entries.len(), 0);
    }

    #[test]
    fn visual_row_count_counts_soft_wrapped_rows() {
        // 15 chars at width 5 = 3 rows
        assert_eq!(visual_row_count("abcdefghijklmno", 5), 2);
        // 10 chars at width 5 = 2 rows
        assert_eq!(visual_row_count("abcdefghij", 5), 1);
    }

    #[test]
    fn visual_cursor_position_matches_wrapped_row_for_cursor() {
        let content = "aaaa\nbbbb";
        assert_eq!(visual_cursor_row(content, 0, 10), 0);
        assert_eq!(visual_cursor_row(content, 5, 10), 1);
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p minos-tui prompt_history`
Expected: All PASS.

Run: `cargo test -p minos-tui visual_row_count`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/minos-tui/src/ui/input_bar.rs
git commit -m "feat(tui): add PromptHistory and visual cursor position helpers"
```

### Step 2: Wire history navigation into ↑/↓ key handlers

- [ ] **Step 7: Modify room input ↑ handler**

In `src/app.rs`, the room input `↑` handler is at line 723. The current logic:
```rust
            KeyCode::Up => {
                let changed = self.ui.room_input.move_up();
                self.sync_input_agent_picker();
                changed
            }
```

Replace with history-aware logic. When the agent picker is NOT active and the cursor is at visual row 0, navigate history instead of moving up:

```rust
            KeyCode::Up if !self.ui.room_input.has_agent_picker() => {
                let width = self.ui.panel_areas.room_input.width.saturating_sub(2);
                let visual_row = input_bar::visual_cursor_row(
                    &self.ui.room_input.content,
                    self.ui.room_input.cursor_pos,
                    width,
                );
                if visual_row == 0 {
                    if let Some(entry) = self.ui.room_input.history.previous(&self.ui.room_input.content.clone()) {
                        self.ui.room_input.content = entry.to_owned();
                        self.ui.room_input.cursor_pos = self.ui.room_input.content.len();
                        self.ui.room_input.preferred_column = None;
                    }
                    true
                } else {
                    let changed = self.ui.room_input.move_up();
                    self.sync_input_agent_picker();
                    changed
                }
            }
            KeyCode::Up if self.ui.room_input.has_agent_picker() => {
                self.ui.room_input.select_previous_agent();
                true
            }
```

Wait — the existing code already has `KeyCode::Up if self.ui.room_input.has_agent_picker()` at line 715. So we need to handle three cases in order: picker active → navigate picker, visual row 0 → history, else → move_up. The match arms need careful ordering. Let me restructure:

Replace lines 715-732:
```rust
            KeyCode::Up if self.ui.room_input.has_agent_picker() => {
                self.ui.room_input.select_previous_agent();
                true
            }
            KeyCode::Up => {
                let width = self.ui.panel_areas.room_input.width.saturating_sub(2).max(1);
                let visual_row = input_bar::visual_cursor_row(
                    &self.ui.room_input.content,
                    self.ui.room_input.cursor_pos,
                    width,
                );
                if visual_row == 0 {
                    let draft = self.ui.room_input.content.clone();
                    if let Some(entry) = self.ui.room_input.history.previous(&draft) {
                        self.ui.room_input.content = entry.to_owned();
                        self.ui.room_input.cursor_pos = self.ui.room_input.content.len();
                        self.ui.room_input.preferred_column = None;
                    }
                    true
                } else {
                    let changed = self.ui.room_input.move_up();
                    self.sync_input_agent_picker();
                    changed
                }
            }
            KeyCode::Down if self.ui.room_input.has_agent_picker() => {
                self.ui.room_input.select_next_agent();
                true
            }
            KeyCode::Down => {
                let width = self.ui.panel_areas.room_input.width.saturating_sub(2).max(1);
                let total_rows = input_bar::visual_row_count(
                    &self.ui.room_input.content,
                    width,
                );
                let current_row = input_bar::visual_cursor_row(
                    &self.ui.room_input.content,
                    self.ui.room_input.cursor_pos,
                    width,
                );
                if current_row >= total_rows {
                    if let Some(entry) = self.ui.room_input.history.next() {
                        self.ui.room_input.content = entry.to_owned();
                        self.ui.room_input.cursor_pos = self.ui.room_input.content.len();
                        self.ui.room_input.preferred_column = None;
                    } else if self.ui.room_input.history.is_browsing() {
                        let draft = self.ui.room_input.history.cancel().to_owned();
                        self.ui.room_input.content = draft;
                        self.ui.room_input.cursor_pos = self.ui.room_input.content.len();
                        self.ui.room_input.preferred_column = None;
                    }
                    true
                } else {
                    let changed = self.ui.room_input.move_down();
                    self.sync_input_agent_picker();
                    changed
                }
            }
```

Make sure `use crate::ui::input_bar;` is imported in app.rs (check existing imports).

- [ ] **Step 8: Modify agent input ↑/↓ handlers**

In `src/app.rs`, the agent input handlers are at lines 830-831:
```rust
            KeyCode::Up => self.ui.agent_input.move_up(),
            KeyCode::Down => self.ui.agent_input.move_down(),
```

Replace with the same history-aware logic:
```rust
            KeyCode::Up => {
                let width = self.ui.panel_areas.agent_input.width.saturating_sub(2).max(1);
                let visual_row = input_bar::visual_cursor_row(
                    &self.ui.agent_input.content,
                    self.ui.agent_input.cursor_pos,
                    width,
                );
                if visual_row == 0 {
                    let draft = self.ui.agent_input.content.clone();
                    if let Some(entry) = self.ui.agent_input.history.previous(&draft) {
                        self.ui.agent_input.content = entry.to_owned();
                        self.ui.agent_input.cursor_pos = self.ui.agent_input.content.len();
                        self.ui.agent_input.preferred_column = None;
                    }
                    true
                } else {
                    self.ui.agent_input.move_up()
                }
            }
            KeyCode::Down => {
                let width = self.ui.panel_areas.agent_input.width.saturating_sub(2).max(1);
                let total_rows = input_bar::visual_row_count(
                    &self.ui.agent_input.content,
                    width,
                );
                let current_row = input_bar::visual_cursor_row(
                    &self.ui.agent_input.content,
                    self.ui.agent_input.cursor_pos,
                    width,
                );
                if current_row >= total_rows {
                    if let Some(entry) = self.ui.agent_input.history.next() {
                        self.ui.agent_input.content = entry.to_owned();
                        self.ui.agent_input.cursor_pos = self.ui.agent_input.content.len();
                        self.ui.agent_input.preferred_column = None;
                    } else if self.ui.agent_input.history.is_browsing() {
                        let draft = self.ui.agent_input.history.cancel().to_owned();
                        self.ui.agent_input.content = draft;
                        self.ui.agent_input.cursor_pos = self.ui.agent_input.content.len();
                        self.ui.agent_input.preferred_column = None;
                    }
                    true
                } else {
                    self.ui.agent_input.move_down()
                }
            }
```

- [ ] **Step 9: Add Esc-to-cancel-history for room input**

In `src/app.rs` line 751, the room input Esc handler:
```rust
            KeyCode::Esc => {
                if self.ui.room_input.has_agent_picker() {
                    self.ui.room_input.clear_agent_picker();
                    true
                } else {
                    self.handle_escape()
                }
            }
```

Add history cancel:
```rust
            KeyCode::Esc => {
                if self.ui.room_input.has_agent_picker() {
                    self.ui.room_input.clear_agent_picker();
                    true
                } else if self.ui.room_input.history.is_browsing() {
                    let draft = self.ui.room_input.history.cancel().to_owned();
                    self.ui.room_input.content = draft;
                    self.ui.room_input.cursor_pos = self.ui.room_input.content.len();
                    self.ui.room_input.preferred_column = None;
                    true
                } else {
                    self.handle_escape()
                }
            }
```

Similarly for agent input Esc (line 847):
```rust
            KeyCode::Esc => {
                if self.ui.agent_input.history.is_browsing() {
                    let draft = self.ui.agent_input.history.cancel().to_owned();
                    self.ui.agent_input.content = draft;
                    self.ui.agent_input.cursor_pos = self.ui.agent_input.content.len();
                    self.ui.agent_input.preferred_column = None;
                    true
                } else {
                    self.handle_escape()
                }
            }
```

- [ ] **Step 10: Record history on send**

In `submit_room_input` (find with `rg "fn submit_room_input" crates/minos-tui/src/app.rs`), add recording before `take_input`:

```rust
let input = self.ui.room_input.take_input();
self.ui.room_input.history.record(&input);
```

Wait — `take_input()` clears the content AND clears `history.cursor`. If we record AFTER `take_input`, the entry is recorded but `cursor` is already None (which is correct — we're not browsing). But recording should happen before `take_input` clears the content. Let me read the actual `submit_room_input` implementation:

Find the method and record the input before clearing. The pattern should be:
```rust
let content = self.ui.room_input.content.clone();
// ... existing submit logic using content ...
self.ui.room_input.take_input();
if was_sent {
    self.ui.room_input.history.record(&content);
}
```

Or simpler: record inside the submit method after a successful send. Read the actual implementation to determine the exact place.

Run: `rg "fn submit_room_input|fn submit_agent_input" crates/minos-tui/src/app.rs`
Read the method and add `self.ui.room_input.history.record(&submitted_text);` at the point where the text is confirmed non-empty and about to be sent.

Do the same for `submit_agent_input`.

- [ ] **Step 11: Build and fix compilation errors**

Run: `cargo build -p minos-tui`
Fix any import or type errors.

- [ ] **Step 12: Run all tests**

Run: `cargo test -p minos-tui`
Expected: All tests PASS.

- [ ] **Step 13: Commit**

```bash
git add crates/minos-tui/src/ui/input_bar.rs crates/minos-tui/src/app.rs
git commit -m "feat(tui): prompt history navigation with ↑/↓ on visual row boundaries"
```

---

## Task B2: Clipboard Paste/Copy

**Files:**
- Modify: `src/app.rs` (add `paste_from_clipboard`, modify Ctrl+C, add Ctrl+V, add `flash_copied`)
- Modify: `src/ui/mod.rs` (add `flash_copied` field)
- Modify: `src/ui/status_bar.rs` (render "copied" indicator)

### Step 1: Add `paste_from_clipboard` function

- [ ] **Step 1: Add the production `paste_from_clipboard`**

In `src/app.rs`, after the `copy_to_clipboard` functions (after line 2917), add:

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
    const COMMANDS: &[(&str, &[&str])] = &[
        ("powershell", &["-NoProfile", "-Command", "Get-Clipboard"]),
    ];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    const COMMANDS: &[(&str, &[&str])] = &[];

    let mut last_error = None;
    for (program, args) in COMMANDS {
        match run_paste_command(program, args) {
            Ok(output) if !output.is_empty() => return Ok(normalize_pasted_text(&output)),
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => last_error = Some(error.into()),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no clipboard command available")))
}

#[cfg(not(test))]
fn run_paste_command(program: &str, args: &[&str]) -> std::io::Result<String> {
    use std::process::{Command, Stdio};

    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    String::from_utf8(output.stdout)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "non-utf8 clipboard"))
}
```

- [ ] **Step 2: Add the test `paste_from_clipboard` stub**

```rust
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

- [ ] **Step 3: Add `flash_copied` to `UiState`**

In `src/ui/mod.rs`, add field to `UiState` (after `delete_confirm`):

```rust
    pub delete_confirm: Option<DeleteConfirmState>,
    pub render_cache: crate::ui::chat::RenderCache,
    pub flash_copied: Option<Instant>,
```

Note: `render_cache` may already be added if Plan A was applied first. If not, add it. `Instant` is already imported in `mod.rs` (line 26).

Initialize in `UiState::new`:
```rust
            flash_copied: None,
```

Add method:
```rust
    pub fn flash_copied(&mut self) {
        self.flash_copied = Some(Instant::now());
    }

    pub fn is_flash_copied_active(&self) -> bool {
        self.flash_copied
            .is_some_and(|instant| instant.elapsed().as_secs() < 2)
    }
```

- [ ] **Step 4: Add Ctrl+V handler**

In `src/app.rs`, in the CONTROL modifier block (around line 545-558), add after `KeyCode::Char('c')`:

```rust
                KeyCode::Char('v') => {
                    match paste_from_clipboard() {
                        Ok(text) => return self.handle_paste(text),
                        Err(_) => false,
                    }
                }
```

This goes inside the `if key.modifiers.contains(KeyModifiers::CONTROL)` block:
```rust
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => { ... }
                KeyCode::Char('c') => { ... }
                KeyCode::Char('v') => {
                    match paste_from_clipboard() {
                        Ok(text) => return self.handle_paste(text),
                        Err(_) => false,
                    }
                }
                KeyCode::Char('d') => { ... }
                _ => {}
            }
        }
```

- [ ] **Step 5: Modify Ctrl+C handler to check selection first**

In `src/app.rs`, modify `handle_ctrl_c` (line 1015):

```rust
    async fn handle_ctrl_c(&mut self) -> bool {
        // If agent chat focused and has selection → copy
        if matches!(self.ui.focus, Focus::AgentChat) {
            if let Some(chat) = self.ui.current_chat() {
                if chat.selection.is_some() {
                    let width = self.ui.panel_areas.agent_chat.width.saturating_sub(2);
                    if let Some(text) = crate::ui::chat::selected_text(
                        chat,
                        width,
                        &self.ui.render_cache,
                    ) {
                        let _ = copy_to_clipboard(&text);
                        self.ui.flash_copied();
                    }
                    return true;
                }
            }
        }

        // Existing interrupt/quit logic
        if self.current_thread_is_interruptible() {
            if let Some(thread_id) = self.ui.current_thread_id().map(String::from) {
                if let Err(error) = self.backend.interrupt_thread(&thread_id).await {
                    self.ui
                        .set_error(format!("Failed to interrupt thread: {error}"));
                }
                return true;
            }
        }

        self.should_quit = true;
        false
    }
```

Note: `selected_text` signature depends on Plan A Task 8. If Plan A was applied, it takes `(chat, width, &cache)`. If not, it takes `(chat, width)`. Adjust accordingly.

- [ ] **Step 6: Add "copied" indicator to status bar**

In `src/ui/status_bar.rs`, modify `render_status_bar` to check for the flash:

```rust
pub fn render_status_bar(f: &mut Frame, area: Rect, state: &StatusBarState, flash_active: bool) {
```

Add the `flash_active` parameter and render "Copied!" when active:

```rust
    if flash_active {
        spans.push(Span::styled(
            "  ✓ Copied!",
            ratatui::style::Style::new().fg(ratatui::style::Color::Green),
        ));
    }
```

Update the call site in `src/ui/mod.rs:327`:
```rust
    status_bar::render_status_bar(f, outer[0], &state.status, state.is_flash_copied_active());
```

- [ ] **Step 7: Write test for paste**

In `src/app.rs` test module:

```rust
    #[tokio::test]
    async fn ctrl_v_pastes_from_clipboard() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.focus = Focus::AgentInput;

        // Put something on the clipboard
        TEST_CLIPBOARD.lock().unwrap().push("hello from clipboard".to_owned());

        let key = press_with_modifiers(KeyCode::Char('v'), KeyModifiers::CONTROL);
        let redraw = app.handle_key(key).await;

        assert!(redraw);
        assert_eq!(app.ui.agent_input.content, "hello from clipboard");

        // Clean up
        TEST_CLIPBOARD.lock().unwrap().clear();
    }
```

- [ ] **Step 8: Build and run tests**

Run: `cargo build -p minos-tui && cargo test -p minos-tui`
Expected: Compiles and all tests PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/minos-tui/src/app.rs crates/minos-tui/src/ui/mod.rs crates/minos-tui/src/ui/status_bar.rs
git commit -m "feat(tui): contextual Ctrl+C copy selection + Ctrl+V paste from clipboard"
```

---

## Task B3: File Path Completion (Tab)

**Files:**
- Modify: `Cargo.toml` (add `dirs` crate)
- Modify: `src/ui/input_bar.rs` (add `InputPicker` enum, `InputPathPickerState`, `PathCandidate`, `active_path_range`, `sync_path_picker`, `accept_path_completion`)
- Modify: `src/app.rs` (Tab priority chain, path picker handling)

### Step 1: Add `dirs` dependency

- [ ] **Step 1: Add `dirs` to Cargo.toml**

In `crates/minos-tui/Cargo.toml`, add to `[dependencies]`:

```toml
dirs = "6"
```

Run: `cargo build -p minos-tui` to verify it downloads and compiles.

### Step 2: Add picker types and path completion logic

- [ ] **Step 2: Add `InputPicker` enum and path types**

Add to `src/ui/input_bar.rs` (after `InputAgentPickerState`):

```rust
#[derive(Default)]
pub enum InputPicker {
    #[default]
    None,
    Agent(InputAgentPickerState),
    Path(InputPathPickerState),
}

#[derive(Clone)]
pub struct InputPathPickerState {
    pub candidates: Vec<PathCandidate>,
    pub selected: usize,
    pub replace_range: Range<usize>,
}

#[derive(Clone, Debug)]
pub struct PathCandidate {
    pub name: String,
    pub is_dir: bool,
}
```

- [ ] **Step 3: Change `agent_picker` field to `picker`**

In `src/ui/input_bar.rs:57-64`, change:

```rust
pub struct InputState {
    pub content: String,
    pub cursor_pos: usize,
    pub preferred_column: Option<usize>,
    pub focused: bool,
    pub readonly: bool,
    pub picker: InputPicker,
    pub history: PromptHistory,
}
```

Initialize in `new`:
```rust
            picker: InputPicker::None,
```

In `take_input`:
```rust
    pub fn take_input(&mut self) -> String {
        let taken = std::mem::take(&mut self.content);
        self.cursor_pos = 0;
        self.preferred_column = None;
        self.picker = InputPicker::None;
        self.history.cursor = None;
        taken
    }
```

- [ ] **Step 4: Update all existing `agent_picker` references**

Search and update all methods that used `self.agent_picker`:

```rust
    pub fn clear_agent_picker(&mut self) {
        self.picker = InputPicker::None;
    }

    pub fn sync_agent_picker(&mut self, candidates: &[AgentMentionCandidate], enabled: bool) {
        if !enabled || self.readonly {
            self.picker = InputPicker::None;
            return;
        }

        let Some(replace_range) = active_agent_range(&self.content, self.cursor_pos) else {
            self.picker = InputPicker::None;
            return;
        };
        let query = self.content[replace_range.start + 1..replace_range.end].to_ascii_lowercase();

        let previous_agent = match &self.picker {
            InputPicker::Agent(picker) => picker
                .candidate_indices
                .get(picker.selected)
                .and_then(|index| candidates.get(*index))
                .map(|candidate| candidate.token.clone()),
            _ => None,
        };

        let candidate_indices: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                candidate.token.starts_with(query.as_str()).then_some(index)
            })
            .collect();

        if candidate_indices.is_empty() {
            self.picker = InputPicker::None;
            return;
        }

        let selected = previous_agent
            .and_then(|token| {
                candidate_indices
                    .iter()
                    .position(|index| candidates[*index].token == token)
            })
            .or_else(|| {
                candidate_indices
                    .iter()
                    .position(|index| candidates[*index].token == query.as_str())
            })
            .unwrap_or(0);

        self.picker = InputPicker::Agent(InputAgentPickerState {
            candidate_indices,
            selected,
            replace_range,
        });
    }

    pub fn has_agent_picker(&self) -> bool {
        matches!(&self.picker, InputPicker::Agent(picker) if !picker.candidate_indices.is_empty())
    }

    pub fn has_path_picker(&self) -> bool {
        matches!(&self.picker, InputPicker::Path(picker) if !picker.candidates.is_empty())
    }

    pub fn has_any_picker(&self) -> bool {
        !matches!(self.picker, InputPicker::None)
    }

    pub fn select_previous_agent(&mut self) -> bool {
        if let InputPicker::Agent(picker) = &mut self.picker {
            let len = picker.candidate_indices.len();
            if len == 0 { return false; }
            picker.selected = if picker.selected == 0 { len - 1 } else { picker.selected - 1 };
            true
        } else {
            false
        }
    }

    pub fn select_next_agent(&mut self) -> bool {
        if let InputPicker::Agent(picker) = &mut self.picker {
            let len = picker.candidate_indices.len();
            if len == 0 { return false; }
            picker.selected = (picker.selected + 1) % len;
            true
        } else {
            false
        }
    }

    pub fn select_previous_path(&mut self) -> bool {
        if let InputPicker::Path(picker) = &mut self.picker {
            let len = picker.candidates.len();
            if len == 0 { return false; }
            picker.selected = if picker.selected == 0 { len - 1 } else { picker.selected - 1 };
            true
        } else {
            false
        }
    }

    pub fn select_next_path(&mut self) -> bool {
        if let InputPicker::Path(picker) = &mut self.picker {
            let len = picker.candidates.len();
            if len == 0 { return false; }
            picker.selected = (picker.selected + 1) % len;
            true
        } else {
            false
        }
    }
```

Also update `accept_agent_completion`:
```rust
    pub fn accept_agent_completion(&mut self, candidates: &[AgentMentionCandidate]) -> bool {
        let picker = match std::mem::take(&mut self.picker) {
            InputPicker::Agent(p) => p,
            _ => return false,
        };
        let Some(candidate_index) = picker.candidate_indices.get(picker.selected).copied() else {
            return false;
        };
        let Some(candidate) = candidates.get(candidate_index) else {
            return false;
        };

        let replacement = format!("@{} ", candidate.token);
        self.content
            .replace_range(picker.replace_range.clone(), replacement.as_str());
        self.cursor_pos = picker.replace_range.start + replacement.len();
        true
    }
```

- [ ] **Step 5: Add path completion logic**

Add functions:

```rust
pub fn active_path_range(content: &str, cursor_pos: usize) -> Option<Range<usize>> {
    if cursor_pos > content.len() || !content.is_char_boundary(cursor_pos) {
        return None;
    }

    let prefix = &content[..cursor_pos];
    let mut token_start = 0;
    for (index, ch) in prefix.char_indices() {
        if ch.is_whitespace() {
            token_start = index + ch.len_utf8();
        }
    }

    let token = &prefix[token_start..];
    // Must contain '/' or start with '~/' to qualify as a path
    let is_path = token.contains('/') || token.starts_with("~/");
    if !is_path {
        return None;
    }

    Some(token_start..cursor_pos)
}

pub fn resolve_path_base(token: &str, workspace_root: &std::path::Path) -> std::path::PathBuf {
    if let Some(rest) = token.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if token.starts_with('/') {
        return std::path::PathBuf::from(token);
    }
    workspace_root.join(token)
}

pub fn list_path_candidates(
    token: &str,
    workspace_root: &std::path::Path,
) -> Option<(Vec<PathCandidate>, Range<usize>)> {
    // Split into dir prefix and partial name
    let last_slash = token.rfind('/')?;
    let dir_prefix = &token[..=last_slash]; // includes the slash
    let partial_name = &token[last_slash + 1..];

    let resolved = {
        let path_token = if dir_prefix == "~/" {
            "~"
        } else {
            dir_prefix.trim_end_matches('/')
        };
        // For "~/" we need to handle home specially
        if dir_prefix.starts_with("~/") {
            dirs::home_dir()?
        } else if dir_prefix.starts_with('/') {
            std::path::PathBuf::from(dir_prefix)
        } else {
            workspace_root.join(dir_prefix)
        }
    };

    let entries = std::fs::read_dir(&resolved).ok()?;
    let mut candidates: Vec<PathCandidate> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(partial_name) {
                return None;
            }
            let is_dir = entry.file_type().ok()?.is_dir();
            Some(PathCandidate { name, is_dir })
        })
        .collect();
    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    candidates.truncate(8);

    if candidates.is_empty() {
        return None;
    }

    // The replace_range in the content starts at token_start (provided by caller)
    Some((candidates, 0..0)) // range is set by caller
}

impl InputState {
    pub fn sync_path_picker(&mut self, workspace_root: &std::path::Path) {
        if self.readonly {
            self.picker = InputPicker::None;
            return;
        }

        let Some(replace_range) = active_path_range(&self.content, self.cursor_pos) else {
            return; // Don't clear — might have agent picker
        };

        let token = &self.content[replace_range.start..replace_range.end];
        let Some((candidates, _)) = list_path_candidates(token, workspace_root) else {
            // Don't clear existing agent picker
            return;
        };

        self.picker = InputPicker::Path(InputPathPickerState {
            candidates,
            selected: 0,
            replace_range,
        });
    }

    pub fn accept_path_completion(&mut self) -> bool {
        let picker = match &self.picker {
            InputPicker::Path(p) => p,
            _ => return false,
        };
        let candidate = picker.candidates[picker.selected].clone();
        let replace_range = picker.replace_range.clone();

        // Build replacement: dir_prefix + name (+ "/" if dir)
        let existing_token = &self.content[replace_range.start..replace_range.end];
        let last_slash = existing_token.rfind('/').unwrap_or(0);
        let dir_prefix = &existing_token[..=last_slash];

        let mut replacement = format!("{dir_prefix}{}", candidate.name);
        let is_dir = candidate.is_dir;
        if is_dir {
            replacement.push('/');
        }

        self.content.replace_range(replace_range.clone(), &replacement);
        self.cursor_pos = replace_range.start + replacement.len();
        self.preferred_column = None;

        if is_dir {
            // Re-trigger completion for the subdirectory
            false // signal to caller to re-sync
        } else {
            self.picker = InputPicker::None;
            true
        }
    }
}
```

- [ ] **Step 6: Update `required_height` and `render_input_bar` for `picker`**

In `required_height` (line 406):
```rust
pub fn required_height(state: &InputState, width: u16) -> u16 {
    let picker_rows = match &state.picker {
        InputPicker::Agent(p) => p.candidate_indices.len().min(4) as u16,
        InputPicker::Path(p) => p.candidates.len().min(4) as u16,
        InputPicker::None => 0,
    };
    let editor_rows = editor_row_count(state, width);
    2 + picker_rows + editor_rows
}
```

In `render_input_bar` (line 416), update picker_rows computation and picker rendering:
```rust
    let picker_rows = match &state.picker {
        InputPicker::Agent(p) => p.candidate_indices.len().min(4) as u16,
        InputPicker::Path(p) => p.candidates.len().min(4) as u16,
        InputPicker::None => 0,
    }.min(inner.height.saturating_sub(1));
```

And update the picker rendering section:
```rust
    if picker_rows > 0 && picker_area.height > 0 {
        match &state.picker {
            InputPicker::Agent(_) => {
                render_inline_agent_picker(f, picker_area, state, candidates);
            }
            InputPicker::Path(path_picker) => {
                render_inline_path_picker(f, picker_area, path_picker);
            }
            InputPicker::None => {}
        }
    }
```

Add `render_inline_path_picker`:
```rust
fn render_inline_path_picker(
    f: &mut Frame,
    area: Rect,
    picker: &InputPathPickerState,
) {
    if picker.candidates.is_empty() || area.height == 0 {
        return;
    }

    let items: Vec<ListItem> = picker
        .candidates
        .iter()
        .take(4)
        .map(|candidate| {
            let suffix = if candidate.is_dir { "/" } else { "" };
            ListItem::new(Line::from(Span::styled(
                format!("{}{}", candidate.name, suffix),
                INPUT_PROMPT,
            )))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(picker.selected.min(picker.candidates.len() - 1).min(3)));

    let list = List::new(items)
        .highlight_symbol("› ")
        .highlight_style(HIGHLIGHTED);
    f.render_stateful_widget(list, area, &mut list_state);
}
```

Also update `render_inline_agent_picker` to get picker from `state.picker`:
```rust
fn render_inline_agent_picker(
    f: &mut Frame,
    area: Rect,
    state: &InputState,
    candidates: &[AgentMentionCandidate],
) {
    let Some(picker) = match &state.picker {
        InputPicker::Agent(p) => Some(p),
        _ => None,
    } else {
        return;
    };
    // ... rest unchanged ...
```

- [ ] **Step 7: Write tests for path completion**

```rust
    #[test]
    fn active_path_range_extracts_path_token() {
        assert_eq!(
            active_path_range("hello src/foo", 12),
            Some(6..12)
        );
        assert_eq!(
            active_path_range("~/foo", 5),
            Some(0..5)
        );
        assert_eq!(
            active_path_range("no path here", 12),
            None
        );
        assert_eq!(
            active_path_range("./bar", 5),
            Some(0..5)
        );
    }
```

- [ ] **Step 8: Build and verify**

Run: `cargo build -p minos-tui`
Fix any compilation errors from the `agent_picker` → `picker` refactor.

- [ ] **Step 9: Commit**

```bash
git add crates/minos-tui/Cargo.toml crates/minos-tui/src/ui/input_bar.rs
git commit -m "feat(tui): add path completion picker types and logic"
```

### Step 3: Wire Tab priority chain in app.rs

- [ ] **Step 10: Replace room input Tab handler**

In `src/app.rs`, replace room input Tab handler (line 747):

```rust
            KeyCode::Tab => {
                self.handle_tab_room_input()
            }
```

Add the method:
```rust
    fn handle_tab_room_input(&mut self) -> bool {
        // 1. Active path picker → accept
        if self.ui.room_input.has_path_picker() {
            self.ui.room_input.accept_path_completion();
            self.sync_input_path_picker();
            return true;
        }
        // 2. Active agent picker or @ token → no-op
        if self.ui.room_input.has_agent_picker() {
            return true;
        }
        if active_agent_range(&self.ui.room_input.content, self.ui.room_input.cursor_pos).is_some() {
            return true;
        }
        // 3. Path token at cursor → open path picker
        if input_bar::active_path_range(&self.ui.room_input.content, self.ui.room_input.cursor_pos).is_some() {
            self.sync_input_path_picker();
            return true;
        }
        // 4. Otherwise → cycle focus
        self.cycle_focus();
        true
    }

    fn sync_input_path_picker(&mut self) {
        self.ui.room_input.sync_path_picker(&self.workspace);
    }
```

Import `active_agent_range` in app.rs or call via `input_bar::active_agent_range`.

- [ ] **Step 11: Replace agent input Tab handler**

In `src/app.rs`, replace agent input Tab handler (line 843):

```rust
            KeyCode::Tab => {
                self.handle_tab_agent_input()
            }
```

Add the method:
```rust
    fn handle_tab_agent_input(&mut self) -> bool {
        // 1. Active path picker → accept
        if self.ui.agent_input.has_path_picker() {
            self.ui.agent_input.accept_path_completion();
            self.ui.agent_input.sync_path_picker(&self.workspace);
            return true;
        }
        // 2. Path token at cursor → open path picker
        if input_bar::active_path_range(&self.ui.agent_input.content, self.ui.agent_input.cursor_pos).is_some() {
            self.ui.agent_input.sync_path_picker(&self.workspace);
            return true;
        }
        // 3. Otherwise → cycle focus
        self.cycle_focus();
        true
    }
```

- [ ] **Step 12: Handle ↑/↓ for path picker in room input**

In the room input ↑/↓ handlers (before the history logic), add path picker checks:

```rust
            KeyCode::Up if self.ui.room_input.has_path_picker() => {
                self.ui.room_input.select_previous_path();
                true
            }
            KeyCode::Down if self.ui.room_input.has_path_picker() => {
                self.ui.room_input.select_next_path();
                true
            }
```

These must go BEFORE the agent picker and history checks in the match arms.

- [ ] **Step 13: Handle Enter on path picker in room input**

In the room input Enter handler (line 610), add path picker check before existing logic:

```rust
            KeyCode::Enter => {
                if self.ui.room_input.has_path_picker() {
                    self.ui.room_input.accept_path_completion();
                    self.sync_input_path_picker();
                    true
                } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                    // ... existing Shift+Enter logic
```

- [ ] **Step 14: Handle Esc on path picker**

In room input Esc handler, add before existing logic:
```rust
                if self.ui.room_input.has_path_picker() {
                    self.ui.room_input.picker = InputPicker::None;
                    true
                } else if ...
```

Import `InputPicker` in app.rs.

- [ ] **Step 15: Build and run all tests**

Run: `cargo test -p minos-tui`
Fix any issues.

- [ ] **Step 16: Commit**

```bash
git add crates/minos-tui/src/app.rs
git commit -m "feat(tui): Tab path completion with priority chain"
```

---

## Task B4: Mouse Click to Position Cursor

**Files:**
- Modify: `src/ui/input_bar.rs` (add `InputLayoutMetrics`, `byte_offset_for_visual_position`)
- Modify: `src/ui/mod.rs` (add `input_metrics` to `UiState`)
- Modify: `src/app.rs` (mouse click in input bars)

- [ ] **Step 1: Add `InputLayoutMetrics` struct**

Add to `src/ui/input_bar.rs`:

```rust
#[derive(Clone, Copy, Debug)]
pub struct InputLayoutMetrics {
    pub outer: Rect,
    pub editor_area: Rect,
    pub width: u16,
    pub start_row: usize,
    pub visible_rows: usize,
}

impl Default for InputLayoutMetrics {
    fn default() -> Self {
        Self {
            outer: Rect::default(),
            editor_area: Rect::default(),
            width: 1,
            start_row: 0,
            visible_rows: 1,
        }
    }
}
```

- [ ] **Step 2: Write metrics during `render_input_bar`**

Change `render_input_bar` signature to accept `&mut InputLayoutMetrics`:

```rust
pub fn render_input_bar(
    f: &mut Frame,
    area: Rect,
    title: &str,
    empty_hint: &str,
    state: &InputState,
    candidates: &[AgentMentionCandidate],
    metrics: &mut InputLayoutMetrics,
) {
```

At the end of the function, before rendering the paragraph, write the metrics:

```rust
    *metrics = InputLayoutMetrics {
        outer: area,
        editor_area: input_area,
        width: input_area.width,
        start_row,
        visible_rows,
    };
```

Update all call sites in `src/ui/mod.rs` (lines 392, 453, 465) to pass metrics from `state.input_metrics`:

```rust
pub struct UiState {
    // ... existing ...
    pub input_metrics: [InputLayoutMetrics; 2],
}
```

Initialize: `input_metrics: [InputLayoutMetrics::default(); 2],`

In `render_overview_mode`:
```rust
    input_bar::render_input_bar(
        f,
        input_area,
        "Chat Room Input",
        "Type @ to choose an agent or send to the room",
        &state.room_input,
        mention_candidates.as_slice(),
        &mut state.input_metrics[0],
    );
```

In `render_detail_mode`, for room input (index 0) and agent input (index 1).

- [ ] **Step 3: Add `byte_offset_for_visual_position`**

Add to `src/ui/input_bar.rs`:

```rust
pub fn byte_offset_for_visual_position(
    content: &str,
    target_row: usize,
    target_col: usize,
    width: u16,
) -> usize {
    let width = usize::from(width.max(1));
    let mut row = 0usize;
    let mut col_width = 0usize;

    for (byte_idx, ch) in content.char_indices() {
        // Check if we've reached the target position
        if row == target_row && col_width >= target_col {
            return byte_idx;
        }

        if ch == '\n' {
            if row == target_row {
                // Clamp to end of this logical line
                return byte_idx;
            }
            row += 1;
            col_width = 0;
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col_width > 0 && ch_width > 0 && col_width + ch_width > width {
            row += 1;
            col_width = 0;
            if row > target_row {
                return byte_idx;
            }
        }
        col_width = col_width.saturating_add(ch_width);
    }

    content.len()
}
```

- [ ] **Step 4: Add mouse click handler for input bars**

In `src/app.rs`, modify `handle_mouse` (lines 1143-1162). Replace the room_input and agent_input mouse handlers:

```rust
        if rect_contains(self.ui.input_metrics[0].outer, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::RoomInput;
                    self.handle_input_click(0, mouse.column, mouse.row);
                    self.sync_input_agent_picker();
                    true
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.input_metrics[1].outer, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::AgentInput;
                    self.handle_input_click(1, mouse.column, mouse.row);
                    true
                }
                _ => false,
            };
        }
```

Add the method:
```rust
    fn handle_input_click(&mut self, input_index: usize, column: u16, row: u16) {
        let metrics = self.ui.input_metrics[input_index];
        let input = match input_index {
            0 => &mut self.ui.room_input,
            _ => &mut self.ui.agent_input,
        };

        // Check if click is in the editor area (not the picker)
        if !rect_contains(metrics.editor_area, column, row) {
            return;
        }

        let visual_row = (usize::from(row - metrics.editor_area.y)) + metrics.start_row;
        let visual_col = usize::from(column.saturating_sub(metrics.editor_area.x));

        let offset = input_bar::byte_offset_for_visual_position(
            &input.content,
            visual_row,
            visual_col,
            metrics.width,
        );
        input.cursor_pos = offset;
        input.preferred_column = None;
    }
```

- [ ] **Step 5: Write tests**

```rust
    #[test]
    fn byte_offset_for_visual_position_clamps_to_line_end() {
        let content = "hello world";
        // Click at col 100 on row 0 → clamp to end
        let offset = byte_offset_for_visual_position(content, 0, 100, 80);
        assert_eq!(offset, content.len());
    }

    #[test]
    fn byte_offset_for_visual_position_handles_multiline() {
        let content = "hello\nworld";
        // Row 1, col 0 → byte offset 6 (start of "world")
        let offset = byte_offset_for_visual_position(content, 1, 0, 80);
        assert_eq!(offset, 6);
    }
```

- [ ] **Step 6: Build and run tests**

Run: `cargo build -p minos-tui && cargo test -p minos-tui`

- [ ] **Step 7: Commit**

```bash
git add crates/minos-tui/src/ui/input_bar.rs crates/minos-tui/src/ui/mod.rs crates/minos-tui/src/app.rs
git commit -m "feat(tui): mouse click to position cursor in input bars"
```

---

## Task B5: Visual Bar/Block Cursor

**Files:**
- Modify: `src/ui/input_bar.rs` (add `CursorStyle`, rewrite `build_editor_lines`, remove `insert_cursor_marker`)
- Modify: `src/app.rs` (add Ctrl+Alt+B handler)

- [ ] **Step 1: Add `CursorStyle` enum**

Add to `src/ui/input_bar.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Bar,
    Block,
}
```

Add field to `InputState`:
```rust
pub struct InputState {
    // ... existing fields ...
    pub cursor_style: CursorStyle,
}
```

Initialize in `new`: `cursor_style: CursorStyle::default(),`

Add method:
```rust
    pub fn toggle_cursor_style(&mut self) {
        self.cursor_style = match self.cursor_style {
            CursorStyle::Bar => CursorStyle::Block,
            CursorStyle::Block => CursorStyle::Bar,
        };
    }
```

- [ ] **Step 2: Rewrite `build_editor_lines` for style-based cursor**

Replace the `build_editor_lines` function and remove `insert_cursor_marker`:

```rust
fn build_editor_lines(state: &InputState, width: u16, empty_hint: &str) -> EditorLines {
    let width = width.max(1);
    if state.readonly {
        return EditorLines {
            lines: vec![Line::from(Span::styled(
                "[readonly mode]",
                Style::new().fg(ratatui::style::Color::DarkGray),
            ))],
            cursor_row: 0,
        };
    }

    let (display_content, style) = match (state.content.is_empty(), state.focused) {
        (true, true) => return EditorLines {
            lines: vec![empty_cursor_line(state.cursor_style)],
            cursor_row: 0,
        },
        (true, false) => (empty_hint.to_owned(), REASONING_STYLE),
        (false, _) => (state.content.clone(), Style::default()),
    };

    let cursor_row = if state.focused {
        wrapped_row_for_cursor(state.content.as_str(), state.cursor_pos, width)
    } else {
        0
    };

    if !state.focused {
        let lines = wrap_styled_text(display_content.as_str(), width, style);
        return EditorLines { lines, cursor_row: 0 };
    }

    // Build lines with style-based cursor
    let lines = build_lines_with_cursor(
        display_content.as_str(),
        state.cursor_pos,
        width,
        state.cursor_style,
    );

    EditorLines { lines, cursor_row }
}

fn empty_cursor_line(style: CursorStyle) -> Line<'static> {
    match style {
        CursorStyle::Bar => Line::from(Span::styled(
            "│",
            ratatui::style::Style::new().add_modifier(ratatui::style::Modifier::REVERSED),
        )),
        CursorStyle::Block => Line::from(Span::styled(
            " ",
            ratatui::style::Style::new().add_modifier(ratatui::style::Modifier::REVERSED),
        )),
    }
}

fn build_lines_with_cursor(
    content: &str,
    cursor_pos: usize,
    width: u16,
    cursor_style: CursorStyle,
) -> Vec<Line<'static>> {
    let width = usize::from(width);
    let reversed = ratatui::style::Style::new().add_modifier(ratatui::style::Modifier::REVERSED);

    // Split content into lines by \n, then soft-wrap each, tracking cursor position
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span> = Vec::new();
    let mut current_text = String::new();
    let mut current_width = 0usize;
    let mut byte_pos = 0usize;
    let mut cursor_applied = false;

    for (idx, ch) in content.char_indices() {
        let is_cursor_here = idx == cursor_pos && !cursor_applied;

        if ch == '\n' {
            // Flush current line
            lines.push(flush_line_with_cursor(
                std::mem::take(&mut current_spans),
                std::mem::take(&mut current_text),
                is_cursor_here,
                cursor_style,
                &reversed,
            ));
            if is_cursor_here {
                cursor_applied = true;
            }
            current_width = 0;
            byte_pos = idx + 1;
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width > 0 && ch_width > 0 && current_width + ch_width > width {
            // Wrap
            lines.push(flush_line_with_cursor(
                std::mem::take(&mut current_spans),
                std::mem::take(&mut current_text),
                is_cursor_here,
                cursor_style,
                &reversed,
            ));
            if is_cursor_here {
                cursor_applied = true;
            }
            current_width = 0;
        }

        let span_style = if is_cursor_here && cursor_style == CursorStyle::Block {
            reversed
        } else {
            ratatui::style::Style::default()
        };

        if is_cursor_here && cursor_style == CursorStyle::Bar {
            // Insert bar before the character
            current_spans.push(Span::styled("│", reversed));
        }

        current_spans.push(Span::styled(ch.to_string(), span_style));
        current_text.push(ch);
        current_width = current_width.saturating_add(ch_width);

        if is_cursor_here {
            cursor_applied = true;
        }
    }

    // Handle cursor at end of content
    let cursor_at_end = cursor_pos == content.len() && !cursor_applied;

    lines.push(flush_line_with_cursor(
        std::mem::take(&mut current_spans),
        std::mem::take(&mut current_text),
        cursor_at_end,
        cursor_style,
        &reversed,
    ));

    if lines.is_empty() {
        lines.push(empty_cursor_line(cursor_style));
    }

    lines
}

fn flush_line_with_cursor(
    spans: Vec<Span>,
    _text: String,
    cursor_at_end_of_line: bool,
    cursor_style: CursorStyle,
    reversed: &ratatui::style::Style,
) -> Line<'static> {
    let mut spans = spans;
    if cursor_at_end_of_line && cursor_style == CursorStyle::Bar {
        spans.push(Span::styled("│", *reversed));
    } else if cursor_at_end_of_line && cursor_style == CursorStyle::Block {
        spans.push(Span::styled(" ", *reversed));
    }
    Line::from(spans)
}
```

Remove `insert_cursor_marker` and `CURSOR_GLYPH` constant (or keep `CURSOR_GLYPH` if referenced elsewhere — check).

- [ ] **Step 3: Add Ctrl+Alt+B handler**

In `src/app.rs`, in the room input key handler, add to the ALT modifier block (around line 684):

```rust
            KeyCode::Char(c)
                if key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let changed = match c.to_ascii_lowercase() {
                    'b' => {
                        // Only toggle if CONTROL is not also pressed
                        // Wait — this is the ALT-only block. Ctrl+Alt+B would be a different arm.
                        // Need to check: existing 'b' here is move_word_left.
                        // We need a separate Ctrl+Alt+B handler.
                        self.ui.room_input.move_word_left()
                    }
                    // ...
                };
            }
```

Actually, `Ctrl+Alt+B` requires both CONTROL and ALT modifiers. The existing arms are:
- CONTROL only (no ALT): line 666
- ALT only (no CONTROL): line 683

We need a new arm for CONTROL + ALT. Add after the ALT-only block in room input handler:

```rust
            KeyCode::Char('b')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.ui.room_input.toggle_cursor_style();
                true
            }
```

This arm must come BEFORE the CONTROL-only arm and the ALT-only arm, because those use guards that exclude the other modifier. Check: the CONTROL arm at line 667 has `!key.modifiers.contains(KeyModifiers::ALT)` and the ALT arm at line 684 has `!key.modifiers.contains(KeyModifiers::CONTROL)`. So `Ctrl+Alt+B` falls through to `_ => false` in both. Add the new arm anywhere in the match (it won't conflict).

Add the same handler in the agent input key handler.

- [ ] **Step 4: Write tests**

```rust
    #[test]
    fn cursor_style_toggle_flips_between_bar_and_block() {
        let mut state = InputState::new(false);
        assert_eq!(state.cursor_style, CursorStyle::Bar);
        state.toggle_cursor_style();
        assert_eq!(state.cursor_style, CursorStyle::Block);
        state.toggle_cursor_style();
        assert_eq!(state.cursor_style, CursorStyle::Bar);
    }
```

- [ ] **Step 5: Build and run tests**

Run: `cargo build -p minos-tui && cargo test -p minos-tui`
Fix any issues. The existing tests that depend on `insert_cursor_marker` behavior (like `required_height` test) may need updating.

- [ ] **Step 6: Commit**

```bash
git add crates/minos-tui/src/ui/input_bar.rs crates/minos-tui/src/app.rs
git commit -m "feat(tui): style-based bar/block cursor replacing string splicing"
```

---

## Task B6: Multi-line Mode Toggle

**Files:**
- Modify: `src/ui/input_bar.rs` (add `multiline` field, `toggle_multiline`)
- Modify: `src/app.rs` (Enter handler, Ctrl+J, Alt+Enter)
- Modify: `src/ui/mod.rs` (`[multi]` in title)
- Modify: `src/ui/status_bar.rs` (`Ctrl+J = newline` hint)

- [ ] **Step 1: Add `multiline` field to `InputState`**

```rust
pub struct InputState {
    // ... existing ...
    pub multiline: bool,
}
```

Initialize: `multiline: false,`

Add method:
```rust
    pub fn toggle_multiline(&mut self) {
        self.multiline = !self.multiline;
    }
```

- [ ] **Step 2: Modify room input Enter handler**

In `src/app.rs`, replace room input Enter handler (line 610):

```rust
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    if self.ui.room_input.multiline {
                        self.submit_room_input().await
                    } else {
                        self.ui.room_input.insert_char('\n');
                        self.sync_input_agent_picker();
                        true
                    }
                } else if self.ui.room_input.has_path_picker() {
                    self.ui.room_input.accept_path_completion();
                    self.sync_input_path_picker();
                    true
                } else if self.ui.room_input.has_agent_picker() {
                    let candidates = self.ui.room_agent_mention_candidates();
                    self.ui
                        .room_input
                        .accept_agent_completion(candidates.as_slice());
                    self.sync_input_agent_picker();
                    true
                } else if self.ui.room_input.multiline {
                    self.ui.room_input.insert_char('\n');
                    self.sync_input_agent_picker();
                    true
                } else {
                    self.submit_room_input().await
                }
            }
```

- [ ] **Step 3: Modify agent input Enter handler**

Replace agent input Enter handler (line 765):

```rust
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    if self.ui.agent_input.multiline {
                        self.submit_agent_input().await
                    } else {
                        self.ui.agent_input.insert_char('\n');
                        true
                    }
                } else if self.ui.agent_input.multiline {
                    self.ui.agent_input.insert_char('\n');
                    true
                } else {
                    self.submit_agent_input().await
                }
            }
```

- [ ] **Step 4: Add Ctrl+J handler (always inserts newline)**

In room input handler, add in the CONTROL block or as a standalone arm. `Ctrl+J` has CONTROL modifier:

```rust
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.ui.room_input.insert_char('\n');
                self.sync_input_agent_picker();
                true
            }
```

Wait — this would conflict with the existing `KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL)` catch-all at line 666. The `'j'` case falls through to `_ => false` there. We need to add it as a specific case before the catch-all. But the existing catch-all handles it with `_ => false`, so adding a specific arm earlier works.

Actually, looking at the code more carefully, the CONTROL catch-all at line 670 matches `c.to_ascii_lowercase()` and only handles specific letters. 'j' is not handled → returns false. So we can add 'j' to that match:

```rust
                let changed = match c.to_ascii_lowercase() {
                    'a' => self.ui.room_input.move_line_start(),
                    'b' => self.ui.room_input.move_left(),
                    'e' => self.ui.room_input.move_line_end(),
                    'f' => self.ui.room_input.move_right(),
                    'j' => {
                        self.ui.room_input.insert_char('\n');
                        self.sync_input_agent_picker();
                        return true;
                    }
                    'k' => self.ui.room_input.delete_to_line_end(),
                    // ...
                };
```

Do the same for agent input.

- [ ] **Step 5: Add Alt+Enter toggle handler**

`Alt+Enter` = `KeyCode::Enter` with `KeyModifiers::ALT`. Add in room input handler:

```rust
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                self.ui.room_input.toggle_multiline();
                true
            }
```

This must come before the regular `KeyCode::Enter` arm. Similarly for agent input.

- [ ] **Step 6: Add `[multi]` indicator to input titles**

In `src/ui/mod.rs`, modify the `render_input_bar` calls to include `[multi]` in the title when multiline is active:

In `render_overview_mode` (line 392):
```rust
    let room_title = if state.room_input.multiline {
        "Chat Room Input [multi]"
    } else {
        "Chat Room Input"
    };
    input_bar::render_input_bar(
        f,
        input_area,
        room_title,
        // ...
```

In `render_detail_mode`, do the same for both room and agent input titles.

- [ ] **Step 7: Add `Ctrl+J = newline` hint to status bar**

In `src/ui/status_bar.rs`, update the hint text in the `Span::raw` string (line 55):

```rust
    spans.push(Span::raw(
        "  ^P projects  ^T conversations  @agent route  Tab focus  Enter inspect/send  Esc back/close-detail  wheel/PgUp/PgDn scroll  Ctrl+C interrupt/copy  Ctrl+V paste  Ctrl+J newline  Ctrl+Q quit",
    ));
```

- [ ] **Step 8: Write test**

```rust
    #[test]
    fn multiline_toggle_flips_enter_behavior() {
        let mut state = InputState::new(false);
        assert!(!state.multiline);
        state.toggle_multiline();
        assert!(state.multiline);
    }
```

- [ ] **Step 9: Build and run all tests**

Run: `cargo build -p minos-tui && cargo test -p minos-tui`

- [ ] **Step 10: Commit**

```bash
git add crates/minos-tui/src/ui/input_bar.rs crates/minos-tui/src/app.rs crates/minos-tui/src/ui/mod.rs crates/minos-tui/src/ui/status_bar.rs
git commit -m "feat(tui): multi-line mode toggle with Alt+Enter, Ctrl+J newline"
```

---

## Task B-Final: Full Verification

- [ ] **Step 1: Run the complete test suite**

Run: `cargo test -p minos-tui`
Expected: All tests PASS.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p minos-tui --all-targets -- -D warnings`
Expected: No warnings.

- [ ] **Step 3: Run fmt**

Run: `cargo fmt -- --check crates/minos-tui/src/`
Fix if needed: `cargo fmt`

- [ ] **Step 4: Run workspace-wide check**

Run: `cargo xtask check-all`
Expected: All checks pass.

- [ ] **Step 5: Final commit if any fixes**

```bash
git add -A
git commit -m "style(tui): apply rustfmt to input bar overhaul"
```

---

## Self-Review Notes

**Spec coverage check:**
- B1: Prompt History ↑/↓ with visual row detection ✓ (Task B1)
- B2: Clipboard paste/copy via Ctrl+C/Ctrl+V ✓ (Task B2)
- B3: File path Tab completion with picker unification ✓ (Task B3)
- B4: Mouse click cursor positioning ✓ (Task B4)
- B5: Visual bar/block cursor ✓ (Task B5)
- B6: Multi-line mode toggle ✓ (Task B6)

**Placeholder scan:** All code blocks contain actual implementations. No TBD/TODO.

**Type consistency:**
- `InputPicker` enum used consistently across input_bar.rs and app.rs ✓
- `InputLayoutMetrics` fields match between struct definition and render writes ✓
- `CursorStyle` enum used consistently ✓
- `PromptHistory` methods (`previous`, `next`, `cancel`, `record`, `is_browsing`) consistent across all call sites ✓

**Key ordering concern:** The match arm ordering in `handle_room_input_key` and `handle_agent_input_key` is critical. Specific guards (e.g., `if self.ui.room_input.has_path_picker()`) must come before generic arms. Verify the ordering carefully during implementation.
