# Grok ACP 原始数据 Schema（对标 Codex App-Server v2）

> **双重验证**：① `~/.minos/daemon.sqlite` 中 `agent='grok'` 的真实事件（2 会话，7436+2812 条）；
> ② grok-build 源码 `/Users/fannnzhang/code/github.com/grok-build` 权威类型定义（Rust）。
>
> 源码分两层：
> - **ACP 标准**（upstream）：`agent-client-protocol-schema` v0.11.4 crate（`client.rs`、`tool_call.rs`、`plan.rs`、`content.rs`）
> - **xAI 扩展**：`xai-grok-shell` crate（`extensions/notification.rs`）、`xai-grok-tools` crate（`types/tool.rs`、`types/tool_io.rs`、`types/output.rs`）
>
> Minos 存储：`events.body_inline`（BLOB，原始 JSON），翻译投影缓存于 `events.projection_json`。

---

## 1. 顶层封装（Minos Wire Envelope）

Grok 事件以 JSON-RPC notification 的变体存入 Minos，由顶层 `kind` 字段区分：

| `kind` 值 | 说明 | 数量占比 |
|-----------|------|---------|
| `acp_notification` | ACP 标准 + xAI 扩展通知（绝大多数） | ~99.5% |
| `user_message` | Minos 合成的用户消息信封 | 少量 |
| `acp_prompt_response` | prompt 完成响应（stopReason） | 每个 turn 1 条 |

### 1.1 `acp_notification` 通用结构

```jsonc
{
  "kind": "acp_notification",
  "method": "session/update" | "_x.ai/session_notification" | "_x.ai/*",
  "params": {
    "_meta": { ... },          // xAI 扩展元数据（见 §1.2）
    "sessionId": "019f8870-...", // grok CLI 的 session ID
    "update": { ... }            // sessionUpdate 载荷（见 §2-§4）
  }
}
```

### 1.2 `_meta` 字段（xAI 扩展）

所有 `session/update` 通知都携带 `_meta`：

| 字段 | 类型 | 说明 |
|------|------|------|
| `eventId` | string | grok 内部事件 ID（格式 `{sessionId}-{seq}`） |
| `promptId` | string | 当前 prompt/turn 的唯一 ID |
| `agentTimestampMs` | i64 | agent 端时间戳（毫秒） |
| `turnStartMs` | i64 | 当前 turn 开始时间 |
| `streamStartMs` | i64 | 当前流式输出开始时间（变化 = 新 assistant message 边界） |
| `chunkId` | i64 | 流式 chunk 序号（从 1 开始） |
| `totalTokens` | i64 | 截至当前累计 token |
| `updateType` | string | 冗余类型名（`"AgentMessageChunk"` / `"ToolCall"` 等） |
| `updateParams` | object | tool_call 相关参数（status, toolCallId, kind, title 等） |

### 1.3 `method` 取值分布

| `method` | 数量 | 说明 |
|----------|------|------|
| `session/update` | 4260 | ACP 标准会话更新（文本流、工具调用、计划等） |
| `_x.ai/session_notification` | 195 | xAI 扩展（交互审批、turn 完成、delta chunk） |
| `_x.ai/queue/changed` | 7 | 任务队列变化 |
| `_x.ai/mcp/init_progress` | 6 | MCP 服务器初始化进度 |
| `_x.ai/mcp/server_status` | 5 | MCP 服务器状态 |
| `_x.ai/sessions/changed` | 4 | 会话列表变化 |
| `_x.ai/models/update` | 3 | 可用模型更新 |
| `_x.ai/announcements/update` | 3 | 公告更新 |
| `_x.ai/session/prompt_complete` | 2 | prompt 完成 |
| `_x.ai/task_backgrounded` | 1 | 后台任务 |
| `_x.ai/settings/update` | 1 | 设置更新 |
| `_x.ai/mcp_initialized` | 1 | MCP 初始化完成 |
| `_x.ai/mcp/servers_updated` | 1 | MCP 服务器列表更新 |

---

## 2. `session/update` 子类型（ACP 标准 `params.update.sessionUpdate`）

这是 Grok 的核心内容流，类似 Codex 的 `item/*` 通知系列。

### 2.1 子类型分布

| `sessionUpdate` 值 | 数量 | Codex 对标 |
|-------------------|------|-----------|
| `agent_message_chunk` | 3664 | `item/agentMessage/delta` |
| `agent_thought_chunk` | 415 | `item/reasoning/textDelta` |
| `tool_call_update` | 128 | `item/completed` (CommandExecution/FileChange) |
| `tool_call` | 48 | `item/started` (CommandExecution/FileChange) |
| `plan` | 3 | `item/plan/delta` + `item/completed` (Plan) |
| `user_message_chunk` | 2 | `item/started` (UserMessage) |

### 2.2 `agent_message_chunk` — 助手文本流

```jsonc
{
  "kind": "acp_notification",
  "method": "session/update",
  "params": {
    "_meta": {
      "eventId": "019f8870-...-41",
      "promptId": "3f323f60-...",
      "agentTimestampMs": 1784700504451,
      "chunkId": 39,
      "streamStartMs": 1784700502475,
      "turnStartMs": 1784700500755,
      "totalTokens": 10811,
      "updateType": "AgentMessageChunk"
    },
    "sessionId": "019f8870-...",
    "update": {
      "content": { "text": "先", "type": "text" },
      "sessionUpdate": "agent_message_chunk"
    }
  }
}
```

**与 Codex 差异**：Codex 的 `item/agentMessage/delta` 只携带 `{ delta, itemId, threadId, turnId }`，不含 token/timestamp 元数据；Grok 把所有上下文信息放在 `_meta` 里，没有独立的 `itemId` — 消息边界由 `streamStartMs` 变化推断。

### 2.3 `agent_thought_chunk` — 推理文本流

结构与 `agent_message_chunk` 完全一致，区别仅在 `sessionUpdate: "agent_thought_chunk"`。

```jsonc
"update": {
  "content": { "text": "The", "type": "text" },
  "sessionUpdate": "agent_thought_chunk"
}
```

**与 Codex 差异**：Codex 的 reasoning 有 `summaryTextDelta` + `textDelta` + `summaryPartAdded` 三种子通知，且有 `contentIndex` / `summaryIndex`；Grok 只有一种 `agent_thought_chunk`，无索引概念。

### 2.4 `user_message_chunk` — 用户消息

```jsonc
"update": {
  "_meta": { "modelId": "grok-4.5", "promptIndex": 0 },
  "content": {
    "text": "看看当前webview页面...",
    "type": "text"
  },
  "sessionUpdate": "user_message_chunk"
}
```

### 2.5 `tool_call` — 工具调用开始

```jsonc
{
  "kind": "acp_notification",
  "method": "session/update",
  "params": {
    "_meta": {
      "eventId": "019f8870-...-64",
      "promptId": "3f323f60-...",
      "updateParams": {
        "kind": "Other",        // 初始 kind（可能为 "Other"，在 update 中细化为 "read"）
        "status": "Pending",
        "title": "read_file",
        "toolCallId": "call-39dc3e70-...-0"
      },
      "updateType": "ToolCall"
    },
    "sessionId": "019f8870-...",
    "update": {
      "_meta": {
        "x.ai/tool": {                    // 工具元信息（xAI 扩展）
          "kind": "read",
          "label": "Read",
          "name": "read_file",
          "namespace": "grok_build",
          "read_only": true,
          "version": 1
        }
      },
      "rawInput": {                       // 原始工具入参（见 §3）
        "limit": 80,
        "target_file": "/Users/.../README.md"
      },
      "sessionUpdate": "tool_call",
      "title": "read_file",
      "toolCallId": "call-39dc3e70-...-0"
    }
  }
}
```

### 2.6 `tool_call_update` — 工具调用更新/完成

这是 Grok 最丰富的事件类型，可能携带工具输入、输出、状态变更。

```jsonc
{
  "kind": "acp_notification",
  "method": "session/update",
  "params": {
    "_meta": {
      "eventId": "019f8870-...-65",
      "updateParams": {
        "status": null,                  // null=进行中, "Completed"=完成, "Failed"=失败
        "toolCallId": "call-39dc3e70-...-0"
      },
      "updateType": "ToolCallUpdate"
    },
    "sessionId": "019f8870-...",
    "update": {
      "_meta": {
        "x.ai/tool": {
          "input": { "limit": 80, "path": "/Users/.../README.md" },
          "kind": "read",
          "label": "Read",
          "name": "read_file",
          "namespace": "grok_build",
          "read_only": true,
          "version": 1
        }
      },
      "kind": "read",                    // 语义 kind（read/search/edit/execute/list/fetch/...）
      "locations": [                     // 涉及文件位置（可为空数组）
        { "path": "/Users/.../README.md", "line": 1 }
      ],
      "rawInput": { ... },               // 原始入参（同 §3）
      "rawOutput": { ... },              // 原始输出（仅完成时存在，见 §3）
      "content": [ ... ],                // 结构化内容块（见 §3.5）
      "sessionUpdate": "tool_call_update",
      "title": "Read `/Users/.../README.md`",
      "toolCallId": "call-39dc3e70-...-0"
    }
  }
}
```

### 2.7 `plan` — 计划更新

```jsonc
"update": {
  "entries": [
    {
      "content": "重写 WebViewFragment 返回逻辑",
      "priority": "medium",              // "low" | "medium" | "high"
      "status": "in_progress"            // "pending" | "in_progress" | "completed" | "failed"
    },
    { "content": "统一 WebViewScreen 错误页", "priority": "medium", "status": "pending" }
  ],
  "sessionUpdate": "plan"
}
```

**与 Codex 差异**：Codex 的 Plan 是一个 `ThreadItem` 变体（`{ id, text }`），通过 `item/plan/delta` 流式更新；Grok 的 plan 直接发送完整 entries 数组（快照式），无增量 delta。

---

## 3. 工具调用详细 Schema（`rawInput` / `rawOutput` / `content`）

Grok 的工具数据采用 **`rawInput` + `rawOutput` 双通道**设计，这是与 Codex 最大的结构差异。

### 3.1 `x.ai/tool` 元信息（所有工具共有）

> **源码定义**：`crates/codegen/xai-grok-tools/src/tool_taxonomy.rs:190` — `CanonicalToolMeta`

| 字段 | 类型 | 说明 |
|------|------|------|
| `kind` | string | 语义类别（见 §3.1.1 完整列表） |
| `name` | string | harness-specific model-facing name（如 `read_file`） |
| `label` | string | cross-harness 显示名（如 `"Read"`） |
| `namespace` | string | 工具命名空间（见 §3.1.2） |
| `read_only` | bool | 是否只读 |
| `version` | i32 | 工具元版本号（当前固定为 `1`，`TOOL_META_VERSION`） |
| `input` | object? | 规范化入参投影（可选，见 §3.2.1） |

#### 3.1.1 `ToolKind` 完整枚举

> **源码**：`crates/codegen/xai-grok-tools/src/types/tool.rs:70`（xAI 扩展，`#[serde(rename_all = "snake_case")]`）

| wire 值 | 说明 |
|---------|------|
| `read` | 文件读取 |
| `edit` | 文件编辑 |
| `delete` | 文件删除 |
| `list_dir` | 目录列表 |
| `write` | 文件写入 |
| `move` | 文件移动 |
| `search` | grep 搜索 |
| `lsp` | LSP 操作 |
| `execute` | 终端命令 |
| `plan` | 计划/Todo |
| `web_search` | 网页搜索 |
| `web_fetch` | 网页抓取 |
| `background_task_action` | 后台任务操作 |
| `wait_tasks_action` | 等待任务 |
| `kill_task_action` | 终止任务 |
| `list` | 通用列表 |
| `skill` | Skill 调用 |
| `memory_search` | 记忆搜索 |
| `memory_get` | 记忆获取 |
| `task` | 子 agent 派生 |
| `enter_plan` | 进入计划模式 |
| `exit_plan` | 退出计划模式 |
| `ask_user` | 用户提问 |
| `image_gen` | 图像生成 |
| `video_gen` | 视频生成 |
| `image_to_video` | 图转视频 |
| `reference_to_video` | 参考转视频 |
| `deploy_app` | 应用部署 |
| `search_tool` | 搜索工具 |
| `use_tool` | 使用工具 |
| `monitor` | 监控 |
| `goal_update` | 目标更新 |
| `other` | 未识别（fallback） |

> **注意**：ACP upstream 的 `ToolKind`（`agent-client-protocol-schema/src/tool_call.rs:380`）是一个更小的枚举：
> `read, edit, delete, move, search, execute, think, fetch, switch_mode, other`。
> `update.kind` 字段使用的是 ACP upstream 枚举，因此实际值可能是 `think`（对应 xAI 的 `plan`）或 `other`（对应 xAI 的 `list`）。
> `x.ai/tool.kind` 使用的是 xAI 扩展枚举（上表）。

#### 3.1.2 `ToolNamespace` 枚举

> **源码**：`crates/codegen/xai-grok-tools/src/types/tool.rs:33`

| wire 值 | 说明 |
|---------|------|
| `grok_build` | Grok Build harness（默认） |
| `grok_build_concise` | Grok Build Concise |
| `grok_build_hashline` | Grok Build Hashline |
| `codex` | Codex 兼容 |
| `opencode` | OpenCode 兼容 |
| `mcp` | MCP 服务器 |

### 3.2 `rawInput` 按工具类型

> **源码**：`crates/codegen/xai-grok-tools/src/types/tool_io.rs:60` — `ToolInput` enum（`#[serde(tag = "variant")]`）

每种工具的 `rawInput` 结构不同，通过 `variant` 字段标识。完整变体列表：

| `variant` | 对应工具 | 核心字段 |
|-----------|---------|---------|
| `ReadFile` | read_file | `path, offset?, limit?, pages?, format?` |
| `SearchReplace` | search_replace | `file_path, old_string, new_string, replace_all` |
| `Bash` | run_terminal_command | `command, timeout?, description, is_background` |
| `Grep` | grep | `pattern, path?, glob?, output_mode?` |
| `ListDir` | list_dir | `target_directory` |
| `TodoWrite` | todo_write | `merge, todos[]` |
| `Write` | write | `file_path, content` |
| `WebFetch` | web_fetch | `url, prompt` |
| `WebSearch` | web_search | `query, allowed_domains?` |
| `Skill` | skill | （skill 参数） |
| `MCPTool` | MCP 工具 | `tool_name, tool_input` |
| `Task` | 子 agent | `description, prompt, ...` |
| `TaskOutput` | 任务输出获取 | `task_id, ...` |
| `WaitTasks` | 等待任务 | `task_ids[]` |
| `KillTask` | 终止任务 | `task_ids[]` |
| `ApplyPatch` | 应用补丁 | `patch` |
| `HashlineEdit` | Hashline 编辑 | `file_path, ...` |
| `CodexReadFile` | Codex 兼容读 | `path, ...` |
| `CodexListDir` | Codex 兼容列表 | `path` |
| `CodexGrepFiles` | Codex 兼容搜索 | `pattern, ...` |
| `MemorySearch` | 记忆搜索 | `query` |
| `MemoryGet` | 记忆获取 | `key` |
| `ImageGen` | 图像生成 | `prompt, ...` |
| `ImageEdit` | 图像编辑 | `image, prompt, ...` |
| `EnterPlanMode` | 进入计划模式 | （空） |
| `ExitPlanMode` | 退出计划模式 | `plan_summary` |
| `AskUserQuestion` | 用户提问 | `question, options[]` |
| `Lsp` | LSP 操作 | `action, ...` |
| `Monitor` | 监控 | `task_id, ...` |
| `Dynamic` | 动态/未识别 | 任意 JSON |

#### 3.2.1 规范化 `input` 投影（`x.ai/tool.input`）

> **源码**：`crates/codegen/xai-grok-tools/src/normalization.rs:63`

只有以下工具会产生规范化 `input` 投影（其余工具的 `input` 为 `None`，需从 `rawInput` 获取参数）：

| 工具 | `input` 字段 |
|------|-------------|
| ReadFile | `{ path, offset?, limit? }` |
| Bash | `{ command, description }` |
| SearchReplace | `{ path }` |
| Write | `{ path }` |
| ListDir | `{ directory }` |
| Grep | `{ pattern, path? }` |

#### `ReadFile`（read_file）
```jsonc
{ "variant": "ReadFile", "target_file": "/path/to/file", "limit": 80, "offset": 1 }
```

#### `Grep`（grep）
```jsonc
{
  "variant": "Grep",
  "pattern": "isLastActivity|WebViewScreen\\(",
  "path": "/path/to/search",
  "glob": "**/*.{kt,java}",
  "-i": null,           // 可选，是否忽略大小写
  "type": null           // 可选，文件类型过滤
}
```

#### `SearchReplace`（search_replace）
```jsonc
{
  "variant": "SearchReplace",
  "target_file": "/path/to/file.kt",
  // 注意：oldText/newText 在 update.content 中，不在 rawInput 中
}
```

#### `Bash`（run_terminal_command）
```jsonc
{
  "variant": "Bash",
  "command": "cd /path && ./gradlew compileDebugKotlin --quiet 2>&1",
  "description": "Compile webview-shell and capacitor-app",
  "is_background": false
}
```

#### `ListDir`（list_dir）
```jsonc
{ "variant": "ListDir", "target_directory": "/path/to/dir" }
```

#### `WebFetch`（web_fetch）
```jsonc
{ "variant": "WebFetch", "url": "https://github.com/GrenderG/Toasty" }
```

#### `TodoWrite`（todo_write）
```jsonc
{
  "variant": "TodoWrite",
  "merge": true,
  "todos": [
    { "id": "4", "content": null, "status": "completed" }
  ]
}
```

### 3.3 `rawOutput` 结构（工具完成时）

> **源码**：`crates/codegen/xai-grok-tools/src/types/output.rs:624` — `ToolOutput` enum（`#[serde(tag = "type")]`）

`rawOutput` 是 tagged union，完整变体列表：

| `type` 值 | 说明 |
|-----------|------|
| `Bash` | 终端命令输出（stdout/stderr/exit_code） |
| `BackgroundTaskStarted` | 后台任务启动 |
| `GrepSearch` | grep 搜索结果 |
| `ReadFile` | 文件读取结果 |
| `ListDir` | 目录列表结果 |
| `SearchReplace` | 编辑结果（含 diff） |
| `Todo` | Todo 更新结果 |
| `WebSearch` | 网页搜索结果 |
| `WebFetch` | 网页抓取结果 |
| `MCP` | MCP 工具结果 |
| `TaskOutput` | 任务输出获取结果 |
| `KillTask` | 终止任务结果 |
| `Skill` | Skill 调用结果 |
| `ApplyPatch` | 补丁应用结果 |
| `CodexGrepFiles` | Codex 兼容搜索结果 |
| `SearchTool` | 搜索工具结果 |
| `SubagentCompleted` | 子 agent 完成结果 |
| `EnterPlanMode` | 进入计划模式结果 |
| `ExitPlanMode` | 退出计划模式结果 |
| `AskUserQuestion` | 用户提问结果 |
| `Monitor` | 监控结果 |
| `Text` | 纯文本输出 |
| `ImageGen` | 图像生成结果 |
| `ImageToVideo` | 图转视频结果 |
| `ReferenceToVideo` | 参考转视频结果 |
| `ImageEdit` | 图像编辑结果 |
| `Dynamic` | 动态/未识别输出 |

#### 常见 `rawOutput` 样本

**文件列表输出（ListDir）**：
```jsonc
"rawOutput": {
  "Content": {                           // 注意：DB 中实际为非 tagged 格式
    "absolute_root_path": "/Users/.../webview-shell",
    "content": "- /Users/.../webview-shell/\n  - build.gradle.kts\n  ..."
  }
}
```

> **注意**：真实 DB 数据中 `rawOutput` 的 wire 格式可能不严格匹配源码中的 tagged union。
> DB 中观察到的格式是 `{"Content": { absolute_root_path, content }}`，这是一个 key-value 形式，
> 可能是上游序列化与 `ToolOutput` tagged union 之间的适配层差异。测试时需容错处理。

### 3.4 `content` 块（SearchReplace 的 diff）

编辑类工具的 `content` 数组携带结构化 diff：

```jsonc
"content": [
  {
    "_meta": { "new_line": 577, "old_line": 577 },
    "newText": "    private fun handleSystemBack() {\n        ...",
    "oldText": "    private fun handleSystemBack() {\n        ..."
  }
]
```

### 3.5 `content` 块（Bash 的描述）

执行类工具的 `content` 数组：

```jsonc
"content": [
  {
    "content": { "text": "Compile webview-shell and capacitor-app", "type": "text" },
    "type": "content"
  }
]
```

### 3.6 `kind` 映射（`x.ai/tool.kind` → `update.kind`）

| `x.ai/tool.kind` | `update.kind` (ACP) | 说明 |
|------------------|---------------------|------|
| `read` | `read` | 文件读取 |
| `search` | `search` | grep 搜索 |
| `edit` | `edit` | 文件编辑 |
| `execute` | `execute` | 终端命令 |
| `list_dir` / `list` | `other` | 目录列表（ACP 无对应，降级为 `other`） |
| `web_fetch` | `fetch` | 网页抓取 |
| `plan` | `think` | 计划更新（ACP `think`） |
| `web_search` | `other` | 网页搜索（ACP 无对应） |
| `write` | `other` | 文件写入（ACP 无对应） |
| `task` | `other` | 子 agent（ACP 无对应） |
| `skill` | `other` | Skill（ACP 无对应） |

> **源码依据**：`update.kind` 使用 ACP upstream `ToolKind`（`agent-client-protocol-schema/src/tool_call.rs:380`），
> 该枚举只有 `read, edit, delete, move, search, execute, think, fetch, switch_mode, other`。
> `x.ai/tool.kind` 使用 xAI 扩展 `ToolKind`（32 个值），映射时自动降级到 ACP 枚举。

### 3.7 ACP 标准 `ToolCall` / `ToolCallUpdate` 完整结构

> **源码**：`agent-client-protocol-schema-0.11.4/src/tool_call.rs:24`

#### `ToolCall`（对应 `sessionUpdate: "tool_call"`）

```rust
pub struct ToolCall {
    pub tool_call_id: ToolCallId,         // string
    pub title: String,
    pub kind: ToolKind,                   // ACP 枚举，default: Other
    pub status: ToolCallStatus,           // default: Pending
    pub content: Vec<ToolCallContent>,    // diff/terminal/content 块
    pub locations: Vec<ToolCallLocation>, // 文件位置
    pub raw_input: Option<Value>,         // 工具原始入参
    pub raw_output: Option<Value>,        // 工具原始输出
    pub _meta: Option<Meta>,              // xAI 扩展（含 x.ai/tool）
}
```

#### `ToolCallUpdate`（对应 `sessionUpdate: "tool_call_update"`）

```rust
pub struct ToolCallUpdate {
    pub tool_call_id: ToolCallId,
    pub fields: ToolCallUpdateFields,     // flatten 到 update 顶层
    pub _meta: Option<Meta>,
}

pub struct ToolCallUpdateFields {
    pub kind: Option<ToolKind>,
    pub status: Option<ToolCallStatus>,
    pub title: Option<String>,
    pub content: Option<Vec<ToolCallContent>>,
    pub locations: Option<Vec<ToolCallLocation>>,
    pub raw_input: Option<Value>,
    pub raw_output: Option<Value>,
}
```

#### `ToolCallStatus` 枚举

> **源码**：`tool_call.rs:420`

```rust
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus { Pending, InProgress, Completed, Failed }
```

wire 值：`pending`, `in_progress`, `completed`, `failed`

> **注意**：DB 中 `_meta.updateParams.status` 使用 PascalCase（`"Pending"`, `"Completed"`），
> 而 `update` 内的 status（如果有）使用 snake_case。这是 ACP 规范 vs xAI `_meta` 扩展的命名差异。

#### `ToolCallContent` 枚举（diff/terminal/content 块）

> **源码**：`tool_call.rs:450`

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCallContent {
    Content(Content),     // { content: ContentBlock (text/image/...) }
    Diff(Diff),           // { path, old_text?, new_text, _meta? }
    Terminal(Terminal),   // 终端输出块
}
```

#### `Diff` 结构（编辑类工具的 content 块）

> **源码**：`tool_call.rs:560`

```rust
pub struct Diff {
    pub path: PathBuf,
    pub old_text: Option<String>,
    pub new_text: String,
    pub _meta: Option<Meta>,    // 可含 { old_line, new_line } 行号信息
}
```

#### `ToolCallLocation` 结构

> **源码**：`tool_call.rs:615`

```rust
pub struct ToolCallLocation {
    pub path: PathBuf,
    pub line: Option<u32>,      // 1-based 行号
    pub _meta: Option<Meta>,
}
```

---

## 4. `_x.ai/session_notification` 子类型

这些是 xAI 对 ACP 的私有扩展，类似 Codex 的 approval 请求。

### 4.1 `pending_interaction` — 权限请求

> **源码**：`crates/codegen/xai-grok-shell/src/session/pending_interaction.rs:36`

```jsonc
{
  "kind": "acp_notification",
  "method": "_x.ai/session_notification",
  "params": {
    "sessionId": "019f8870-...",
    "update": {
      "kind": "permission",              // PendingKind 枚举（见下）
      "sessionUpdate": "pending_interaction",
      "tool_call_id": "call-39dc3e70-...-0"
    }
  }
}
```

**`PendingKind` 枚举**（`pending_interaction.rs:36`）：

| wire 值 | 说明 | 对应审批类型 |
|---------|------|-------------|
| `permission` | 工具执行权限 | `request_permission` |
| `question` | 用户提问 | `x.ai/ask_user_question` |
| `plan_approval` | 计划审批 | `x.ai/exit_plan_mode` |

**与 Codex 差异**：Codex 的 approval 是独立 JSON-RPC server→client request（`commandExecution/requestApproval`），携带 `{ threadId, turnId, itemId }`；Grok 的审批是一个 notification，只携带 `tool_call_id`，具体工具详情需要回查之前的 `tool_call` 事件。

### 4.2 `interaction_resolved` — 权限已处理

```jsonc
"update": {
  "sessionUpdate": "interaction_resolved",
  "tool_call_id": "call-39dc3e70-...-0"
}
```

### 4.3 `tool_call_delta_chunk` — 工具流式增量

```jsonc
"update": {
  "name": "read_file",
  "sessionUpdate": "tool_call_delta_chunk",
  "tool_call_id": "call-39dc3e70-...-0",
  "tool_index": 0
}
```

### 4.4 `turn_completed` — Turn 完成（含 usage）

> **源码**：`crates/codegen/xai-grok-shell/src/extensions/notification.rs:48` — `PromptUsage` + `PromptUsageModel`

```jsonc
{
  "kind": "acp_notification",
  "method": "_x.ai/session_notification",
  "params": {
    "_meta": { "eventId": "019f8870-...-3449", "agentTimestampMs": 1784700657779 },
    "sessionId": "019f8870-...",
    "update": {
      "prompt_id": "3f323f60-...",
      "sessionUpdate": "turn_completed",
      "stop_reason": "end_turn",         // "end_turn" | "cancelled"
      "usage": {
        // PromptUsageModel 平铺字段（汇总）：
        "apiDurationMs": 155950,
        "cachedReadTokens": 376576,
        "costUsdTicks": 2842748000,      // 成本（1e10 ticks = $1 USD）
        "costIsPartial": false,          // 成本是否部分统计
        "inputTokens": 432353,
        "modelCalls": 10,
        "outputTokens": 9958,
        "reasoningTokens": 6134,
        "totalTokens": 442311,
        // PromptUsage 顶层字段：
        "modelUsage": {                  // per-model 分解
          "grok-4.5-build": { ...同 PromptUsageModel }
        },
        "numTurns": 10,
        "usageIsIncomplete": false       // usage 是否不完整
      }
    }
  }
}
```

**`PromptUsageModel` 完整字段**（`notification.rs:62`）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `inputTokens` | u64 | 输入 token |
| `outputTokens` | u64 | 输出 token |
| `totalTokens` | u64 | 总 token |
| `cachedReadTokens` | u64 | 缓存读取 token |
| `reasoningTokens` | u64 | 推理 token |
| `modelCalls` | u64 | 模型调用次数 |
| `apiDurationMs` | u64 | API 耗时（毫秒） |
| `costUsdTicks` | i64? | 成本（ticks，1e10 ticks = $1） |
| `costIsPartial` | bool | 成本是否部分统计 |

### 4.5 `session_summary_generated` — 会话摘要

```jsonc
"update": {
  "sessionUpdate": "session_summary_generated",
  "session_summary": "WebView返回事件处理逻辑审查"
}
```

---

## 5. Minos 合成事件

### 5.1 `user_message`（Minos 合成）

Minos 在用户发消息时生成的信封，非 grok 原生产：

```jsonc
{
  "kind": "user_message",
  "messageId": "e112a121-0128-4785-9b63-03ae737ad3c0",
  "sessionId": "d168531d-...",            // Minos session_id
  "text": "看看当前webview页面..."
}
```

### 5.2 `acp_prompt_response`

prompt 结束信号：

```jsonc
{ "kind": "acp_prompt_response", "stopReason": "end_turn" }
```

---

## 6. `_x.ai/*` 其他通知

### 6.1 `_x.ai/models/update`

```jsonc
{
  "kind": "acp_notification",
  "method": "_x.ai/models/update",
  "params": {
    "availableModels": [
      {
        "modelId": "grok-4.5",
        "name": "Grok 4.5",
        "description": "SpaceXAI's new frontier model",
        "_meta": {
          "agentType": "grok-build-plan",
          "reasoningEffort": "high",
          "supportsReasoningEffort": true,
          "totalContextTokens": 500000,
          "reasoningEfforts": [
            { "id": "high", "label": "High Effort", "value": "high", "default": true, "description": "..." },
            { "id": "medium", "label": "Medium Effort", "value": "medium", "default": false, "description": "..." },
            { "id": "low", "label": "Low Effort", "value": "low", "default": false, "description": "..." }
          ]
        }
      }
    ],
    "currentModelId": "grok-4.5"
  }
}
```

### 6.2 `_x.ai/mcp/init_progress`

```jsonc
{
  "kind": "acp_notification",
  "method": "_x.ai/mcp/init_progress",
  "params": { "connected": 0, "sessionId": "019f8870-...", "total": 6 }
}
```

### 6.3 `_x.ai/session/prompt_complete`

> **源码**：`crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs:2383`

```jsonc
{
  "kind": "acp_notification",
  "method": "_x.ai/session/prompt_complete",
  "params": {
    "promptId": "3f323f60-...",
    "sessionId": "019f8870-...",
    "stopReason": "end_turn",
    "agentResult": null,
    "turnId": 1,                    // 可选，turn 序号
    "cancelTrigger": "..."          // 可选，取消触发原因
  }
}
```

---

## 7. Grok ↔ Codex 概念映射总表

| 概念 | Grok | Codex v2 |
|------|------|---------|
| 会话标识 | `sessionId`（grok CLI 内部 ID） | `threadId` |
| Turn 标识 | `promptId`（在 `_meta` 中） | `turnId`（独立字段） |
| 消息项标识 | 无（由 `streamStartMs` 推断边界） | `itemId`（每条消息有唯一 ID） |
| 助手文本流 | `session/update` → `agent_message_chunk` | `item/agentMessage/delta` |
| 推理文本流 | `session/update` → `agent_thought_chunk` | `item/reasoning/textDelta` + `summaryTextDelta` |
| 用户消息 | `session/update` → `user_message_chunk` | `item/started` → `UserMessage` |
| 工具调用开始 | `session/update` → `tool_call` | `item/started` → `CommandExecution`/`FileChange`/`McpToolCall` |
| 工具调用更新/完成 | `session/update` → `tool_call_update` | `item/completed` + `outputDelta` 通知 |
| 工具入参 | `update.rawInput` + `update._meta.x.ai/tool.input` | `ThreadItem.arguments` / `command` |
| 工具输出 | `update.rawOutput` | `ThreadItem.aggregatedOutput` / `result` |
| 审批请求 | `_x.ai/session_notification` → `pending_interaction`（notification） | `commandExecution/requestApproval`（server request） |
| Turn 结束 | `_x.ai/session_notification` → `turn_completed` | `turn/completed` |
| 计划 | `session/update` → `plan`（快照式 entries） | `ThreadItem` → `plan` + `item/plan/delta`（增量） |
| Token usage | `turn_completed` → `usage` | `Turn` → `tokens` / 不在事件流中 |
| 寻址模型 | 无 itemId，靠 `_meta.streamStartMs` + `toolCallId` | `threadId` + `turnId` + `itemId` 三元组 |

---

## 8. 单元测试注意事项

1. **消息边界推断**：Grok 没有显式的 `item/started` / `item/completed`，翻译器必须通过 `streamStartMs` 变化来检测新消息边界。测试需覆盖：同一 stream 内连续 chunk、stream 切换、tool_call 后新 chunk。

2. **tool_call vs tool_call_update 时序**：`tool_call_update` 可能在 `tool_call` 之前到达（orphan race），翻译器有 `orphan_updates` 缓存。测试需覆盖两种到达顺序。

3. **kind 降级**：`x.ai/tool.kind` 和 `update.kind` 不总是 1:1（如 `list` → `other`，`plan` → `think`）。测试需覆盖每种 kind 的映射。

4. **rawInput variant**：每种工具有不同的 `variant` 值和字段集。测试 fixture 需覆盖所有 variant。

5. **审批 notification vs request**：Grok 的审批是 notification（单向），Codex 是 server request（需 response）。翻译器不应尝试回复 grok 的 notification。

6. **_meta 多变性**：同一 `session/update` 的 `_meta` 字段在不同子类型间差异巨大。测试时不能假设 `_meta` 的字段总是存在。
