# P1: 帧合并 + Group Chat RenderCache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 引入帧率限制的合并重绘机制(FrameRequester),消除高频流式更新时的终端压力;给 group chat 引入 RenderCache,复用 agent chat 已有的 viewport 切片模式。

**Architecture:** `FrameRequester` 把 redraw request 发送给后台 `FrameScheduler`，scheduler 合并突发请求并按 `MIN_FRAME_INTERVAL` 发出 frame token；主事件循环只在收到 token 时 draw，不在 frame branch 内 sleep。`GroupChatState` 持有 `version` + `render_cache`，`render_group_chat()` 按 `(version, width)` 缓存已 wrap 的消息 block，并从缓存切出 viewport。

**Tech Stack:** Rust, tokio, ratatui 0.29。

**Spec:** `docs/superpowers/specs/2026-06-17-tui-three-phase-refactor-design.md` §4

**Test command:** `cargo test -p minos-tui`
**Build command:** `cargo build -p minos-tui`

**Prerequisite:** P0 (A+B+C) 已完成。

**Status 2026-06-17:** P1 implementation, documentation sync, and verification are complete in the working tree. Git commit steps are intentionally not performed by Codex unless explicitly requested.

**Completed shape:**
- `src/frame.rs` provides coalesced `FrameRequester` / `FrameRequestReceiver` plus an internal `FrameScheduler` with `MIN_FRAME_INTERVAL = 33ms`.
- `main.rs` listens to app events and scheduler-emitted frame tokens with `tokio::select!`; redraws are scheduled by App instead of being performed directly from event return values.
- Draw throttling lives in the scheduler, not as `sleep()` inside the main frame branch, so frame pacing does not block event ingestion.
- `GroupChatState` tracks `version` and owns `GroupChatRenderCache`.
- `render_group_chat()` rebuilds cache by `(version, width)`, computes max scroll from cached visual line count, and slices already-wrapped cached viewport lines.
- Duplicate/unchanged group chat message merges do not bump `version`, preserving the render cache across no-op daemon refreshes.
- Verification command used: `cargo fmt -p minos-tui && CARGO_BUILD_JOBS=1 cargo test -p minos-tui && CARGO_BUILD_JOBS=1 cargo clippy -p minos-tui --all-targets -- -D warnings`.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/frame.rs` | `FrameRequester` — 合并帧请求 |
| `src/main.rs` | 事件循环 select! 增加 frame 源 |
| `src/update/mod.rs` | 替换 `StateChange::needs_redraw` 为 `frame_requester.schedule_frame()` |
| `src/ui/group_chat.rs` | 引入 RenderCache, viewport 切片渲染 |
| `src/ui/mod.rs` | GroupChatState 加 version + render_cache |

---

## Task 1: 创建 FrameRequester

**Files:**
- Create: `src/frame.rs`
- Modify: `src/main.rs` — 添加 `mod frame;`

- [x] **Step 1: 创建 src/frame.rs**

```rust
//! 帧请求合并器。
//!
//! update() 调用 `schedule_frame()` 请求重绘。
//! 事件循环通过 frame_rx 接收请求, 按 MIN_FRAME_INTERVAL 节流。

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

/// 帧间最小间隔 (约 30fps 上限)。
pub const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(33);

/// 帧请求发送端。clone-safe。
/// 调用 schedule_frame() 是幂等的 — 多次调用在 frame_rx 端只产生一条消息。
#[derive(Clone)]
pub struct FrameRequester {
    tx: tokio::sync::mpsc::UnboundedSender<()>,
    pending: Arc<AtomicBool>,
}

pub struct FrameRequestReceiver {
    rx: tokio::sync::mpsc::UnboundedReceiver<()>,
    pending: Arc<AtomicBool>,
}

impl FrameRequester {
    pub fn schedule_frame(&self) {
        if self
            .pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _ = self.tx.send(());
        }
    }
}

/// 创建帧请求 channel, 返回 (FrameRequester, FrameRequestReceiver)。
pub fn frame_channel() -> (FrameRequester, FrameRequestReceiver) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let pending = Arc::new(AtomicBool::new(false));
    (
        FrameRequester { tx, pending: Arc::clone(&pending) },
        FrameRequestReceiver { rx, pending },
    )
}
```

- [x] **Step 2: 添加 mod frame; 到 main.rs**

- [x] **Step 3: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -10`
Expected: BUILD SUCCEEDED

- [x] **Step 4: Commit (skipped by Codex unless explicitly requested)**

```bash
git add crates/minos-tui/src/frame.rs crates/minos-tui/src/main.rs
git commit -m "feat(tui): add FrameRequester for coalesced frame scheduling"
```

---

## Task 2: 事件循环增加 frame 源

**Files:**
- Modify: `src/main.rs` — run_main 的事件循环
- Modify: `src/app.rs` / `src/app/lifecycle.rs` / `src/app/event_loop.rs` — App 持有 FrameRequester 并在 redraw 路径 schedule frame

- [x] **Step 1: 修改事件循环**

当前事件循环 (main.rs ~416–430):
```rust
loop {
    if let Some(event) = rx.recv().await {
        if app.handle_event(event).await {
            terminal.draw(|f| { ui::render_ui(f, app.ui()); })?;
        }
    } else { break; }
    if app.should_quit() { break; }
}
```

改为:
```rust
let (frame_requester, mut frame_rx) = crate::frame::frame_channel();
app.set_frame_requester(frame_requester);

loop {
    tokio::select! {
        event = rx.recv() => {
            let Some(event) = event else { break; };
            let _ = app.handle_event(event).await;
        }
        frame = frame_rx.recv() => {
            if frame.is_none() { break; }
            // FrameScheduler has already coalesced and paced this token.
            terminal.draw(|f| { ui::render_ui(f, app.ui()); })?;
        }
    }

    if app.should_quit() { break; }
}
```

初始 draw 保留；draw pacing 由 `FrameScheduler` 维护，不需要 main loop 持有 `last_draw`。

- [x] **Step 2: App 持有 FrameRequester**

在 `src/app.rs`:
```rust
pub struct App {
    // ... 现有字段
    frame_requester: Option<crate::frame::FrameRequester>,
}

impl App {
    pub fn set_frame_requester(&mut self, fr: crate::frame::FrameRequester) {
        self.frame_requester = Some(fr);
    }

    fn request_frame(&self) {
        if let Some(fr) = &self.frame_requester {
            fr.schedule_frame();
        }
    }
}
```

- [x] **Step 3: handle_event schedule frame**

`handle_event` 的 `bool` 返回值保留给测试和过渡调用者；main loop 不再用它直接 draw。状态变更后由 `apply_action()`、输入/paste 分支或 resize 分支调用 `self.request_frame()`。

如果 P0 已完成,handle_event 内部调 update(),update 返回 StateChange。改为:
```rust
let (change, effects) = update(&mut self.state, &mut self.ui, action);
let effects_redraw = self.execute_effects(effects).await;
if change.needs_redraw || effects_redraw {
    self.request_frame();
}
```

- [x] **Step 4: 保留初始 draw**

main.rs 在事件循环前仍有一次初始 draw:
```rust
terminal.draw(|f| { ui::render_ui(f, app.ui()); })?;
```

- [x] **Step 5: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -20`
Expected: BUILD SUCCEEDED

- [x] **Step 6: 运行测试**

Run: `cargo test -p minos-tui 2>&1 | tail -20`
Expected: 全部通过。测试中可能需要 mock FrameRequester — 给 App::new 一个 no-op requester 或在测试中跳过 frame 逻辑。

- [x] **Step 7: Commit (skipped by Codex unless explicitly requested)**

```bash
git add -A crates/minos-tui/src/
git commit -m "feat(tui): coalesce redraws via FrameRequester with 30fps cap"
```

---

## Task 3: GroupChatState 加 version + render_cache

**Files:**
- Modify: `src/ui/mod.rs` — GroupChatState struct

- [x] **Step 1: 给 GroupChatState 加字段**

当前 GroupChatState (ui/mod.rs 80–85):
```rust
pub struct GroupChatState {
    pub messages: Vec<LocalGroupChatMessage>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub max_scroll: u16,
}
```

改为:
```rust
pub struct GroupChatState {
    pub messages: Vec<LocalGroupChatMessage>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub max_scroll: u16,
    pub version: u64,
    pub render_cache: GroupChatRenderCache,
}
```

- [x] **Step 2: 定义 GroupChatRenderCache**

在 `src/ui/group_chat.rs`:

```rust
use crate::ui::chat::RenderCache as ChatRenderCache;

/// Group chat 的渲染缓存, 复用 ChatRenderCache 的 viewport 切片逻辑。
/// 注意: ChatRenderCache 索引的是 ChatItem, group chat 索引的是 LocalGroupChatMessage。
/// 需要一个 group-chat 专用的缓存结构。
pub struct GroupChatRenderCache {
    /// 缓存构建时的消息版本
    indexed_version: u64,
    /// 缓存构建时的宽度
    indexed_width: u16,
    /// 每条消息的起始视觉行号
    message_starts: Vec<usize>,
    /// 总视觉行数
    total_lines: usize,
}

impl Default for GroupChatRenderCache {
    fn default() -> Self {
        Self {
            indexed_version: 0,
            indexed_width: 0,
            message_starts: Vec::new(),
            total_lines: 0,
        }
    }
}
```

**设计决策:** group chat 的消息结构(LocalGroupChatMessage)与 agent chat 的 ChatItem 不同,不能直接复用 `RenderCache`。但可以复用相同的 *算法* (per-item 起始行索引 + visible_window 切片)。这里新建一个专用缓存,内部算法与 ChatRenderCache 一致。

- [x] **Step 3: 实现 rebuild_if_stale + visible_window**

```rust
impl GroupChatRenderCache {
    pub fn is_valid(&self, version: u64, width: u16) -> bool {
        self.indexed_version == version && self.indexed_width == width
    }

    pub fn rebuild_if_stale(&mut self, messages: &[LocalGroupChatMessage], version: u64, width: u16) {
        if self.is_valid(version, width) {
            return;
        }
        self.rebuild(messages, width);
        self.indexed_version = version;
        self.indexed_width = width;
    }

    fn rebuild(&mut self, messages: &[LocalGroupChatMessage], width: u16) {
        let mut starts = Vec::with_capacity(messages.len());
        let mut current = 0usize;

        for (idx, msg) in messages.iter().enumerate() {
            if idx > 0 {
                starts.push(current);
                current += 1; // 分隔空行
            } else {
                starts.push(current);
            }
            // label 行
            current += 1;
            // 消息正文 wrap 行
            for raw_line in msg.text.split('\n') {
                current += count_wrapped_lines(raw_line, width);
            }
        }

        self.message_starts = starts;
        self.total_lines = current;
    }

    /// 返回可见窗口的消息切片 + 起始偏移。
    pub fn visible_window(
        &self,
        messages: &[LocalGroupChatMessage],
        viewport_height: u16,
        scroll_offset: u16,
    ) -> GroupChatVisibleWindow {
        // 二分查找 scroll_offset 对应的消息索引
        // 返回该消息 + 后续消息直到填满 viewport
        // ... (算法与 ChatRenderCache::visible_window 一致)
    }
}

pub struct GroupChatVisibleWindow<'a> {
    pub messages: &'a [LocalGroupChatMessage],
    pub start_message_index: usize,
    pub line_offset_within_first_message: usize,
}

fn count_wrapped_lines(raw: &str, width: u16) -> usize {
    // 复用 ui/chat.rs 的 visual_line_count 逻辑或 wrap_plain_line 计数
    if raw.is_empty() { return 1; }
    let width = usize::from(width.max(1));
    let mut lines = 1usize;
    let mut current = 0usize;
    for ch in raw.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if current > 0 && cw > 0 && current + cw > width {
            lines += 1;
            current = 0;
        }
        current += cw;
    }
    lines
}
```

- [x] **Step 4: 更新 GroupChatState::new**

```rust
impl GroupChatState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            auto_scroll: true,
            max_scroll: 0,
            version: 0,
            render_cache: GroupChatRenderCache::default(),
        }
    }

    /// 任何消息增删改时调用。
    pub fn bump_version(&mut self) {
        self.version += 1;
    }
}
```

- [x] **Step 5: 在所有消息变更处 bump version**

搜索所有修改 `group_chat.messages` 的地方:
Run: `rg "group_chat\.messages\." crates/minos-tui/src/ -l`
在每个 push/extend/remove 后添加 `group_chat.bump_version()`。

- [x] **Step 6: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -20`
Expected: BUILD SUCCEEDED

- [x] **Step 7: Commit (skipped by Codex unless explicitly requested)**

```bash
git add -A crates/minos-tui/src/
git commit -m "feat(tui): add version tracking and render cache to GroupChatState"
```

---

## Task 4: render_group_chat 改为 viewport 切片

**Files:**
- Modify: `src/ui/group_chat.rs` — render_group_chat

- [x] **Step 1: 重写 render_group_chat**

当前 render_group_chat (group_chat.rs 14–51) 全量 build_lines。改为:

```rust
pub fn render_group_chat(
    f: &mut Frame,
    area: Rect,
    title: &str,
    state: &mut GroupChatState,
    focused: bool,
) {
    let block = super::theme::border_block()
        .title(title)
        .border_style(if focused {
            super::theme::FOCUSED_BORDER
        } else {
            ratatui::style::Style::new().fg(BORDER_FG)
        });
    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        f.render_widget(block, area);
        return;
    }

    if state.messages.is_empty() {
        let paragraph = Paragraph::new(vec![Line::from(Span::styled(
            "No group messages yet.",
            REASONING_STYLE,
        ))]).block(block);
        f.render_widget(paragraph, area);
        return;
    }

    // 用缓存索引
    state.render_cache.rebuild_if_stale(
        &state.messages,
        state.version,
        inner.width,
    );

    let max_scroll = state.render_cache.total_lines
        .saturating_sub(usize::from(inner.height))
        .min(usize::from(u16::MAX)) as u16;
    state.update_max_scroll(max_scroll);

    // 切片可见窗口
    let window = state.render_cache.visible_window(
        &state.messages,
        inner.height,
        state.active_scroll(),
    );

    // 只渲染可见消息
    let lines = build_visible_lines(
        window.messages,
        window.line_offset_within_first_message,
        inner.width,
        inner.height as usize,
    );

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

fn build_visible_lines(
    messages: &[LocalGroupChatMessage],
    line_offset_in_first: usize,
    width: u16,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(max_lines + 2);
    let mut skip = line_offset_in_first;

    for (idx, msg) in messages.iter().enumerate() {
        if lines.len() >= max_lines { break; }

        if idx > 0 {
            // 分隔空行
            if skip == 0 {
                lines.push(Line::from(""));
                if lines.len() >= max_lines { break; }
            } else {
                skip = skip.saturating_sub(1);
            }
        }

        // label 行
        let (label, style) = label_for_message(msg);
        if skip == 0 {
            lines.push(Line::from(Span::styled(label, style)));
            if lines.len() >= max_lines { break; }
        } else {
            skip = skip.saturating_sub(1);
        }

        // 正文行
        for raw_line in msg.text.split('\n') {
            let wrapped = wrap_plain_line(raw_line, width);
            for w in &wrapped {
                if skip > 0 {
                    skip = skip.saturating_sub(1);
                } else {
                    lines.push(w.clone());
                    if lines.len() >= max_lines { break; }
                }
            }
            if lines.len() >= max_lines { break; }
        }
    }

    lines
}
```

- [x] **Step 2: 实现 visible_window 的二分查找**

在 GroupChatRenderCache 上实现:

```rust
pub fn visible_window(
    &self,
    messages: &[LocalGroupChatMessage],
    viewport_height: u16,
    scroll_offset: u16,
) -> GroupChatVisibleWindow {
    let offset = usize::from(scroll_offset);
    let starts = &self.message_starts;

    if starts.is_empty() || offset >= self.total_lines {
        return GroupChatVisibleWindow {
            messages: &[],
            start_message_index: 0,
            line_offset_within_first_message: 0,
        };
    }

    // 二分查找第一个 start <= offset 的消息
    let msg_idx = starts.partition_point(|&s| s <= offset);
    let msg_idx = if msg_idx > 0 { msg_idx - 1 } else { 0 };
    let msg_start = starts[msg_idx];
    let line_offset = offset - msg_start;

    // 剩余消息全部返回 (build_visible_lines 会按 viewport 截断)
    GroupChatVisibleWindow {
        messages: &messages[msg_idx..],
        start_message_index: msg_idx,
        line_offset_within_first_message: line_offset,
    }
}
```

- [x] **Step 3: 保留旧的 build_lines 测试 + 添加 viewport 测试**

现有测试 (group_chat.rs 134–156) 测试 `label_for_message`。添加:

```rust
#[test]
fn render_cache_indexes_message_starts() {
    let mut cache = GroupChatRenderCache::default();
    let messages = vec![
        test_message("hello"),    // 1 label + 1 text = 2 lines
        test_message("world"),    // 1 sep + 1 label + 1 text = 3 lines
    ];
    cache.rebuild_if_stale(&messages, 1, 80);
    assert_eq!(cache.total_lines, 5); // 2 + 3
    assert_eq!(cache.message_starts, vec![0, 2]);
}

#[test]
fn visible_window_slices_correctly() {
    let mut cache = GroupChatRenderCache::default();
    let messages = vec![
        test_message("aaa"),  // lines 0-1
        test_message("bbb"),  // lines 2-4
        test_message("ccc"),  // lines 5-7
    ];
    cache.rebuild_if_stale(&messages, 1, 80);

    let win = cache.visible_window(&messages, 2, 3); // scroll=3
    assert_eq!(win.start_message_index, 1); // message "bbb"
    assert_eq!(win.line_offset_within_first_message, 1); // 3 - 2 = 1
}
```

- [x] **Step 4: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -10`
Expected: BUILD SUCCEEDED

- [x] **Step 5: 运行测试**

Run: `cargo test -p minos-tui 2>&1 | tail -20`
Expected: 全部通过

- [x] **Step 6: clippy**

Run: `cargo clippy -p minos-tui --all-targets -- -D warnings 2>&1 | tail -20`
Expected: 无 warnings

- [x] **Step 7: Commit (skipped by Codex unless explicitly requested)**

```bash
git add -A crates/minos-tui/src/
git commit -m "perf(tui): viewport-sliced group chat rendering via render cache"
```

---

## Task 5: 更新 architecture-tui.md

- [x] **Step 1: 更新性能章节**

添加帧合并和 group chat 缓存的描述。

- [x] **Step 2: Commit (skipped by Codex unless explicitly requested)**

```bash
git add docs/architecture-tui.md
git commit -m "docs: document P1 frame coalescing and group chat render cache"
```
