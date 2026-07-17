# P2: Renderable trait + 焦点树 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 完全替换现有的过程式渲染和扁平 Focus enum,引入 `Renderable` trait + `Column`/`Row` flex 容器 + `FocusManager` 焦点树。所有面板迁移为 Renderable struct,布局由 renderable 树驱动。

**Architecture:** 新建 `render/` 模块定义 Renderable trait 和布局容器。每个 UI 面板从渲染函数迁移为持有 `&state` 的 struct 实现 Renderable。`render_ui` 变为组装 renderable 树。`Focus` enum 替换为 `FocusManager` 驱动的 `PaneId` 焦点树,输入路由由树结构决定。

**Tech Stack:** Rust, ratatui 0.29。

**Spec:** `docs/superpowers/specs/2026-06-17-tui-three-phase-refactor-design.md` §5

**Test command:** `cargo test -p minos-tui`
**Build command:** `cargo build -p minos-tui`

**Prerequisite:** P0 + P1 已完成。

## Implementation Status

Completed on 2026-06-17.

- `render/` now defines a Frame-backed `Renderable` trait plus `Column`/`Row` containers. The final trait uses `fn render(&mut self, frame: &mut Frame, area: Rect)` instead of the original Buffer-only sketch because current Ratatui panels update list state, input metrics, scroll bounds, and render caches during draw.
- `Renderable::cursor_pos()` is wired through `Row`/`Column`, and `render_ui()` sets the terminal cursor from the active input renderable after drawing.
- `Focus` enum was removed. `UiState.focus` is now `FocusManager`, with overview/detail focus trees and `PaneId` routing across app/update/event code.
- `Tab` and `BackTab` map to forward/reverse focus cycling through the focus tree.
- Status bar, room list, agent list, group chat, agent chat, input bars, and delete confirm all have renderable adapters. Superseded on 2026-06-23: the modal agent picker renderable was removed; agent selection now uses input `@agent` routing. `ui::render_ui` assembles `Column::with_fill` and `Row` render trees and derives `PanelAreas`/`InputLayoutMetrics` from the same layout ratios.
- Unused `render/primitives.rs` was removed after the render tree converged on direct `Row`/`Column` composition.
- Oversized TUI files were split: `ui/chat/cache.rs`, `ui/input_bar/render.rs`, `ui/chat_tests.rs`, `ui/input_bar_tests.rs`, and `app_tests/` child modules keep all Rust files below the P2 size checkpoint.
- Commit steps in this plan were intentionally skipped; the user requested workspace implementation, not git commits.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/render/mod.rs` | `Renderable` trait, `Column`, `Row` |
| `src/focus.rs` | `FocusManager`, `FocusNode`, `PaneId` |
| `src/ui/mod.rs` | `UiState` 用 `FocusManager` 替换 `Focus`, `render_ui` 组装 renderable 树 |
| `src/ui/*.rs` | 每个面板迁移为 Renderable struct |

---

## Task 1: 创建 render/ 模块 — Renderable trait + Column/Row

**Files:**
- Create: `src/render/mod.rs`
- Create: `src/render/primitives.rs`
- Modify: `src/main.rs` — 添加 `mod render;`

- [x] **Step 1: 创建 src/render/mod.rs**

```rust
//! Renderable trait 与 flex 布局容器。

use ratatui::{buffer::Buffer, layout::Rect};

/// 可渲染单元。所有 UI 面板实现此 trait。
pub trait Renderable {
    /// 渲染到指定区域。
    fn render(&self, area: Rect, buf: &mut Buffer);

    /// 在指定宽度下的期望高度。用于 flex 布局协商。
    fn desired_height(&self, width: u16) -> u16;

    /// 光标位置 (仅输入面板返回 Some)。
    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        None
    }
}

/// 纵向 flex 容器, 子节点从上到下排列。
pub struct Column {
    children: Vec<Box<dyn Renderable>>,
}

impl Column {
    pub fn new(children: Vec<Box<dyn Renderable>>) -> Self {
        Self { children }
    }
}

impl Renderable for Column {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let heights = self.layout_heights(area);
        let mut y = area.y;
        for (child, &h) in self.children.iter().zip(heights.iter()) {
            let child_area = Rect {
                x: area.x,
                y,
                width: area.width,
                height: h,
            };
            child.render(child_area, buf);
            y = y.saturating_add(h);
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.children
            .iter()
            .map(|c| c.desired_height(width))
            .fold(0u16, |acc, h| acc.saturating_add(h))
    }
}

impl Column {
    /// 按子节点 desired_height 分配高度。
    fn layout_heights(&self, area: Rect) -> Vec<u16> {
        let available = area.height;
        let desired: Vec<u16> = self
            .children
            .iter()
            .map(|c| c.desired_height(area.width))
            .collect();
        let total_desired: u16 = desired.iter().copied().fold(0u16, |a, b| a.saturating_add(b));

        if total_desired <= available {
            // 全部满足
            desired
        } else {
            // 按比例缩减
            let scale = f64::from(available) / f64::from(total_desired.max(1));
            desired
                .iter()
                .map(|&h| (f64::from(h) * scale).round() as u16)
                .collect()
        }
    }
}

/// 横向固定比例容器, 子节点从左到右排列。
pub struct Row {
    children: Vec<Box<dyn Renderable>>,
    ratios: Vec<u16>,
}

impl Row {
    /// ratios 长度必须等于 children 长度。
    pub fn new(children: Vec<Box<dyn Renderable>>, ratios: Vec<u16>) -> Self {
        assert_eq!(children.len(), ratios.len(), "Row: children and ratios must have same length");
        Self { children, ratios }
    }
}

impl Renderable for Row {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let widths = self.layout_widths(area);
        let mut x = area.x;
        for (child, &w) in self.children.iter().zip(widths.iter()) {
            let child_area = Rect {
                x,
                y: area.y,
                width: w,
                height: area.height,
            };
            child.render(child_area, buf);
            x = x.saturating_add(w);
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        // Row 的高度 = 最高子节点的高度
        let widths = self.layout_widths_for_width(width);
        self.children
            .iter()
            .zip(widths.iter())
            .map(|(c, &w)| c.desired_height(w))
            .max()
            .unwrap_or(0)
    }
}

impl Row {
    fn layout_widths(&self, area: Rect) -> Vec<u16> {
        self.layout_widths_for_width(area.width)
    }

    fn layout_widths_for_width(&self, total_width: u16) -> Vec<u16> {
        let ratio_sum: u16 = self.ratios.iter().sum();
        if ratio_sum == 0 {
            return vec![0; self.children.len()];
        }
        let mut widths = Vec::with_capacity(self.children.len());
        let mut allocated = 0u16;
        for (i, &ratio) in self.ratios.iter().enumerate() {
            if i == self.children.len() - 1 {
                // 最后一个取剩余
                widths.push(total_width.saturating_sub(allocated));
            } else {
                let w = u16::try_from(
                    (u32::from(total_width) * u32::from(ratio) / u32::from(ratio_sum))
                ).unwrap_or(0);
                widths.push(w);
                allocated = allocated.saturating_add(w);
            }
        }
        widths
    }
}
```

- [x] **Step 2: 创建 src/render/primitives.rs**

```rust
//! 布局原语。

use ratatui::layout::Rect;

/// 内边距。
#[derive(Clone, Copy, Debug, Default)]
pub struct Insets {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

impl Insets {
    pub fn uniform(v: u16) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }

    pub fn apply(&self, area: Rect) -> Rect {
        Rect {
            x: area.x + self.left,
            y: area.y + self.top,
            width: area.width.saturating_sub(self.left + self.right),
            height: area.height.saturating_sub(self.top + self.bottom),
        }
    }
}

/// 给 Renderable 加内边距。
pub struct InsetRenderable {
    inner: Box<dyn super::Renderable>,
    insets: Insets,
}

impl InsetRenderable {
    pub fn new(inner: Box<dyn super::Renderable>, insets: Insets) -> Self {
        Self { inner, insets }
    }
}

impl super::Renderable for InsetRenderable {
    fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let inner_area = self.insets.apply(area);
        self.inner.render(inner_area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let inner_width = width.saturating_sub(self.insets.left + self.insets.right);
        self.inner.desired_height(inner_width)
            .saturating_add(self.insets.top + self.insets.bottom)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let inner_area = self.insets.apply(area);
        self.inner.cursor_pos(inner_area)
            .map(|(x, y)| (x + self.insets.left, y + self.insets.top))
    }
}
```

- [x] **Step 3: 在 render/mod.rs 中声明子模块**

```rust
pub mod primitives;

// 上面的 Renderable/Column/Row 定义
```

- [x] **Step 4: 添加 mod render; 到 main.rs**

- [x] **Step 5: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -10`
Expected: BUILD SUCCEEDED

- [x] **Step 6: Commit**

```bash
git add crates/minos-tui/src/render/ crates/minos-tui/src/main.rs
git commit -m "feat(tui): add Renderable trait with Column/Row flex containers"
```

---

## Task 2: 创建 FocusManager 焦点树

**Files:**
- Create: `src/focus.rs`
- Modify: `src/main.rs` — 添加 `mod focus;`

- [x] **Step 1: 创建 src/focus.rs**

```rust
//! 焦点树管理器, 替换扁平 Focus enum。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneId {
    RoomList,
    GroupChat,
    AgentList,
    AgentChat,
    RoomInput,
    AgentInput,
}

#[derive(Clone, Debug)]
pub enum FocusNode {
    Pane(PaneId),
    Group(Vec<FocusNode>),
}

pub struct FocusManager {
    tree: FocusNode,
    /// 当前焦点在树中的路径 (Group 索引序列)。
    path: Vec<usize>,
}

impl FocusManager {
    pub fn new(detail: bool) -> Self {
        Self {
            tree: default_focus_tree(detail),
            path: vec![0],
        }
    }

    /// 当前聚焦的 PaneId。
    pub fn current(&self) -> PaneId {
        self.node_at_path(&self.path).map(|n| match n {
            FocusNode::Pane(id) => *id,
            FocusNode::Group(_) => panic!("path points to a group, not a pane"),
        }).expect("focus path must point to a valid pane")
    }

    pub fn is(&self, pane: PaneId) -> bool {
        self.current() == pane
    }

    /// 深度优先循环到下一个 pane。
    pub fn cycle_next(&mut self) -> PaneId {
        let order = self.flatten_panes();
        let current = self.current();
        let idx = order.iter().position(|&p| p == current).unwrap_or(0);
        let next = order[(idx + 1) % order.len()];
        self.focus(next);
        next
    }

    /// 深度优先循环到上一个 pane。
    pub fn cycle_prev(&mut self) -> PaneId {
        let order = self.flatten_panes();
        let current = self.current();
        let idx = order.iter().position(|&p| p == current).unwrap_or(0);
        let prev = order[if idx == 0 { order.len() - 1 } else { idx - 1 }];
        self.focus(prev);
        prev
    }

    /// 聚焦到指定 pane。
    pub fn focus(&mut self, pane: PaneId) {
        if let Some(path) = find_pane_path(&self.tree, pane) {
            self.path = path;
        }
    }

    /// 布局切换时重建树。
    pub fn switch_layout(&mut self, detail: bool) {
        let current = self.current();
        self.tree = default_focus_tree(detail);
        self.focus(current);
    }

    fn flatten_panes(&self) -> Vec<PaneId> {
        let mut panes = Vec::new();
        collect_panes(&self.tree, &mut panes);
        panes
    }

    fn node_at_path<'a>(&'a self, path: &[usize]) -> Option<&'a FocusNode> {
        let mut node = &self.tree;
        for &idx in path {
            match node {
                FocusNode::Group(children) => {
                    node = children.get(idx)?;
                }
                FocusNode::Pane(_) => return None,
            }
        }
        Some(node)
    }
}

fn default_focus_tree(detail: bool) -> FocusNode {
    if detail {
        FocusNode::Group(vec![
            FocusNode::Pane(PaneId::GroupChat),
            FocusNode::Group(vec![
                FocusNode::Pane(PaneId::AgentList),
                FocusNode::Pane(PaneId::AgentChat),
            ]),
            FocusNode::Group(vec![
                FocusNode::Pane(PaneId::RoomInput),
                FocusNode::Pane(PaneId::AgentInput),
            ]),
        ])
    } else {
        FocusNode::Group(vec![
            FocusNode::Pane(PaneId::RoomList),
            FocusNode::Pane(PaneId::GroupChat),
            FocusNode::Pane(PaneId::AgentList),
            FocusNode::Pane(PaneId::RoomInput),
        ])
    }
}

fn collect_panes(node: &FocusNode, out: &mut Vec<PaneId>) {
    match node {
        FocusNode::Pane(id) => out.push(*id),
        FocusNode::Group(children) => {
            for child in children {
                collect_panes(child, out);
            }
        }
    }
}

fn find_pane_path(node: &FocusNode, target: PaneId) -> Option<Vec<usize>> {
    match node {
        FocusNode::Pane(id) if *id == target => Some(vec![]),
        FocusNode::Pane(_) => None,
        FocusNode::Group(children) => {
            for (idx, child) in children.iter().enumerate() {
                if let Some(mut sub) = find_pane_path(child, target) {
                    sub.insert(0, idx);
                    return Some(sub);
                }
            }
            None
        }
    }
}

#[cfg(test)]
#[path = "focus_tests.rs"]
mod tests;
```

- [x] **Step 2: 创建 src/focus_tests.rs**

```rust
use super::*;

#[test]
fn detail_tree_cycles_all_panes() {
    let mut fm = FocusManager::new(true);
    let order: Vec<PaneId> = (0..6).map(|_| fm.cycle_next()).collect();
    assert_eq!(order, vec![
        PaneId::GroupChat,
        PaneId::AgentList,
        PaneId::AgentChat,
        PaneId::RoomInput,
        PaneId::AgentInput,
        PaneId::GroupChat, // 循环回来
    ]);
}

#[test]
fn overview_tree_cycles_all_panes() {
    let mut fm = FocusManager::new(false);
    let order: Vec<PaneId> = (0..4).map(|_| fm.cycle_next()).collect();
    assert_eq!(order, vec![
        PaneId::RoomList,
        PaneId::GroupChat,
        PaneId::AgentList,
        PaneId::RoomInput,
    ]);
}

#[test]
fn focus_specific_pane() {
    let mut fm = FocusManager::new(true);
    fm.focus(PaneId::AgentChat);
    assert_eq!(fm.current(), PaneId::AgentChat);
}

#[test]
fn switch_layout_preserves_focus() {
    let mut fm = FocusManager::new(true);
    fm.focus(PaneId::AgentChat);
    fm.switch_layout(false);
    // AgentChat 不在 overview 树中, 应回到第一个 pane
    assert_eq!(fm.current(), PaneId::RoomList);
}
```

- [x] **Step 3: 添加 mod focus; 到 main.rs**

- [x] **Step 4: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -10`
Expected: BUILD SUCCEEDED

- [x] **Step 5: 运行测试**

Run: `cargo test -p minos-tui -- focus 2>&1 | tail -10`
Expected: 4 个 focus 测试通过

- [x] **Step 6: Commit**

```bash
git add crates/minos-tui/src/focus.rs crates/minos-tui/src/focus_tests.rs crates/minos-tui/src/main.rs
git commit -m "feat(tui): add FocusManager with tree-based focus navigation"
```

---

## Task 3: UiState 用 FocusManager 替换 Focus enum

**Files:**
- Modify: `src/ui/mod.rs` — Focus enum 替换为 FocusManager
- Modify: `src/update/*.rs` — cycle_focus 改为 FocusManager::cycle_next
- Modify: `src/app.rs` — 所有 `ui.focus == Focus::X` 改为 `ui.focus.is(PaneId::X)`

- [x] **Step 1: 替换 UiState.focus 类型**

```rust
// ui/mod.rs
// 删除 Focus enum
// 添加:
use crate::focus::{FocusManager, PaneId};

pub struct UiState {
    // ...
    pub focus: FocusManager,  // 替换原来的 focus: Focus
    // ...
}
```

UiState::new 中:
```rust
focus: FocusManager::new(false), // 初始为 overview 模式
```

- [x] **Step 2: 全局替换 Focus 引用**

Run: `rg "\bFocus::" crates/minos-tui/src/ -l`
列出所有引用 Focus enum 的文件。

每个文件中:
- `Focus::RoomList` → `PaneId::RoomList`
- `ui.focus == Focus::RoomList` → `ui.focus.is(PaneId::RoomList)`
- `ui.focus = Focus::RoomList` → `ui.focus.focus(PaneId::RoomList)`

- [x] **Step 3: 更新 agent_detail_visible 切换**

原来设置 `agent_detail_visible = true/false` 的地方,添加:
```rust
ui.focus.switch_layout(detail_visible);
```

- [x] **Step 4: 编译验证 + 修复**

Run: `cargo build -p minos-tui 2>&1 | tail -30`
Expected: 可能有编译错误(Focus enum 被删但还有引用)。按编译器提示逐一修复。

- [x] **Step 5: 运行测试**

Run: `cargo test -p minos-tui 2>&1 | tail -20`
Expected: 全部通过

- [x] **Step 6: Commit**

```bash
git add -A crates/minos-tui/src/
git commit -m "refactor(tui): replace Focus enum with FocusManager tree"
```

---

## Task 4–9: 逐个面板迁移为 Renderable

每个 task 把一个 UI 面板从渲染函数迁移为 Renderable struct。迁移后该面板由 renderable 树驱动而非 render_ui 硬编码。

### 通用迁移模式

每个面板:
1. 创建 `XxxRenderable<'a>` struct 持有 `&'a XxxState` 和 `focused: bool`
2. 实现 `Renderable` trait (`render`, `desired_height`, 可选 `cursor_pos`)
3. 把现有 `render_xxx()` 函数体搬入 `Renderable::render()`
4. 在 `build_render_tree()` 中用 `XxxRenderable::new(...)` 替换旧调用
5. 编译 + 测试 + Commit

### Task 4: StatusBarRenderable

- [x] **Step 1: 创建 StatusBarRenderable**
- [x] **Step 2: 实现 Renderable**
- [x] **Step 3: Commit**

### Task 5: RoomListRenderable + AgentListRenderable (ThreadList)

- [x] **Step 1: 创建两个 Renderable**
- [x] **Step 2: 实现 Renderable**
- [x] **Step 3: Commit**

### Task 6: GroupChatRenderable

- [x] **Step 1: 创建 GroupChatRenderable (包装现有 render_group_chat + cache)**
- [x] **Step 2: 实现 Renderable**
- [x] **Step 3: Commit**

### Task 7: AgentChatRenderable

- [x] **Step 1: 创建 AgentChatRenderable (包装现有 render_chat)**
- [x] **Step 2: 实现 Renderable**
- [x] **Step 3: Commit**

### Task 8: InputBarRenderable

- [x] **Step 1: 创建 InputBarRenderable (实现 cursor_pos)**
- [x] **Step 2: 实现 Renderable (desired_height = required_height)**
- [x] **Step 3: Commit**

### Task 9: DeleteConfirmRenderable (overlay)

Superseded on 2026-06-23: `AgentPickerRenderable` was removed with the modal agent picker.

- [x] **Step 1: 创建两个 overlay Renderable**
- [x] **Step 2: 实现 Renderable (render 在最上层)**
- [x] **Step 3: Commit**

---

## Task 10: render_ui 组装 renderable 树 + 删除旧函数

**Files:**
- Modify: `src/ui/mod.rs` — render_ui 改为 build_render_tree + render

- [x] **Step 1: 实现 build_render_tree**

```rust
use crate::render::{Renderable, Column, Row};
use crate::focus::PaneId;

pub fn build_render_tree<'a>(state: &'a AppState, ui: &'a UiState) -> Box<dyn Renderable + 'a> {
    let detail = ui.agent_detail_visible;

    let main_row: Box<dyn Renderable> = if detail {
        Box::new(Row::new(vec![
            Box::new(GroupChatRenderable::new(&ui.group_chat, ui.focus.is(PaneId::GroupChat))),
            Box::new(AgentListRenderable::new(&ui.threads, &ui.agent_list_state, ui.focus.is(PaneId::AgentList))),
            Box::new(AgentChatRenderable::new(/* active chat state */, ui.focus.is(PaneId::AgentChat))),
        ], vec![45, 20, 35]))
    } else {
        Box::new(Row::new(vec![
            Box::new(RoomListRenderable::new(&ui.rooms, &ui.room_list_state, ui.focus.is(PaneId::RoomList))),
            Box::new(GroupChatRenderable::new(&ui.group_chat, ui.focus.is(PaneId::GroupChat))),
            Box::new(AgentListRenderable::new(&ui.threads, &ui.agent_list_state, ui.focus.is(PaneId::AgentList))),
        ], vec![20, 55, 25]))
    };

    let input_row: Box<dyn Renderable> = if detail {
        Box::new(Row::new(vec![
            Box::new(InputBarRenderable::new(&ui.room_input, InputTarget::Room, ui.focus.is(PaneId::RoomInput))),
            Box::new(InputBarRenderable::new(&ui.agent_input, InputTarget::Agent, ui.focus.is(PaneId::AgentInput))),
        ], vec![65, 35]))
    } else {
        Box::new(InputBarRenderable::new(&ui.room_input, InputTarget::Room, ui.focus.is(PaneId::RoomInput)))
    };

    Box::new(Column::new(vec![
        Box::new(StatusBarRenderable::new(&ui.status)),
        main_row,
        input_row,
    ]))
}
```

- [x] **Step 2: 重写 render_ui**

```rust
pub fn render_ui(f: &mut Frame, state: &AppState, ui: &UiState) {
    let area = f.area();
    let tree = build_render_tree(state, ui);
    let buf = f.buffer_mut();
    tree.render(area, buf);

    // Overlay 渲染在最上层
    if let Some(confirm) = &ui.delete_confirm {
        DeleteConfirmRenderable::new(confirm).render(area, buf);
    }

    // 设置光标
    if let Some((x, y)) = cursor_pos_for_focus(ui) {
        f.set_cursor_position(x, y);
    }
}
```

- [x] **Step 3: 删除旧的 render_* 函数**

所有被 Renderable 替换的旧 `render_xxx(f, area, ...)` 函数删除。

- [x] **Step 4: 更新 main.rs 的 draw 调用**

```rust
// Before:
terminal.draw(|f| { ui::render_ui(f, app.ui()); })?;
// After:
terminal.draw(|f| { ui::render_ui(f, &app.state, &app.ui); })?;
```

注意: `app.ui()` 当前返回 `&mut UiState`。renderable 树需要 `&UiState`(不可变)。需要调整 — 要么 render 时借用不可变引用,要么 renderable 接收 `&mut`。建议: render 前先 clone 需要的可变状态,或在 render_ui 内部获取不可变引用。

**方案:** 把 `GroupChatState.render_cache` 的 rebuild 移到 update 层(pre-render),render 时只需要不可变引用。

- [x] **Step 5: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -30`
Expected: BUILD SUCCEEDED

- [x] **Step 6: 运行测试**

Run: `cargo test -p minos-tui 2>&1 | tail -20`
Expected: 全部通过

- [x] **Step 7: clippy**

Run: `cargo clippy -p minos-tui --all-targets -- -D warnings 2>&1 | tail -20`
Expected: 无 warnings

- [x] **Step 8: Commit**

```bash
git add -A crates/minos-tui/src/
git commit -m "refactor(tui): complete P2 — renderable tree drives all rendering"
```

---

## Task 11: 最终验证 + 文档

- [x] **Step 1: 验证所有成功标准**

Run: `wc -l crates/minos-tui/src/app.rs`
Expected: < 400 行

Run: `find crates/minos-tui/src -name '*.rs' -exec wc -l {} + | sort -rn | head -20`
Expected: 无文件超过 800 行。如有,拆分为子模块。

Run: `cargo test -p minos-tui`
Expected: 全部通过

Run: `cargo clippy -p minos-tui --all-targets -- -D warnings`
Expected: 无 warnings

- [x] **Step 2: 重写 architecture-tui.md**

描述新的三阶段架构:
- P0: Action/Effect/Update 四层分离
- P1: FrameRequester + group chat cache
- P2: Renderable trait + FocusManager 焦点树

更新文件清单和所有章节。

- [x] **Step 3: Commit**

```bash
git add docs/architecture-tui.md
git commit -m "docs: rewrite architecture-tui.md for completed three-phase refactor"
```
