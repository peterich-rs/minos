# Grok / Opencode / Codex 三方数据格式对比

> **双重验证**：① 真实数据库采样；② 源码权威类型定义。
> - Grok 源码：`/Users/fannnzhang/code/github.com/grok-build`（Rust，ACP upstream v0.11.4 + xAI 扩展）
> - Opencode 源码：`/Users/fannnzhang/code/github.com/opencode`（TypeScript + Effect Schema，V1 wire 协议）
> - Codex 源码：`crates/minos-codex-protocol/src/generated/types.rs`（v2 codegen）
>
> 本文是 `schema-grok-acp.md` 和 `schema-opencode.md` 的对比索引。
> Codex App-Server v2 作为参考标准，Grok ACP 和 Opencode 作为被对标方。

---

## 1. 架构范式对比

| 维度 | Codex v2 | Grok ACP | Opencode |
|------|---------|---------|---------|
| **源码语言** | TypeScript (JSON Schema → Rust codegen) | Rust（ACP upstream + xAI 扩展两层） | TypeScript + Effect Schema |
| **事件模型** | Item lifecycle (started→completed + delta) | session/update notification (chunk 流) | Event-sourcing (message.part.updated + delta) |
| **内容层级** | Thread → Turn → ThreadItem | Session → Prompt → Update | Session → Message → Part |
| **寻址** | `threadId + turnId + itemId` 三元组 | `sessionId + streamStartMs + toolCallId` | `sessionID + messageID + partID` 三元组 |
| **流式 vs 快照** | delta 通知 + completed 快照（两种方法） | 只有 chunk 流，无显式完成 | delta + updated 双路径（需去重） |
| **工具模型** | ThreadItem tagged union (16 variants) | tool_call + tool_call_update (rawInput/rawOutput 双通道，ACP 标准) | tool part + state machine (pending→running→completed/error) |
| **工具 kind 枚举** | 内嵌于 variant type | ACP 10 值 + xAI 扩展 32 值（自动降级映射） | 无 kind 概念（用 tool name 字符串） |
| **审批** | server→client request (需 response) | notification (单向，PendingKind 3 种：permission/question/plan_approval) | permission.asked/replied（或 SDK 名 permission.updated） |
| **元数据** | 结构化字段（在 Params 里） | `_meta` 扩展对象（eventId/promptId/streamStartMs 等 xAI 私有） | 嵌入在 info/part 内（Effect Schema 结构化） |
| **持久化** | Minos daemon.sqlite (events.body_inline) | Minos daemon.sqlite (events.body_inline) | opencode.db (event/message/part 表，ephemeral delta 不持久化) |
| **事件系统** | JSON-RPC notification | ACP notification + xAI fire-and-forget | V1 wire (SSE) + V2 event-sourcing（内部） |

---

## 2. 事件类型映射表

### 2.1 会话生命周期

| 事件 | Codex | Grok | Opencode |
|------|-------|------|---------|
| 会话创建 | `thread/started` → `{ thread }` | （无显式事件，隐含在第一个 session/update） | `session.created` → `{ sessionID, info }` |
| 会话更新 | （通过 Thread.status） | （无） | `session.updated` → `{ sessionID, info }` |
| 会话标题 | Thread.name | （无） | `session.updated` info.title |
| 会话关闭 | （连接关闭） | `acp_closed` | `session.idle` |

### 2.2 Turn 生命周期

| 事件 | Codex | Grok | Opencode |
|------|-------|------|---------|
| Turn 开始 | `turn/started` → `{ threadId, turn }` | （隐含：`_meta.turnStartMs` 首次出现） | （隐含：message.created + step-start） |
| Turn 完成 | `turn/completed` → `{ threadId, turn }` | `_x.ai/session_notification` → `turn_completed` | message.updated finish=`stop` + time.completed |

### 2.3 文本内容

| 内容 | Codex | Grok | Opencode |
|------|-------|------|---------|
| 助手文本 delta | `item/agentMessage/delta` `{ delta, itemId, turnId }` | `session/update` → `agent_message_chunk` `{ content: { text, type } }` | `message.part.delta` `{ delta, messageID, partID, field }` |
| 助手文本完成 | `item/completed` → `AgentMessage` `{ id, text }` | （无完成事件，靠 streamStartMs 变化推断） | `message.part.updated` type=`text` + `time.end` |
| 推理文本 delta | `item/reasoning/textDelta` `{ delta, contentIndex }` | `session/update` → `agent_thought_chunk` | `message.part.delta`（part type=reasoning） |
| 推理完成 | `item/completed` → `Reasoning` | （无） | `message.part.updated` type=`reasoning` + `time.end` |
| 用户消息 | `item/started` → `UserMessage` | `session/update` → `user_message_chunk` | `message.updated` role=`user` |

### 2.4 工具调用

| 阶段 | Codex | Grok | Opencode |
|------|-------|------|---------|
| 开始 | `item/started` → `CommandExecution`/`FileChange` | `session/update` → `tool_call` | `message.part.updated` type=`tool` status=`pending`/`running` |
| 流式输出 | `item/commandExecution/outputDelta` | `session/update` → `tool_call_update` (可多次) | （同一 part 多次 updated） |
| 完成 | `item/completed` → `CommandExecution` (status=completed) | `session/update` → `tool_call_update` (status=`Completed`) | `message.part.updated` status=`completed` |
| 错误 | `item/completed` (status=failed) | `tool_call_update` (status=`Failed`) | status=`error` |
| 工具入参 | `ThreadItem.command` / `.arguments` | `update.rawInput` + `_meta.x.ai/tool.input` | `state.input` (per-tool shape) |
| 工具输出 | `ThreadItem.aggregatedOutput` / `.result` | `update.rawOutput` (Content/...) | `state.output` (string, XML-like) |

### 2.5 计划 / Todo

| | Codex | Grok | Opencode |
|---|-------|------|---------|
| 形式 | `ThreadItem` → `Plan` + `item/plan/delta` | `session/update` → `plan` (entries 快照) | `todowrite` tool |
| 数据 | `{ id, text }` 增量 | `[{ content, priority, status }]` 完整快照 | `state.input.todos: [{ id, content, status, priority }]` |

### 2.6 审批

| | Codex | Grok | Opencode |
|---|-------|------|---------|
| 请求 | `commandExecution/requestApproval` (server request, `{ threadId, turnId, itemId }`) | `_x.ai/session_notification` → `pending_interaction` (notification, `{ tool_call_id }`) | `permission.updated` (透传) |
| 解决 | client → `serverRequest/resolved` | `_x.ai/session_notification` → `interaction_resolved` | （无显式） |

### 2.7 Token / Usage

| | Codex | Grok | Opencode |
|---|-------|------|---------|
| 时机 | Turn 完成（`Turn` 结构） | `_x.ai/session_notification` → `turn_completed` | `step-finish` part + message.tokens |
| 字段 | Turn.tokens | `usage: { inputTokens, outputTokens, reasoningTokens, cachedReadTokens, totalTokens, costUsdTicks, apiDurationMs, modelCalls }` | `tokens: { input, output, reasoning, cache: { read, write } }` + `cost` |

---

## 3. 工具入参格式对比

### 3.1 文件读取

| | 格式 | 源码 |
|---|-------|------|
| **Codex** | `CommandExecution.command: "read file.txt"` (natural language) | — |
| **Grok** | `rawInput: { variant: "ReadFile", path, limit, offset }` (tagged union) | `xai-grok-tools/types/tool_io.rs:60` |
| **Opencode** | `state.input: { filePath, offset, limit }` (Effect Schema struct) | `opencode/tool/read.ts:28` |

### 3.2 文件编辑

| | 格式 | 源码 |
|---|-------|------|
| **Codex** | `FileChange` → `changes: FileUpdateChange[]` (结构化 diff) | — |
| **Grok** | `rawInput: { variant: "SearchReplace", file_path, old_string, new_string, replace_all }` + `content: [Diff{ path, old_text, new_text }]` | `xai-grok-tools/types/tool_io.rs` + ACP `tool_call.rs:560` |
| **Opencode** | `state.input: { filePath, oldString, newString, replaceAll }` | `opencode/tool/edit.ts:47` |

### 3.3 终端命令

| | 格式 | 源码 |
|---|-------|------|
| **Codex** | `CommandExecution.command: string, cwd: AbsolutePathBuf` | — |
| **Grok** | `rawInput: { variant: "Bash", command, description, is_background, timeout? }` | `xai-grok-tools/types/tool_io.rs` |
| **Opencode** | `state.input: { command, timeout?, workdir?, description }` | `opencode/tool/shell/prompt.ts:22` |

### 3.4 搜索

| | 格式 | 源码 |
|---|-------|------|
| **Codex** | `CommandExecution.command: "grep ..."` | — |
| **Grok** | `rawInput: { variant: "Grep", pattern, path?, glob?, output_mode? }` | `xai-grok-tools/types/tool_io.rs` |
| **Opencode** | `state.input: { pattern, path?, include? }` | `opencode/tool/grep.ts:10` |

### 3.5 工具入参变体数量

| | 变体数量 | 枚举方式 |
|---|---------|---------|
| **Codex** | 16 ThreadItem variants | tagged union by `type` |
| **Grok** | 35+ ToolInput variants | tagged union by `variant`（`tool_io.rs:60`） |
| **Opencode** | 14+ tool definitions | 无枚举，用 `tool` 字符串名 + Effect Schema parameters |

---

## 4. 消息边界推断策略对比

这是翻译器实现中最关键的差异：

| | Codex | Grok | Opencode |
|---|-------|------|---------|
| **新消息信号** | `item/started` 携带新 itemId | `streamStartMs` 值变化 | `message.updated` 携带新 messageID |
| **完成信号** | `item/completed` | 下一个 tool_call 或 streamStartMs 变化 | `part.time.end` / `message.time.completed` |
| **翻译器状态** | itemId → message 映射 | `open_assistant_message_id` + `last_stream_start_ms` | `emitted_message_ids` + `open_assistant_message_id` |
| **复杂度** | 最低（显式 lifecycle） | 最高（隐式边界推断） | 中等（需跨事件维护 role 映射） |

---

## 5. 翻译器测试覆盖矩阵

### 5.1 共通测试场景（三个 agent 都需覆盖）

| 场景 | Codex | Grok | Opencode |
|------|-------|------|---------|
| 助手单条文本消息 | ✅ golden 01-03 | ✅ inline test | ✅ inline test |
| 助手多 turn 文本 | ✅ golden 10 | 需新增 | 需新增 |
| 推理 + 文本混合 | ✅ golden 12 | 需新增 | 需新增 |
| 工具调用完整生命周期 | ✅ golden 05-06 | ✅ inline test | ✅ inline test |
| 工具调用错误 | ✅ golden 06 | 需新增 | 需新增 |
| 用户消息 | ✅ golden 02 | ✅ inline test | ✅ inline test |

### 5.2 Agent 特有测试场景

**Grok 专属**：
- `streamStartMs` 变化 → 新消息边界（源码：`updates.rs:87-156`，`send_update_full()` 构建 `_meta`）
- `tool_call_update` 在 `tool_call` 之前到达（orphan race）
- `kind` 降级映射：xAI 32 值 → ACP 10 值（`list_dir`→`other`, `plan`→`think`, `web_search`→`other`）
- `rawInput.variant` 覆盖：源码定义 35+ 变体（`tool_io.rs:60`），测试至少覆盖 ReadFile/Grep/SearchReplace/Bash/ListDir/WebFetch/TodoWrite
- `rawOutput.type` 覆盖：源码定义 25+ 变体（`output.rs:624`），测试至少覆盖 Bash/ReadFile/ListDir/SearchReplace
- `pending_interaction` + `interaction_resolved` 审批对，`PendingKind` 3 种值（permission/question/plan_approval）
- `turn_completed` usage 解析（`PromptUsage` + `PromptUsageModel` 结构，含 `costUsdTicks`/`usageIsIncomplete`）
- ACP `ToolCallStatus` 4 值（pending/in_progress/completed/failed）vs `_meta.updateParams.status` PascalCase
- `ToolCallContent` 3 变体（content/diff/terminal），`Diff` 结构（path/old_text/new_text/_meta）

**Opencode 专属**：
- `finish: "tool-calls"` 非终态（源码：`session/prompt.ts:347,379`）
- `finish` 是自由格式 string（非固定枚举），可能值：stop/tool-calls/error/content-filter/unknown/length/end_turn
- `parts_with_streamed_delta` → `TextReplace` vs `TextDelta`
- `message.part.delta` 是 ephemeral（不持久化到 DB），测试需通过 SSE mock
- legacy tool 格式（state=string）vs 新版（state=object，4 种 ToolState 变体）
- `ToolStatePending` 包含 `raw` 字段（原始 JSON 字符串）
- `ToolStateCompleted` 包含 `attachments` 和 `time.compacted`
- `task` 工具 → subagent 关联（XML output 解析 `<task id="ses_..." state="completed">`）
- 用户消息去重（`pending_synthetic_user_texts` + `normalize_user_text` 空白归一化）
- `step-start` / `step-finish` / `patch` / `file` / `snapshot` / `agent` / `subtask` / `retry` / `compaction` 不产生 UI 事件
- `session.status` 3 种状态（idle/busy/retry），retry 携带 `action` 恢复操作

---

## 6. 文档索引

### Minos schema 文档
- [docs/schema-grok-acp.md](schema-grok-acp.md) — Grok ACP 完整 schema（DB + 源码双重验证）
- [docs/schema-opencode.md](schema-opencode.md) — Opencode 完整 schema（DB + 源码双重验证）
- [docs/architecture-grok-acp-projection.md](architecture-grok-acp-projection.md) — Grok ACP 投影清单（既有文档）

### 外部源码（权威类型定义）
- `/Users/fannnzhang/code/github.com/grok-build` — Grok Build 源码（Rust）
  - `crates/codegen/xai-grok-shell/src/extensions/notification.rs` — xAI SessionUpdate 扩展枚举
  - `crates/codegen/xai-grok-tools/src/types/tool.rs` — ToolKind / ToolNamespace 枚举
  - `crates/codegen/xai-grok-tools/src/types/tool_io.rs` — ToolInput rawInput 变体
  - `crates/codegen/xai-grok-tools/src/types/output.rs` — ToolOutput rawOutput 变体
  - `crates/codegen/xai-grok-tools/src/tool_taxonomy.rs` — CanonicalToolMeta (x.ai/tool)
  - `crates/codegen/xai-grok-shell/src/session/acp_session_impl/updates.rs` — _meta 流式字段构建
- `/Users/fannnzhang/code/github.com/opencode` — Opencode 源码（TypeScript）
  - `packages/core/src/v1/session.ts` — V1 wire 协议全部类型（Session/Message/Part/ToolState）
  - `packages/core/src/v1/session.ts:573-632` — V1 事件定义
  - `packages/opencode/src/session/status.ts` — SessionStatus 类型
  - `packages/opencode/src/session/message-v2.ts:57-72` — PartDelta 事件定义
  - `packages/core/src/session/sql.ts` — DB 表定义
  - `packages/core/src/session/event.ts` — V2 event-sourcing 事件定义

### Codex 参考
- [.agents/skills/codex-app-server/references/json-v2-protocol.md](../.agents/skills/codex-app-server/references/json-v2-protocol.md) — Codex v2 协议参考

### Minos 翻译器实现
- [crates/minos-ui-protocol/src/grok.rs](../crates/minos-ui-protocol/src/grok.rs) — Grok 翻译器（3056 行）
- [crates/minos-ui-protocol/src/opencode.rs](../crates/minos-ui-protocol/src/opencode.rs) — Opencode 翻译器（1597 行）
- [crates/minos-ui-protocol/tests/golden/codex/](../crates/minos-ui-protocol/tests/golden/codex/) — Codex golden 测试（13 cases）
