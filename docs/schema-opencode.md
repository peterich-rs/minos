# Opencode 原始数据 Schema（对标 Codex App-Server v2）

> **双重验证**：① `~/.local/share/opencode/opencode.db`（opencode CLI 原生存储，25,000+ 条事件）；
> ② opencode 源码 `/Users/fannnzhang/code/github.com/opencode` 权威类型定义（TypeScript + Effect Schema）。
>
> **核心文件**：
> - V1 wire 协议类型：`packages/core/src/v1/session.ts`
> - V1 事件定义：`packages/core/src/v1/session.ts:573-632`
> - Session 状态事件：`packages/opencode/src/session/status.ts`
> - Part delta 事件：`packages/opencode/src/session/message-v2.ts:57-72`
> - 权限事件：`packages/opencode/src/permission/index.ts:11-21`
> - V2 事件溯源层：`packages/core/src/session/event.ts`
> - DB Schema：`packages/core/src/session/sql.ts`、`packages/core/src/event/sql.ts`
> - SSE 端点：`packages/opencode/src/server/routes/instance/httpapi/handlers/event.ts`
>
> 存储为 SQLite event-sourcing 模式：`event` 表（append-only）、`message` 表、`part` 表。
> Minos 通过 opencode SSE subscription 消费事件，wire 格式与 DB 存储格式有细微差异（见 §1）。

---

## 1. Wire 格式 vs DB 存储格式

### 1.1 Minos 消费的 Wire 格式（翻译器入口）

Minos 的 `OpencodeTranslatorState::translate()` 接收的事件格式：

```jsonc
{
  "type": "message.part.updated",         // 事件类型
  "properties": {                          // 载荷（对应 DB 中 event.data）
    "sessionID": "sess_...",
    "part": { ... },
    "info": { ... }
  }
}
```

或对于 codex-style 的合成事件：

```jsonc
{
  "method": "item/started",               // codex-style 方法名
  "params": {
    "item": { "type": "userMessage", "id": "...", "content": [...] }
  }
}
```

### 1.2 事件类型一览

> **源码**：`packages/core/src/v1/session.ts:573-632`（V1 wire 协议事件定义）

Minos 翻译器处理的事件类型（`raw["type"]`）：

| 事件类型 | 来源 | 说明 |
|----------|------|------|
| `session.created` | opencode | 会话创建 |
| `session.updated` | opencode | 会话元数据更新（标题等） |
| `session.status` | opencode | 会话状态变更（idle/busy/retry） |
| `session.idle` | opencode | 会话空闲（已废弃，被 session.status 包含） |
| `session.error` | opencode | 会话错误 |
| `message.updated` | opencode | 消息创建/更新（含角色、完成状态） |
| `message.part.updated` | opencode | 消息 part 创建/更新 |
| `message.part.delta` | opencode | 流式增量（text/reasoning） |
| `permission.updated` | opencode | 权限变更（透传为 Raw） |
| `minos.subagent.spawned` | Minos 合成 | 子 agent 派生 |
| `item/started` | Minos 合成 | codex-style 用户消息 |

**V1 wire 协议完整事件列表**（`packages/core/src/v1/session.ts:573-632`）：

| 事件类型 | DB type | `data` 顶层字段 |
|----------|---------|----------------|
| `session.created` | `session.created.1` | `{ sessionID, info }` |
| `session.updated` | `session.updated.1` | `{ sessionID, info }` |
| `session.deleted` | `session.deleted.1` | `{ sessionID, info }` |
| `message.updated` | `message.updated.1` | `{ sessionID, info }` |
| `message.removed` | `message.removed.1` | `{ sessionID, messageID }` |
| `message.part.updated` | `message.part.updated.1` | `{ sessionID, part, time }` |
| `message.part.removed` | `message.part.removed.1` | `{ sessionID, messageID, partID }` |
| `message.part.delta` | （ephemeral，不持久化） | `{ sessionID, messageID, partID, field, delta }` |

> **注意 `message.part.delta` 是 ephemeral 事件**：不写入 DB event 表，仅通过 SSE live stream 传输。
> DB 中只有 `message.part.updated`（完整快照）。

**其他 wire 事件**（不在 Minos 翻译器处理范围，但存在于 SSE stream 中）：

| 事件类型 | 说明 |
|----------|------|
| `session.compacted` | 上下文压缩 |
| `session.diff` | 会话 diff |
| `file.edited` | 文件编辑 |
| `todo.updated` | Todo 更新 |
| `command.executed` | 命令执行 |
| `permission.asked` | 权限请求（运行时名，SDK 生成名可能是 `permission.updated`） |
| `permission.replied` | 权限回复 |
| `file.watcher.updated` | 文件监听更新 |
| `vcs.branch.updated` | VCS 分支更新 |
| `pty.created` / `pty.updated` / `pty.exited` / `pty.deleted` | PTY 生命周期 |
| `server.connected` / `server.instance.disposed` | 连接生命周期 |

### 1.2.1 V1 与 V2 双事件系统

> **源码**：`packages/core/src/session/event.ts:50-509`

Opencode 内部有 **V2 event-sourcing** 层（`session.next.*` 前缀），通过 bridge 投影为 V1 wire 协议事件。
Minos 只消费 V1 wire 协议（SSE `/event` 端点）。V2 事件仅作为内部持久化层，不直接暴露给 SSE 客户端。

**V2 durable 事件**（持久化到 DB）：

| V2 事件类型 | 说明 |
|------------|------|
| `session.next.agent.switched` | agent 切换 |
| `session.next.model.switched` | 模型切换 |
| `session.next.prompted` | 用户 prompt |
| `session.next.prompt.admitted` | prompt 被接受 |
| `session.next.prompt.promoted` | prompt 被提升 |
| `session.next.step.started` / `.ended` / `.failed` | 推理步骤生命周期 |
| `session.next.text.started` / `.ended` | 文本生成生命周期 |
| `session.next.tool.input.started` / `.ended` | 工具入参生命周期 |
| `session.next.tool.called` / `.progress` / `.success` / `.failed` | 工具调用生命周期 |
| `session.next.reasoning.started` / `.ended` | 推理生成生命周期 |
| `session.next.compaction.started` / `.ended` | 上下文压缩生命周期 |
| `session.next.retried` | 重试 |

**V2 ephemeral 事件**（仅 live stream，不持久化）：

| V2 事件类型 | 说明 |
|------------|------|
| `session.next.text.delta` | 文本流式增量 |
| `session.next.tool.input.delta` | 工具入参流式增量 |
| `session.next.reasoning.delta` | 推理流式增量 |
| `session.next.compaction.delta` | 压缩流式增量 |

### 1.3 DB event 表结构

```sql
CREATE TABLE event (
  id           TEXT PRIMARY KEY,
  aggregate_id TEXT NOT NULL,      -- = session_id
  seq          INTEGER NOT NULL,    -- 单调递增
  type         TEXT NOT NULL,       -- "session.created.1" 等（带版本后缀）
  data         TEXT NOT NULL        -- JSON 载荷
);
```

DB 中 `type` 带 schema 版本后缀（`.1`），`data` 对应 wire 格式的 `properties`：

| DB `type` | Wire `type` | `data` 顶层字段 |
|-----------|-------------|----------------|
| `session.created.1` | `session.created` | `{ sessionID, info }` |
| `session.updated.1` | `session.updated` | `{ sessionID, info }` |
| `message.updated.1` | `message.updated` | `{ sessionID, info }` |
| `message.part.updated.1` | `message.part.updated` | `{ sessionID, part, time }` |

### 1.4 事件数量分布（真实数据）

| DB type | 数量 |
|---------|------|
| `message.part.updated.1` | 16,966 |
| `message.updated.1` | 6,189 |
| `session.updated.1` | 1,758 |
| `session.created.1` | 57 |

---

## 2. Session Schema

### 2.1 `session.created` / `session.updated` — `info` 结构

> **源码**：`packages/core/src/v1/session.ts:545-571` — `SessionInfo`

```jsonc
{
  "id": "ses_0a5a73b81ffe...",           // SessionSchema.ID (branded string)
  "slug": "curious-mountain",
  "projectID": "global",
  "workspaceID": "ws_...",               // 可选
  "directory": "/Users/.../project",
  "path": "",                            // 可选：子路径
  "parentID": "ses_0a5acd597...",        // 可选：父 session（subagent）
  "summary": {                           // 可选
    "additions": 42,
    "deletions": 10,
    "files": 3,
    "diffs": [ { "file": "...", "patch": "..." } ]   // 可选
  },
  "cost": 0.0,                           // 可选
  "tokens": {                            // 可选
    "input": 0, "output": 0, "reasoning": 0,
    "cache": { "read": 0, "write": 0 }
  },
  "share": { "url": "..." },             // 可选
  "title": "Audit harness coupling...",
  "agent": "explore",                    // 可选
  "model": {                             // 可选（updated 时存在）
    "id": "glm-5.2-zp",
    "providerID": "webank-provider-zp",
    "variant": "max"                     // 可选
  },
  "version": "1.17.18",
  "metadata": { ... },                   // 可选：Record<string, any>
  "time": {
    "created": 1783927194750,            // unix ms
    "updated": 1783927194755,
    "compacting": 1783927200000,         // 可选
    "archived": 1783927300000            // 可选
  },
  "permission": [                        // 可选：PermissionV1.Ruleset
    { "permission": "todowrite", "pattern": "*", "action": "deny" },
    { "permission": "task", "pattern": "*", "action": "deny" }
  ],
  "revert": {                            // 可选
    "messageID": "msg_...",
    "partID": "prt_...",                 // 可选
    "snapshot": "...",                   // 可选
    "diff": "..."                        // 可选
  }
}
```

### 2.1.1 `session.status` 事件

> **源码**：`packages/opencode/src/session/status.ts:9-33` — `Info` union

```jsonc
// idle 状态：
{ "sessionID": "ses_...", "status": { "type": "idle" } }

// busy 状态：
{ "sessionID": "ses_...", "status": { "type": "busy" } }

// retry 状态：
{
  "sessionID": "ses_...",
  "status": {
    "type": "retry",
    "attempt": 2,                        // 当前重试次数
    "message": "Rate limit exceeded",
    "action": {                          // 可选：用户可执行的恢复操作
      "reason": "rate_limit",
      "provider": "anthropic",
      "title": "Switch provider",
      "message": "Rate limited by Anthropic",
      "label": "Switch",
      "link": "https://..."              // 可选
    },
    "next": 1784701000000                // 下次重试时间（unix ms）
  }
}
```

### 2.2 DB `session` 表（持久化后的 session 元数据）

```sql
CREATE TABLE session (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  parent_id TEXT,                         -- subagent 链
  slug TEXT NOT NULL,
  directory TEXT NOT NULL,
  title TEXT NOT NULL,
  version TEXT NOT NULL,
  share_url TEXT,
  summary_additions INTEGER,
  summary_deletions INTEGER,
  summary_files INTEGER,
  summary_diffs TEXT,
  revert TEXT,
  permission TEXT,
  time_created INTEGER NOT NULL,
  time_updated INTEGER NOT NULL,
  time_compacting INTEGER,
  time_archived INTEGER,
  workspace_id TEXT,
  path TEXT,
  agent TEXT,                             -- "build" | "explore" | "general"
  model TEXT,                             -- JSON: { id, providerID, variant }
  cost REAL DEFAULT 0,
  tokens_input INTEGER DEFAULT 0,
  tokens_output INTEGER DEFAULT 0,
  tokens_reasoning INTEGER DEFAULT 0,
  tokens_cache_read INTEGER DEFAULT 0,
  tokens_cache_write INTEGER DEFAULT 0,
  metadata TEXT
);
```

---

## 3. Message Schema

### 3.1 `message.updated` — `info` 结构（user）

> **源码**：`packages/core/src/v1/session.ts:334` — `User` struct

```jsonc
{
  "id": "msg_f5a539acb001...",           // MessageID (branded "msg_*")
  "sessionID": "ses_...",
  "role": "user",
  "time": { "created": 1783926856395 },
  "format": {                            // 可选
    "type": "text"                       // 或 { type: "json_schema", schema, retryCount }
  },
  "summary": {                           // 可选
    "title": "...",                      // 可选
    "body": "...",                       // 可选
    "diffs": [
      { "file": "apps/desktop/.../daemon.rs", "patch": "Index: ...\n..." }
    ]
  },
  "agent": "build",
  "model": {
    "providerID": "webank-provider-zp",
    "modelID": "glm-5.2-zp",
    "variant": "max"                     // 可选
  },
  "system": "...",                       // 可选：系统 prompt 覆盖
  "tools": { "bash": true, "read": true }  // 可选：工具启用/禁用覆盖
}
```

### 3.2 `message.updated` — `info` 结构（assistant）

> **源码**：`packages/core/src/v1/session.ts:368` — `Assistant` struct

```jsonc
{
  "id": "msg_f83f346b4001...",
  "sessionID": "ses_...",
  "role": "assistant",
  "time": {
    "created": 1784625186484,
    "completed": 1784625200000          // 可选：存在表示消息完成
  },
  "error": {                            // 可选：AssistantError discriminated union
    "name": "rate_limit",               // error 类型 discriminator
    ...                                  // 类型特定字段
  },
  "parentID": "msg_f83e894b7001...",     // 对应的 user 消息 ID
  "modelID": "glm-5.2-zp",
  "providerID": "webank-provider-zp",
  "mode": "build",                       // "build" | "explore" | "general"
  "agent": "build",
  "path": {
    "cwd": "/Users/fannnzhang/code/github.com/Minos",
    "root": "/Users/fannnzhang/code/github.com/Minos"
  },
  "summary": false,                      // 可选：是否为摘要消息
  "cost": 0.0,
  "tokens": {
    "total": 442311,                     // 可选
    "input": 432353,
    "output": 9958,
    "reasoning": 6134,
    "cache": { "read": 376576, "write": 0 }
  },
  "structured": { ... },                 // 可选：结构化输出
  "variant": "max",                      // 可选：模型 variant
  "finish": "stop"                       // 可选：见 §3.3
}
```

### 3.3 `finish` 值与终态判定

> **源码**：`packages/core/src/v1/session.ts` — `Assistant.finish: Schema.optional(Schema.String)`
>
> `finish` 是自由格式 string，不是固定枚举。实际值来自 AI SDK 的 `finishStep` reason 或 provider 透传。

翻译器通过 `finish` + `time.completed` 判断消息是否终态：

| `finish` 值 | 终态？ | 来源 | 说明 |
|-------------|--------|------|------|
| 无（仅 `time.completed`） | ✅ 终态 | 默认 | 正常完成 |
| `"stop"` | ✅ 终态 | `session/prompt.ts:1351` | 结构化输出完成 |
| `"tool-calls"` | ❌ 非终态 | `session/prompt.ts:347,379` | 工具调用后继续 |
| `"error"` | ✅ 终态 | `session/processor.ts:929` | 步骤失败 |
| `"content-filter"` | ✅ 终态 | `session/prompt.ts:1362` | 内容过滤 |
| `"unknown"` | ✅ 终态 | `session/prompt.ts:1356` | 未知原因 |
| `"length"` | ✅ 终态 | provider 透传 | 达到长度限制 |
| `"end_turn"` | ✅ 终态 | provider 透传 | provider 的 turn 结束 |

> **翻译器逻辑**（`opencode.rs:822`）：`opencode_message_completion_is_terminal()` —
> 如果 `time.completed` 不存在 → 非终态；
> 如果 `finish` 不存在 → 终态；
> 如果 `finish == "tool-calls"` → 非终态；
> 其他 → 终态。

### 3.4 内联 parts（`message.updated` 携带）

`message.updated` 的 `info` 可以直接内联 `parts` 数组（快照式），也可通过 `message.part.updated` 逐个推送。

### 3.5 DB `message` 表

```sql
CREATE TABLE message (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  time_created INTEGER NOT NULL,
  time_updated INTEGER NOT NULL,
  data TEXT NOT NULL                      -- JSON: 同 §3.1/§3.2 的 info 结构
);
```

---

## 4. Part Schema（核心内容模型）

Part 是 opencode 的原子内容单元，类似 Codex 的 `ThreadItem`。一条 message 可包含多个 part。

### 4.1 Part 类型分布（真实数据）

| `part.type` | 数量 | Codex 对标 |
|-------------|------|-----------|
| `tool` | 10,750 | `CommandExecution` / `FileChange` / `McpToolCall` / `DynamicToolCall` |
| `text` | 2,700 | `AgentMessage` |
| `step-start` | 1,490 | （无直接对标，类似 turn 内的 step 边界） |
| `step-finish` | 1,483 | （类似 `Turn` 内 step 完成的 token 统计） |
| `patch` | 326 | `FileChange` |
| `file` | 1 | `ImageView` / 文件附件 |
| `reasoning` | — | `Reasoning`（实时流中有，DB 样本未命中） |

> **源码**：`packages/core/src/v1/session.ts:359` — `Part` union（12 个变体）

**完整 Part Union**（DB 样本只命中了 6 种，源码定义了 12 种）：

| `part.type` | 是否在 DB 样本中命中 | 说明 |
|-------------|---------------------|------|
| `text` | ✅ | 助手/用户文本 |
| `tool` | ✅ | 工具调用 |
| `step-start` | ✅ | 推理步骤开始 |
| `step-finish` | ✅ | 推理步骤完成 |
| `patch` | ✅ | 文件变更汇总 |
| `file` | ✅ | 文件附件 |
| `reasoning` | ❌ | 推理文本（实时流中有） |
| `snapshot` | ❌ | 代码库快照 |
| `agent` | ❌ | agent 注解（如 `@explore`） |
| `subtask` | ❌ | 子任务声明 |
| `retry` | ❌ | 重试信息 |
| `compaction` | ❌ | 上下文压缩 |

### 4.2 `text` Part

> **源码**：`packages/core/src/v1/session.ts:96`

```jsonc
{
  "id": "prt_f5a539acc001...",            // PartID (branded "prt_*")
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "text",
  "text": "continue",                     // 完整文本
  "synthetic": false,                     // 可选：是否合成的
  "ignored": false,                       // 可选：是否被忽略的
  "time": {                               // 可选
    "start": 1783926856400,
    "end": 1783926856500                  // 可选：存在表示完成
  },
  "metadata": { ... }                     // 可选：Record<string, any>
}
```

**流式 delta 等价物**（`message.part.delta`，ephemeral，不持久化）：

> **源码**：`packages/opencode/src/session/message-v2.ts:61-70`

```jsonc
{
  "sessionID": "sess_1",
  "messageID": "msg_a1",
  "partID": "part_1",
  "field": "text",                        // "text" | "" (reasoning 也用 "text")
  "delta": "Hello"                        // 增量文本
}
```

### 4.3 `reasoning` Part

> **源码**：`packages/core/src/v1/session.ts:112`

```jsonc
{
  "id": "prt_...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "reasoning",
  "text": "...",
  "metadata": { ... },                    // 可选
  "time": {
    "start": 1783926856400,               // 必须存在
    "end": 1783926856500                  // 可选
  }
}
```

### 4.4 `tool` Part（最复杂）

```jsonc
{
  "id": "prt_f83f343ac001...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "tool",
  "callID": "call_eb71c7d995094bdaa88d847e",  // 工具调用 ID
  "tool": "edit",                              // 工具名（见 §4.4.2）
  "state": {
    "status": "running",                       // "pending" | "running" | "completed" | "error"
    "input": { ... },                          // 工具入参（见 §4.4.3）
    "output": "...",                           // 工具输出（completed 时存在）
    "error": "...",                            // 错误信息（error 时存在）
    "title": "apps/desktop/.../Timeline.tsx",  // 显示标题
    "metadata": { "preview": "..." }           // 可选元数据
  }
}
```

#### 4.4.1 tool status 状态机

> **源码**：`packages/core/src/v1/session.ts:253-319` — `ToolState` union

```
pending → running → completed
                  → error
```

**`ToolStatePending`**（`session.ts:253`）：
```jsonc
{
  "status": "pending",
  "input": { ... },          // Record<string, any>：解析后的 JSON 对象
  "raw": "..."               // 原始 JSON 字符串（入参的原始序列化形式）
}
```

**`ToolStateRunning`**（`session.ts:260`）：
```jsonc
{
  "status": "running",
  "input": { ... },
  "title": "...",             // 可选：显示标题
  "metadata": { ... },        // 可选：工具特定元数据
  "time": { "start": 1784625186 }
}
```

**`ToolStateCompleted`**（`session.ts:271`）：
```jsonc
{
  "status": "completed",
  "input": { ... },
  "output": "...",            // 工具输出文本
  "title": "...",             // 显示标题
  "metadata": { ... },        // 工具特定元数据（如 preview）
  "time": {
    "start": 1784625186,
    "end": 1784625187,
    "compacted": 1784625188   // 可选：输出被压缩的时间
  },
  "attachments": [ ... ]      // 可选：FilePart[] 附件
}
```

**`ToolStateError`**（`session.ts:286`）：
```jsonc
{
  "status": "error",
  "input": { ... },
  "error": "...",             // 错误消息文本
  "metadata": { ... },        // 可选
  "time": { "start": ..., "end": ... }
}
```

| status | 数量 | 翻译器行为 |
|--------|------|-----------|
| `running` | 5,982 | `ToolCallPlaced` |
| `pending` | 2,412 | `ToolCallPlaced` |
| `completed` | 2,347 | `ToolCallCompleted` (is_error=false) |
| `error` | 60 | `ToolCallCompleted` (is_error=true) |

#### 4.4.2 tool 名称列表

| `tool` | 数量 | 说明 |
|--------|------|------|
| `bash` | 4,752 | 终端命令 |
| `read` | 3,534 | 文件读取 |
| `grep` | 873 | 内容搜索 |
| `edit` | 675 | 文件编辑 |
| `glob` | 411 | 文件匹配 |
| `todowrite` | 210 | 计划/Todo |
| `task` | 131 | 子 agent 派生 |
| `skill` | 60 | Skill 调用 |
| `question` | 44 | 用户交互 |
| `minos_teamwork_*` | 111 | Minos MCP 工具 |

#### 4.4.3 tool `input` 按工具名

> **源码**：`packages/opencode/src/tool/` 目录下各工具定义文件

**`read`** (`tool/read.ts:28`)：
```jsonc
{ "filePath": "/path/to/file.ts", "offset": 834, "limit": 30 }
```

**`edit`** (`tool/edit.ts:47`)：
```jsonc
{
  "filePath": "/path/to/file.ts",
  "oldString": "...",
  "newString": "...",
  "replaceAll": false          // 可选：是否全部替换
}
```

**`write`** (`tool/write.ts:20`)：
```jsonc
{ "content": "...", "filePath": "/path/to/file" }
```

**`bash`** (`tool/shell/prompt.ts:22`)：
```jsonc
{ "command": "cargo build", "timeout": 60000, "workdir": "/path", "description": "Build the project" }
```

**`grep`** (`tool/grep.ts:10`)：
```jsonc
{ "pattern": "TODO", "path": "/src", "include": "*.ts" }
```

**`glob`** (`tool/glob.ts:10`)：
```jsonc
{ "pattern": "**/*.rs", "path": "/src" }
```

**`todowrite`** (`tool/todo.ts:17`)：
```jsonc
{ "todos": [ { "content": "...", "status": "pending", "priority": "high" } ] }
```

**`task`** (`tool/task.ts:56`)：
```jsonc
{
  "description": "Audit harness coupling",
  "prompt": "...",
  "subagent_type": "explore",
  "task_id": "...",           // 可选
  "command": "...",           // 可选
  "background": false         // 可选
}
```

**`webfetch`** (`tool/webfetch.ts`)：
```jsonc
{ "url": "https://...", "prompt": "...", "format": "markdown" }
```

**`websearch`** (`tool/websearch.ts`)：
```jsonc
{ "query": "..." }
```

**`question`** (`tool/question.ts`)：
```jsonc
{
  "question": "Which approach?",
  "header": "Choose",
  "options": [ { "label": "A", "description": "..." } ]
}
```

#### 4.4.4 tool `output`（completed 时）

**`read` 输出格式**（带行号 + XML 标签）：
```
<path>/Users/.../Timeline.tsx</path>
<type>file</type>
<content>
834: }) {
835:   const openSessionTranscript = useUiStore((s) => s.openSessionTranscript);
...
(Showing lines 834-863 of 1020. Use offset=864 to continue.)
</content>
```

**`task` 工具输出**（子 agent 完成时的 XML 格式）：
```xml
<task id="ses_..." state="completed">
  ...result...
</task>
```

翻译器从此格式中提取 `sub_session_id`。

**Minos UI 投影（desktop / TUI，2026-07-23+）**：

| Wire | UI |
|------|-----|
| `tool: task` + `minos.subagent.spawned` + status | 单张 `subagent` 卡：`Running/Ran subagent {agent} #{short} · {status}` + 短 description（非整段 prompt） |
| task completed XML | 只用于抽 `sub_session_id` / 终态；**禁止**作为 transcript header（`Ran <task id=…>`） |
| text/reasoning `part.id` | 事件 `message_id` 绑定为 `messageID + U+001E + partID`，多 part 分段；tool 后 `TextReplace` 不回写上方气泡 |

详见 `architecture-desktop.md` timeline freeze / subagent 单卡。

### 4.5 `step-start` Part

> **源码**：`packages/core/src/v1/session.ts:227`

标记推理步骤的开始（类似多步推理的边界）：

```jsonc
{
  "id": "prt_f83f354b9001...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "step-start",
  "snapshot": "c49ec7f4777a2a331d6d7b031db2c0f61e99a790"   // 可选：代码库 git snapshot hash
}
```

### 4.6 `step-finish` Part

> **源码**：`packages/core/src/v1/session.ts:234`

标记推理步骤完成，携带 token 使用量：

```jsonc
{
  "id": "prt_f83f34663001...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "step-finish",
  "reason": "tool-calls",                    // finish reason（同 message.finish）
  "snapshot": "dd6d6acc8d25493e8d4f82db822bf852f73b0c00",   // 可选
  "cost": 0,
  "tokens": {
    "total": 191529,                         // 可选
    "input": 2362,
    "output": 623,
    "reasoning": 0,
    "cache": { "read": 188544, "write": 0 }
  }
}
```

### 4.7 `patch` Part

文件变更汇总：

```jsonc
{
  "id": "prt_f83f371bb001...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "patch",
  "hash": "c49ec7f4777a2a331d6d7b031db2c0f61e99a790",
  "files": [
    "/Users/.../src-tauri/src/lib.rs",
    "/Users/.../Timeline.tsx",
    "/Users/.../architecture-desktop.md"
  ]
}
```

### 4.8 `file` Part

> **源码**：`packages/core/src/v1/session.ts:165`

文件附件（如工作目录引用）：

```jsonc
{
  "type": "file",
  "mime": "application/x-directory",
  "filename": ".",
  "url": "file:///Users/fannnzhang/code/github.com/Minos",
  "source": { ... }    // 可选：FileSource | SymbolSource | ResourceSource
}
```

### 4.9 `snapshot` Part

> **源码**：`packages/core/src/v1/session.ts:81`

代码库快照（git snapshot hash）：

```jsonc
{
  "id": "prt_...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "snapshot",
  "snapshot": "c49ec7f4777a2a331d6d7b031db2c0f61e99a790"
}
```

### 4.10 `agent` Part

> **源码**：`packages/core/src/v1/session.ts:175`

Agent 注解（如 `@explore`、`@build` mention）：

```jsonc
{
  "id": "prt_...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "agent",
  "name": "explore",
  "source": {                    // 可选：mention 在文本中的位置
    "value": "@explore",
    "start": 0,                  // 字符偏移
    "end": 8
  }
}
```

### 4.11 `subtask` Part

> **源码**：`packages/core/src/v1/session.ts:198`

子任务声明（task 工具派生子 agent 的元数据）：

```jsonc
{
  "id": "prt_...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "subtask",
  "prompt": "Audit harness coupling in codebase",
  "description": "Audit harness coupling (@explore subagent)",
  "agent": "explore",
  "model": {                     // 可选
    "providerID": "webank-provider-zp",
    "modelID": "glm-5.2-zp"
  },
  "command": "..."               // 可选
}
```

### 4.12 `retry` Part

> **源码**：`packages/core/src/v1/session.ts:214`

重试信息（API 调用失败后重试的记录）：

```jsonc
{
  "id": "prt_...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "retry",
  "attempt": 1,                  // 重试次数
  "error": { ... },              // APIError 结构
  "time": { "created": 1784625186484 }
}
```

### 4.13 `compaction` Part

> **源码**：`packages/core/src/v1/session.ts:189`

上下文压缩标记（上下文窗口接近上限时的自动压缩）：

```jsonc
{
  "id": "prt_...",
  "sessionID": "ses_...",
  "messageID": "msg_...",
  "type": "compaction",
  "auto": true,                          // 是否自动触发
  "overflow": true,                      // 可选：是否因溢出触发
  "tail_start_id": "msg_..."             // 可选：压缩后保留的尾部起始 message ID
}
```

### 4.9 Legacy tool 格式（兼容旧版 opencode）

翻译器同时支持旧版 flat 结构：

```jsonc
{
  "type": "tool",
  "callID": "call_...",
  "name": "edit",                    // 旧版用 "name" 而非 "tool"
  "state": "calling",               // 旧版 state 是 string 而非 object
  "args": { ... },                  // 旧版用 "args" 而非 "state.input"
  "output": "...",                  // 旧版直接在顶层
  "is_error": false                 // 旧版直接在顶层
}
```

| Legacy state | 新版 state.status | 翻译器行为 |
|-------------|-------------------|-----------|
| `"calling"` | `"pending"` / `"running"` | `ToolCallPlaced` |
| `"complete"` | `"completed"` | `ToolCallCompleted` (is_error=false) |

---

## 5. Minos 合成事件

### 5.1 `minos.subagent.spawned`

Minos 在检测到 opencode `task` 工具创建子 session 时注入：

```jsonc
{
  "type": "minos.subagent.spawned",
  "properties": {
    "parent_session_id": "sess_...",
    "sub_session_id": "ses_...",
    "tool_call_id": "call_...",
    "model": "glm-5.2-zp",         // 可选
    "prompt": "...",                // 可选
    "title": "..."                  // 可选
  }
}
```

### 5.2 `item/started`（codex-style 用户消息合成）

Minos 将 opencode 用户消息包装为 codex-style 的 `item/started`：

```jsonc
{
  "method": "item/started",
  "params": {
    "item": {
      "type": "userMessage",
      "id": "user_1",
      "content": [ { "type": "text", "text": "hello opencode" } ]
    }
  }
}
```

翻译器对此产生：`MessageStarted(user)` + `TextDelta`，并缓存文本用于后续 part 去重。

---

## 6. Opencode ↔ Codex 概念映射总表

| 概念 | Opencode | Codex v2 |
|------|---------|---------|
| 会话标识 | `sessionID` (`ses_*`) | `threadId` |
| Turn 标识 | 无显式 turn（由 message + step-start/finish 隐含） | `turnId`（独立字段） |
| 消息标识 | `messageID` (`msg_*`) | `itemId`（在 ThreadItem 内） |
| Part 标识 | `partID` (`prt_*`) | 无（Codex 用 itemId 直接寻址） |
| 助手文本 | `message.part.updated` → type=`text` | `item/agentMessage/delta` + `item/completed` |
| 推理文本 | `message.part.updated` → type=`reasoning` | `item/reasoning/textDelta` + `summaryTextDelta` |
| 用户消息 | `message.updated` → role=`user` | `item/started` → `UserMessage` |
| 工具调用 | `message.part.updated` → type=`tool` | `item/started/completed` → `CommandExecution`/`FileChange` |
| 工具入参 | `state.input` (per-tool shape) | `ThreadItem.arguments` / `command` |
| 工具输出 | `state.output` (string, XML-like) | `ThreadItem.aggregatedOutput` / `result` |
| 工具状态 | `state.status` (pending/running/completed/error) | ThreadItem variant status enum |
| 流式 delta | `message.part.delta` (field=`text`) | `item/*/delta` 系列 |
| Step 边界 | `step-start` / `step-finish` | 无直接对标（Codex 用 Turn） |
| Token usage | `step-finish.tokens` / `message.tokens` | `Turn` 上的 tokens 字段 |
| 文件变更 | `patch` part (files + hash) | `FileChange` ThreadItem + `fileChange/patchUpdated` |
| 计划/Todo | `todowrite` tool | `Plan` ThreadItem + `item/plan/delta` |
| 子 agent | `task` tool + `minos.subagent.spawned` | `CollabAgentToolCall` ThreadItem |
| 审批 | `permission.updated` (透传 Raw) | `commandExecution/requestApproval` (server request) |
| 寻址模型 | `sessionID` + `messageID` + `partID` | `threadId` + `turnId` + `itemId` |

---

## 7. 关键架构差异（影响翻译器实现）

### 7.1 三层嵌套 vs 单层 item

- **Codex**: `Thread → Turn → ThreadItem`，item 是扁平的 tagged union。
- **Opencode**: `Session → Message → Part`，part 嵌套在 message 内，需要跨事件维护 message→role 映射。

### 7.2 流式 vs 快照

- **Codex**: delta（增量）和 completed（完整）是不同通知方法。
- **Opencode**: `message.part.delta` 是增量；`message.part.updated` 可以是空文本→非空（delta 模式）或直接完整文本（快照模式）。翻译器通过 `parts_with_streamed_delta` 集合判断 part 是否已经流式输出过，以决定用 `TextDelta` 还是 `TextReplace`。

### 7.3 完成判定

- **Codex**: `item/completed` 携带最终 ThreadItem。
- **Opencode**: 无显式 part-completed 通知；翻译器通过 `part.time.end` 或 `part.time.completed` 字段存在性判断。message 完成通过 `finish` + `time.completed`。

### 7.4 工具状态分散

- **Codex**: 工具调用是一个 ThreadItem 的生命周期（started → completed）。
- **Opencode**: 工具状态变迁通过同一 part 的多次 `message.part.updated` 实现（pending → running → completed/error），翻译器需要去重（`tool_calls` HashSet 确保只发一次 `ToolCallPlaced`）。

### 7.5 用户消息去重

opencode 会重复发送用户消息（实时 part + 最终快照）。翻译器通过 `pending_synthetic_user_texts` + `normalize_user_text` 去重，匹配后 suppress 后续重复。

---

## 8. 单元测试注意事项

1. **Part 流式 vs 快照双路径**：测试需覆盖 delta 先于 updated（流式）和 updated 直接携带完整文本（快照）两种场景。`TextReplace` 仅在 `parts_with_streamed_delta` 包含该 part 且 `part_is_finished()` 时触发。

2. **finish 字段非终态**：`finish: "tool-calls"` 不应触发 `MessageCompleted`。测试需覆盖 `stop`、`tool-calls`、无 finish 三种情况。

3. **Legacy + 新版 tool 格式**：翻译器同时处理旧版 flat state（string）和新版 object state。测试需覆盖两条路径。

4. **task 工具 → subagent 关联**：`task` 工具完成时，翻译器从 output 的 XML 中提取 `sub_session_id`，发 `SubagentStatusUpdated`。测试需覆盖正常 XML、缺失 id、非 task 工具三种情况。

5. **用户消息去重**：`item/started` 合成用户文本后，后续 `message.part.updated` 携带相同文本应被 suppress。测试需覆盖 normalize 差异（空格/换行）。

6. **step-start / step-finish / patch / file**：这些 part type 在翻译器中被归为 `TrackedPartKind::Other`，不产生 UI 事件。测试不应期望它们生成 `TextDelta`。

7. **tool callID 容错**：`callID` 可能缺失，翻译器 fallback 到 `part.id`。测试需覆盖两者。
