# P0-A: translation.rs 拆分 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 2184 行的 `translation.rs` 拆分为 `translation/` 目录下的聚焦模块,不改变任何行为,不改变公开 API。

**Architecture:** 纯文件搬迁 + `mod` 重导出。原 `translation.rs` 删除,新建 `translation/mod.rs` 作为门面,内部子模块按职责拆分。所有 `use crate::translation::*` 的外部引用路径不变。

**Tech Stack:** Rust, edition 2021。

**Spec:** `docs/superpowers/specs/2026-06-17-tui-three-phase-refactor-design.md` §3.5

**Test command:** `cargo test -p minos-tui`
**Build command:** `cargo build -p minos-tui`

---

## File Structure

| File | Responsibility | Source (from translation.rs) |
|---|---|---|
| `src/translation/mod.rs` | 门面: 重导出所有 pub 类型 + 测试模块挂载 | new |
| `src/translation/agent.rs` | `AgentTranslationState` enum + impl + `translate_with_log` | lines 9–61 |
| `src/translation/chat_state.rs` | `ChatState` struct + impl | lines 63–77, 117–607 |
| `src/translation/chat_item.rs` | `ChatItem` enum + impl + `TextPart` | lines 80–115, 609–629, 1434–1442 |
| `src/translation/event_projection.rs` | `apply_ui_event`, `apply_raw_request_event`, `push_*`, `find_*_mut`, role helpers | lines 273–606 (ChatState 的私有方法提取为 free functions 或保留为 impl) |
| `src/translation/tool_summary.rs` | 工具参数/输出格式化 | lines 680–943 |
| `src/translation/pending_request.rs` | `PendingAgentRequest`, `PendingAgentRequestKind`, `PendingQuestionSpec`, `PendingQuestionOption` + 格式化 | lines 945–1330 |
| `src/translation/json_helpers.rs` | JSON 递归查找辅助 | lines 1345–1406 |
| `src/translation/selection.rs` | `ChatSelection`, `ChatSelectionPoint` | lines 1408–1432 |
| `src/translation/translation_tests.rs` | 测试 (25 个) | lines 1444–2184 |

**关键决策:** `ChatState` 的私有方法(`apply_ui_event` 等)保留为 `impl ChatState` 的方法,放在 `chat_state.rs`。不提取为 free functions — 那会破坏 `&mut self` 的访问。`event_projection.rs` 这个文件 **不创建**,相关逻辑留在 `chat_state.rs`。

**最终文件列表:**
- `src/translation/mod.rs`
- `src/translation/agent.rs`
- `src/translation/chat_state.rs`
- `src/translation/chat_item.rs`
- `src/translation/tool_summary.rs`
- `src/translation/pending_request.rs`
- `src/translation/json_helpers.rs`
- `src/translation/selection.rs`
- `src/translation/translation_tests.rs`

---

## Task 1: 创建 translation/ 目录骨架并迁移 agent.rs

**Files:**
- Create: `src/translation/mod.rs`
- Create: `src/translation/agent.rs`
- Delete: `src/translation.rs` (在最后一步删除,本任务先创建新文件)

- [ ] **Step 1: 创建 `src/translation/mod.rs`**

创建目录 `src/translation/`,创建 `mod.rs`:

```rust
//! 聊天状态管理与协议翻译。

mod agent;
mod chat_item;
mod chat_state;
mod json_helpers;
mod pending_request;
mod selection;
mod tool_summary;

pub use agent::AgentTranslationState;
pub use chat_item::{ChatItem, TextPart};
pub use chat_state::ChatState;
pub use pending_request::{
    PendingAgentRequest, PendingAgentRequestKind, PendingQuestionOption, PendingQuestionSpec,
};
pub use selection::{ChatSelection, ChatSelectionPoint};

#[cfg(test)]
#[path = "translation_tests.rs"]
mod tests;
```

- [ ] **Step 2: 创建 `src/translation/agent.rs`**

从 `translation.rs` 的 lines 1–61 提取(包含 imports 和 `AgentTranslationState`):

```rust
use minos_domain::AgentName;
use minos_protocol::UiEventMessage;

pub enum AgentTranslationState {
    // ... (原样搬移 lines 9–14 的 enum 定义)
}

impl AgentTranslationState {
    // ... (原样搬移 lines 16–42 的 impl)
}

fn translate_with_log<F>(agent: &str, payload: &serde_json::Value, f: F) -> Vec<UiEventMessage>
where
    F: FnOnce(&serde_json::Value) -> Result<Vec<UiEventMessage>, anyhow::Error>,
{
    // ... (原样搬移 lines 44–61)
}
```

注意: `translate_with_log` 和 `AgentTranslationState` 的可见性。当前 `translate_with_log` 是 private free function,被 `ChatState` 调用。搬移后需要 `pub(super) fn` 以便 `chat_state.rs` 能调用。

- [ ] **Step 3: 为剩余模块创建占位文件**

创建空文件(后续 task 填充):
- `src/translation/chat_state.rs` — 空文件 + `// placeholder`
- `src/translation/chat_item.rs` — 空文件
- `src/translation/tool_summary.rs` — 空文件
- `src/translation/pending_request.rs` — 空文件
- `src/translation/json_helpers.rs` — 空文件
- `src/translation/selection.rs` — 空文件

- [ ] **Step 4: 编译验证**

此时 `src/translation.rs` (旧文件) 和 `src/translation/` (新目录) 同时存在,Rust 会报冲突。先注释掉 `main.rs` line 20 的 `mod translation;`,改为:

```rust
// mod translation; // 暂时禁用, P0-A 重构中
```

Run: `cargo check -p minos-tui 2>&1 | head -20`
Expected: 编译错误来自 translation 引用,这是预期的。下一步填充内容后恢复。

- [ ] **Step 5: Commit**

```bash
git add crates/minos-tui/src/translation/
git commit -m "refactor(tui): create translation/ directory skeleton with agent.rs"
```

---

## Task 2: 迁移 chat_item.rs (ChatItem + TextPart)

**Files:**
- Fill: `src/translation/chat_item.rs`
- Source: `src/translation.rs` lines 80–115 (ChatItem enum), 609–629 (ChatItem impl), 1434–1442 (TextPart)

- [ ] **Step 1: 搬移 ChatItem enum + impl + TextPart 到 chat_item.rs**

```rust
use minos_domain::AgentName;

#[derive(Debug, PartialEq, Eq)]
pub enum ChatItem {
    // ... (原样搬移 lines 80–115)
}

impl ChatItem {
    // ... (原样搬移 lines 609–629 的私有方法 message_id / set_streaming)
    // 这些方法当前是 private, 但 chat_state.rs 需要调用它们
    // 改为 pub(crate) 或 pub(super)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextPart {
    // ... (原样搬移 lines 1434–1442)
}
```

可见性调整: `ChatItem` 的 `message_id()` 和 `set_streaming()` 当前是 `fn`(private)。搬移后 `chat_state.rs` 的 `impl ChatState` 需要调用它们,改为 `pub(super) fn`。

`ChatItem` 的字段(如 `text_parts`, `args`, `output` 等)当前是 `pub`。保持 `pub`。

- [ ] **Step 2: 在 mod.rs 确认重导出**

`mod.rs` 已有 `pub use chat_item::{ChatItem, TextPart};` — 无需改动。

- [ ] **Step 3: 编译验证(仅 chat_item 模块)**

暂不验证完整编译(等所有模块就位后统一验证)。检查语法:
Run: `cargo check -p minos-tui 2>&1 | rg "chat_item" | head -10`
Expected: 无 chat_item 语法错误(可能有未解析的引用,后续 task 修复)。

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/translation/chat_item.rs
git commit -m "refactor(tui): extract ChatItem and TextPart into translation/chat_item.rs"
```

---

## Task 3: 迁移 tool_summary.rs

**Files:**
- Fill: `src/translation/tool_summary.rs`
- Source: `src/translation.rs` lines 680–943

- [ ] **Step 1: 搬移所有工具格式化函数**

以下函数全部是当前 `translation.rs` 的 private free functions,搬移到 `tool_summary.rs` 并改为 `pub(super)`:

| 原行号 | 函数 |
|--------|------|
| 680–690 | `truncate_str` |
| 692–778 | `summarize_tool_args` |
| 780–788 | `compact_tool_args` |
| 790–807 | `summarize_tool_output` |
| 809–818 | `tool_output_detail` |
| 820–831 | `is_diff_like` |
| 833–839 | `parse_tool_args` |
| 841–843 | `summary_piece` |
| 845–847 | `one_line` |
| 849–851 | `find_stringish` |
| 853–890 | `find_stringish_inner` |
| 892–922 | `value_to_summary_text` |
| 924–943 | `array_len_for_keys` |

`truncate_str` 和 `one_line` 也被 `chat_state.rs` 的其他函数使用(如 `append_text_to_item` 不使用,但 `text_parts_to_string` 不使用)。检查: `truncate_str` 被 `tool_summary` 内部和 `pending_request` 使用。保持 `pub(super)`。

`is_diff_like` 被 `tool_summary` 和 `chat_state` 的渲染辅助使用。`pub(super)`。

文件内容:
```rust
//! 工具调用参数与输出的格式化辅助。

pub(super) fn truncate_str(s: &str, max_len: usize) -> String { ... }
pub(super) fn summarize_tool_args(tool_name: &str, args_json: &str) -> String { ... }
// ... 其余函数
```

- [ ] **Step 2: Commit**

```bash
git add crates/minos-tui/src/translation/tool_summary.rs
git commit -m "refactor(tui): extract tool summary helpers into translation/tool_summary.rs"
```

---

## Task 4: 迁移 json_helpers.rs

**Files:**
- Fill: `src/translation/json_helpers.rs`
- Source: `src/translation.rs` lines 1186–1406

- [ ] **Step 1: 搬移 JSON 查找函数**

| 原行号 | 函数 |
|--------|------|
| 1186–1202 | `find_array_by_key` |
| 1345–1347 | `find_string_by_keys` |
| 1349–1355 | `direct_string_by_keys` |
| 1357–1380 | `find_string_by_keys_inner` |
| 1382–1406 | `json_value_summary` |

全部改为 `pub(super)`。`pending_request.rs` 和 `tool_summary.rs` 会调用这些函数。

```rust
//! JSON 递归查找辅助函数。

pub(super) fn find_array_by_key<'a>(...) -> Option<&'a Vec<serde_json::Value>> { ... }
pub(super) fn find_string_by_keys(...) -> Option<String> { ... }
// ... 其余
```

- [ ] **Step 2: Commit**

```bash
git add crates/minos-tui/src/translation/json_helpers.rs
git commit -m "refactor(tui): extract JSON helper functions into translation/json_helpers.rs"
```

---

## Task 5: 迁移 pending_request.rs

**Files:**
- Fill: `src/translation/pending_request.rs`
- Source: `src/translation.rs` lines 945–1330

- [ ] **Step 1: 搬移类型定义 + impl + 格式化函数**

类型定义(pub):
- `PendingAgentRequest` (945–949) — 保持 `pub`
- `PendingAgentRequestKind` (951–970) — 保持 `pub`
- `PendingQuestionSpec` (972–979) — 保持 `pub`
- `PendingQuestionOption` (981–985) — 保持 `pub`

Impl:
- `impl PendingAgentRequest` (987–1079) — `id`, `from_approval_request`, `from_opencode_permission`, `from_opencode_question`

格式化函数(pub(super)):
| 原行号 | 函数 |
|--------|------|
| 1081–1121 | `find_permission_option_response` |
| 1123–1156 | `opencode_permission_id` |
| 1158–1166 | `opencode_permission_is_completed` |
| 1168–1184 | `find_permission_status` |
| 1204–1211 | `format_user_input_prompt` |
| 1213–1229 | `format_approval_prompt` |
| 1231–1294 | `parse_pending_questions` |
| 1296–1330 | `format_pending_question_prompt` |
| 1332–1343 | `opencode_question_reply_id` |

注意: 这些函数调用 `json_helpers.rs` 的函数(`find_string_by_keys` 等),需要 `use super::json_helpers::*;` 或显式路径。

```rust
//! 待处理 agent 请求 (审批/权限/问题) 的类型与格式化。

use super::json_helpers::*;
use minos_domain::AgentName;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAgentRequest { ... }

// ... 其余
```

- [ ] **Step 2: Commit**

```bash
git add crates/minos-tui/src/translation/pending_request.rs
git commit -m "refactor(tui): extract pending request types into translation/pending_request.rs"
```

---

## Task 6: 迁移 selection.rs

**Files:**
- Fill: `src/translation/selection.rs`
- Source: `src/translation.rs` lines 1408–1432

- [ ] **Step 1: 搬移 ChatSelection + ChatSelectionPoint**

```rust
//! 聊天文本选区。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatSelectionPoint {
    pub item_index: usize,
    pub line_index: usize,
    pub char_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSelection {
    pub anchor: ChatSelectionPoint,
    pub focus: ChatSelectionPoint,
}

impl ChatSelection {
    // ... 原样搬移 lines 1420–1432 (normalized, is_empty)
}
```

保持 `pub`(外部 `app.rs` 和 `ui/chat.rs` 使用)。

- [ ] **Step 2: Commit**

```bash
git add crates/minos-tui/src/translation/selection.rs
git commit -m "refactor(tui): extract ChatSelection into translation/selection.rs"
```

---

## Task 7: 迁移 chat_state.rs (ChatState — 最大的模块)

**Files:**
- Fill: `src/translation/chat_state.rs`
- Source: `src/translation.rs` lines 63–77 (struct), 117–607 (impl), 631–678 (free helpers)

这是最复杂的搬迁,因为 `ChatState` 的 impl 方法调用了 `tool_summary`, `pending_request`, `chat_item` 的函数。

- [ ] **Step 1: 搬移 ChatState struct**

```rust
//! 每线程聊天状态与事件投影。

use super::agent::AgentTranslationState;
use super::chat_item::{ChatItem, TextPart};
use super::pending_request::*;
use super::selection::{ChatSelection, ChatSelectionPoint};
use super::tool_summary::*;
use minos_domain::{AgentName, MessageRole};
use minos_protocol::UiEventMessage;
use std::collections::{HashMap, HashSet};

pub struct ChatState {
    // ... 原样搬移 lines 63–77
}
```

- [ ] **Step 2: 搬移 ChatState impl (全部方法)**

搬移 lines 117–607 的所有 impl 方法。包括:
- `new` (118–134)
- `apply_ui_events` (200–204)
- `apply_ui_event` (273–466)
- `apply_raw_request_event` (529–593)
- `push_pending_request_message` (595–606)
- `push_text_item` (468–483)
- `find_text_item_mut` (485–492)
- `find_reasoning_item_mut` (494–498)
- `find_tool_call_item_mut` (500–504)
- `infer_role` (506–518)
- `message_is_assistant` (520–527)
- `finish_all_streaming` (244–251)
- `toggle_tool_expansion` (253–260)
- `last_completed_assistant_text` (206–242)
- scroll 方法 (136–181)
- selection 方法 (183–198)
- pending request 方法 (262–271)

方法签名不变。`apply_ui_event` 内部调用 `summarize_tool_args` 等 — 现在通过 `use super::tool_summary::*;` 引入。

- [ ] **Step 3: 搬移 chat_state 相关 free helpers**

| 原行号 | 函数 | 可见性 |
|--------|------|--------|
| 631–645 | `assistant_text_before_error` | `pub(super)` |
| 647–665 | `text_parts_to_string` | `pub(super)` |
| 667–678 | `append_text_to_item` | `pub(super)` |

这些被 `ChatState` impl 方法调用,放在同文件。

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/translation/chat_state.rs
git commit -m "refactor(tui): extract ChatState into translation/chat_state.rs"
```

---

## Task 8: 迁移测试模块 + 删除旧 translation.rs + 恢复 mod 声明

**Files:**
- Create: `src/translation/translation_tests.rs`
- Delete: `src/translation.rs`
- Modify: `src/main.rs:20` — 恢复 `mod translation;`

- [ ] **Step 1: 搬移测试到 translation_tests.rs**

从 `translation.rs` lines 1444–2184 搬移全部 25 个测试。改 `use super::*;` 为:

```rust
use super::*;
use super::chat_state::ChatState;
use super::chat_item::{ChatItem, TextPart};
// 测试可能需要直接引用子模块的 private 项
// 如果测试用到了 pub(super) 函数,需要在 mod.rs 的 #[cfg(test)] 块里 use 它们
```

实际上 `use super::*;` 在 `#[cfg(test)] mod tests` 里会引用 `mod.rs` 的 `pub use` 重导出,所以大多数测试代码不需要改动。

检查: 某些测试可能调用了 `pub(super)` 的函数(如 `summarize_tool_args`)。如果是,需要在测试模块顶部添加:
```rust
use super::tool_summary::*;
use super::pending_request::*;
```

- [ ] **Step 2: 删除旧 `src/translation.rs`**

```bash
rm crates/minos-tui/src/translation.rs
```

- [ ] **Step 3: 恢复 `src/main.rs` 的 mod 声明**

`src/main.rs` line 20 (当前被注释为 `// mod translation;`):
```rust
mod translation;
```

Rust 会自动找到 `translation/mod.rs`。

- [ ] **Step 4: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -30`
Expected: BUILD SUCCEEDED

如果失败,常见问题:
- 可见性: 某些 `fn` 需要改为 `pub(super)` — 按编译器提示修复
- 缺少 `use`: 在子模块顶部添加 `use super::*;` 或显式路径

- [ ] **Step 5: 运行测试**

Run: `cargo test -p minos-tui 2>&1 | tail -20`
Expected: 所有测试通过(25 个 translation 测试 + 其他模块测试)

- [ ] **Step 6: 运行 clippy**

Run: `cargo clippy -p minos-tui --all-targets -- -D warnings 2>&1 | tail -20`
Expected: 无 warnings

- [ ] **Step 7: Commit**

```bash
git add -A crates/minos-tui/src/
git commit -m "refactor(tui): complete translation/ split, delete old translation.rs

- translation.rs (2184 lines) split into 7 focused modules
- All 25 tests migrated to translation_tests.rs
- No behavior changes, all public API paths preserved
"
```

---

## Task 9: 更新 architecture-tui.md

**Files:**
- Modify: `docs/architecture-tui.md` §文件清单

- [ ] **Step 1: 更新文件清单**

在 `docs/architecture-tui.md` 的"文件清单"表格中,替换 `translation.rs` 行为:

```markdown
| `translation/mod.rs` | 门面, 重导出 pub 类型 | ~30 |
| `translation/agent.rs` | AgentTranslationState | ~50 |
| `translation/chat_state.rs` | ChatState + 事件投影 | ~600 |
| `translation/chat_item.rs` | ChatItem + TextPart | ~80 |
| `translation/tool_summary.rs` | 工具参数/输出格式化 | ~250 |
| `translation/pending_request.rs` | 待处理请求类型 + 格式化 | ~400 |
| `translation/json_helpers.rs` | JSON 递归查找 | ~70 |
| `translation/selection.rs` | ChatSelection | ~30 |
| `translation/translation_tests.rs` | 测试 | ~740 |
```

同时更新 §翻译管线 的描述,添加子模块说明。

- [ ] **Step 2: Commit**

```bash
git add docs/architecture-tui.md
git commit -m "docs: update architecture-tui.md for translation/ split"
```
