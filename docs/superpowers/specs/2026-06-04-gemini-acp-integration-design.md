# Gemini CLI ACP Integration Design

> Date: 2026-06-04
> Status: Approved
> Author: Minos Team

## 1. Background

Minos host 侧已完整支持 Codex (app-server WS)、初步支持 OpenCode (HTTP+SSE) 与 Claude Code (ndjson)。Gemini CLI 目前仅有 `minos-ui-protocol/src/gemini.rs` stub 返回 `NotImplemented`，host 通过 `PtyAgent` spawn `gemini` 但仅捕获 raw stdout 行。

Gemini CLI 自 v0.40+ 支持 **ACP (Agent Client Protocol)** 模式 (`gemini --acp`)，提供 JSON-RPC 2.0 over stdio 的结构化协议，包含完整的 session 管理、tool approval 和 file system proxy。ACP 是开放标准，有官方 Rust SDK 和 JSON Schema。

## 2. Decision: ACP as Primary Integration

选择 ACP 模式作为 Gemini CLI 集成的主要方式，理由：

| 维度 | ACP 模式 | Headless stream-json |
|------|----------|---------------------|
| 协议 | JSON-RPC 2.0 (ACP v1 标准) | 自定义 JSONL 事件 |
| Session | session/new + session/load + session/resume | 无持久 session |
| 审批 | session/request_permission (结构化) | 无结构化审批 |
| 文件代理 | fs/read_text_file, fs/write_text_file | 无 |
| 类型安全 | 官方 schema.json + Rust SDK | 手写解析 |
| 可复用 | 其他 ACP agent 可复用协议 crate | 不可复用 |

Headless stream-json 作为 fallback 保留，当 gemini 版本不支持 `--acp` 时降级使用。

## 3. Architecture

### 3.1 New Crate: minos-acp-protocol

与 `minos-codex-protocol` 平级，封装 ACP v1 全部类型。

```
minos-acp-protocol/
  Cargo.toml
  src/
    lib.rs                  — 公共导出
    types.rs                — 基础类型 (SessionId, ContentBlock, StopReason, ToolCallKind, ...)
    client_request.rs       — Client→Agent 请求 trait + 类型
    client_notification.rs  — Client→Agent 通知 trait + 类型
    server_request.rs       — Agent→Client 请求类型 (request_permission, fs/*)
    server_notification.rs  — Agent→Client 通知类型 (session/update 及所有变体)
    jsonrpc.rs              — JSON-RPC 2.0 帧封装/解析
  tests/
    round_trip.rs           — 序列化/反序列化 round-trip 测试
```

### 3.2 Core Trait Pattern

复用 `minos-codex-protocol` 的 typed request 模式：

```rust
pub trait AcpClientRequest {
    const METHOD: &'static str;
    type Response: serde::de::DeserializeOwned;
}

pub trait AcpClientNotification {
    const METHOD: &'static str;
}
```

### 3.3 Key Types

#### Client → Agent Requests

| Method | Request | Response |
|--------|---------|----------|
| `initialize` | `InitializeParams` (protocolVersion, clientCapabilities, clientInfo) | `InitializeResponse` (agentCapabilities, authMethods, agentInfo) |
| `authenticate` | `AuthenticateParams` (methodId) | `AuthenticateResponse` |
| `session/new` | `NewSessionParams` (cwd, mcpServers, additionalDirectories) | `NewSessionResponse` (sessionId, modes, configOptions) |
| `session/load` | `LoadSessionParams` (sessionId, cwd, mcpServers) | `LoadSessionResponse` |
| `session/resume` | `ResumeSessionParams` (sessionId, cwd, mcpServers) | `ResumeSessionResponse` |
| `session/prompt` | `PromptParams` (sessionId, prompt: ContentBlock[]) | `PromptResponse` (stopReason) |
| `session/close` | `CloseSessionParams` (sessionId) | `CloseSessionResponse` |
| `session/set_mode` | `SetSessionModeParams` (sessionId, modeId) | `SetSessionModeResponse` |
| `session/set_config_option` | `SetConfigOptionParams` (sessionId, configId, value) | `SetConfigOptionResponse` |
| `session/list` | `ListSessionsParams` (cwd, cursor) | `ListSessionsResponse` (sessions, nextCursor) |
| `logout` | `LogoutParams` | `LogoutResponse` |

#### Client → Agent Notifications

| Method | Params |
|--------|--------|
| `session/cancel` | `{ sessionId }` |

#### Agent → Client Requests (server-initiated)

| Method | Request | Response |
|--------|---------|----------|
| `session/request_permission` | `{ sessionId, toolCall, options }` | `{ outcome }` |
| `fs/read_text_file` | `{ sessionId, path, line, limit }` | `{ content }` |
| `fs/write_text_file` | `{ sessionId, path, content }` | `{}` |
| `terminal/create` | `{ sessionId, command, args, cwd, env }` | `{ terminalId }` |
| `terminal/output` | `{ sessionId, terminalId }` | `{ output, exitStatus, truncated }` |
| `terminal/release` | `{ sessionId, terminalId }` | `{}` |
| `terminal/wait_for_exit` | `{ sessionId, terminalId }` | `{ exitCode, signal }` |
| `terminal/kill` | `{ sessionId, terminalId }` | `{}` |

#### Agent → Client Notifications (session/update variants)

| sessionUpdate | Fields | Maps to UiEventMessage |
|---|---|---|
| `agent_message_chunk` | content: ContentBlock | TextDelta / ReasoningDelta |
| `tool_call` | toolCallId, title, kind, status | ToolCallPlaced |
| `tool_call_update` | toolCallId, status, content | ToolCallCompleted (when completed) |
| `plan` | entries[] | Raw { kind: "gemini/plan" } |
| `thought` | content | ReasoningDelta |
| `current_mode_update` | currentMode | Raw { kind: "gemini/mode_change" } |
| `available_commands_update` | commands[] | Raw { kind: "gemini/commands_update" } |
| `session_info_update` | session info | Raw { kind: "gemini/session_info" } |

#### ContentBlock

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Image { data: String, mime_type: String },
    Audio { data: String, mime_type: String },
    Resource { resource: ResourceContent },
    ResourceLink { uri: String, name: Option<String> },
}
```

#### StopReason

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}
```

#### ToolCallUpdate

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolCallUpdate {
    pub tool_call_id: String,
    pub title: Option<String>,
    pub kind: ToolCallKind,
    pub status: ToolCallStatus,
    pub content: Option<Vec<ToolCallContent>>,
}

#[serde(rename_all = "snake_case")]
pub enum ToolCallKind { Edit, Diff, Terminal, Other }

#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus { Pending, InProgress, Completed, Cancelled }
```

### 3.4 AcpClient (stdio pump)

架构与 `CodexClient` 相同 (Option C — single-task writer)：

```
AcpClient
  ├── outbound_tx: mpsc::Sender<Outbound>
  ├── inbound_rx: Arc<Mutex<mpsc::Receiver<Inbound>>>
  └── pump_task: JoinHandle
```

关键差异于 CodexClient：
1. **传输层**：stdio (newline-delimited JSON) 而非 WebSocket
2. **包含 `jsonrpc: "2.0"` 字段**：ACP 严格遵循 JSON-RPC 2.0
3. **Client 是主动方**：Minos 发起 initialize/session/new/session/prompt
4. **Agent 可发起请求**：session/request_permission 需要 Client 回复 result

pump 核心循环：

```
loop {
    select! {
        outbound = outbound_rx.recv() => {
            // 序列化 JSON-RPC 帧，写入 child stdin
        }
        line = stdout_lines.next() => {
            // 解析 JSON-RPC 帧
            // response → dispatch to pending oneshot
            // notification → send to inbound_tx
            // server request → send to inbound_tx with id
        }
    }
}
```

### 3.5 gemini_driver.rs

```rust
pub struct GeminiAcpInstance {
    pub workspace: PathBuf,
    pub child: Mutex<Option<Child>>,
    pub client: Arc<AcpClient>,
    pub session_id: Mutex<Option<String>>,
    pub modes: Mutex<Option<SessionModeState>>,
    pub spawned_at: Instant,
    pub last_activity_at: Mutex<Instant>,
    pub crash_signal: mpsc::Sender<()>,
}
```

生命周期：
1. `spawn` — 启动 `gemini --acp`，建立 AcpClient pump
2. `initialize` — 发送 initialize 请求，协商版本和能力
3. `authenticate` — 如果 Agent 要求认证
4. `new_session` — 创建 session
5. `prompt` — 发送用户消息，接收 session/update 通知流
6. `handle_permission_request` — Agent 发起审批请求时，转为 Minos approval 流
7. `cancel` — 取消当前 turn
8. `close_session` — 关闭 session
9. `shutdown` — drop AcpClient → pump 退出 → SIGTERM → SIGKILL

### 3.6 gemini.rs Translator (complete rewrite)

```rust
pub struct GeminiTranslatorState {
    thread_id: String,
    session_id: Option<String>,
    open_assistant_message_id: Option<String>,
    open_user_message_id: Option<String>,
    emitted_message_ids: HashSet<String>,
    tool_calls: HashMap<String, OpenGeminiToolCall>,
    current_plan_entries: Vec<PlanEntry>,
}
```

Translation rules:

| ACP Event | UiEventMessage |
|-----------|---------------|
| `session/update { agent_message_chunk, content: Text }` | `TextDelta` |
| `session/update { agent_message_chunk, content: Thought }` | `ReasoningDelta` |
| `session/update { tool_call }` | `ToolCallPlaced` |
| `session/update { tool_call_update, status: completed }` | `ToolCallCompleted` |
| `session/update { plan }` | `Raw { kind: "gemini/plan" }` |
| `session/update { current_mode_update }` | `Raw { kind: "gemini/mode_change" }` |
| `session/update { available_commands_update }` | `Raw { kind: "gemini/commands_update" }` |
| `session/update { session_info_update }` | `Raw { kind: "gemini/session_info" }` |
| `PromptResponse { stopReason: end_turn }` | `MessageCompleted` |
| `PromptResponse { stopReason: cancelled }` | `ThreadClosed { reason: UserStopped }` |
| `PromptResponse { stopReason: max_tokens }` | `ThreadClosed { reason: Crashed { message } }` |
| Unknown sessionUpdate variant | `Raw { kind: "gemini/{variant}" }` |

### 3.7 Approval Flow Alignment

Gemini ACP 审批与 Codex 审批完全对齐，复用 Minos backend 现有 `ApprovalService`：

```
Gemini CLI (Agent)           Minos daemon (Client)          Backend         Mobile
     │                              │                           │               │
     │ session/request_permission   │                           │               │
     │ {sessionId, toolCall, opts}  │                           │               │
     ├─────────────────────────────►│                           │               │
     │                              │ write approval_requests   │               │
     │                              ├──────────────────────────►│               │
     │                              │                           │ durable_event │
     │                              │                           ├──────────────►│
     │                              │                           │               │ UI 展示
     │                              │                           │◄──────────────┤
     │                              │  host_command             │ 用户决定       │
     │                              │◄──────────────────────────┤               │
     │  { id, result: {outcome} }   │                           │               │
     │◄─────────────────────────────┤                           │               │
```

No changes needed to backend `ApprovalService` or mobile client approval UI.

### 3.8 File System Proxy

Phase 1 不实现。`clientCapabilities.fs.readTextFile` 和 `clientCapabilities.fs.writeTextFile` 设为 `false`。Gemini CLI 将直接访问 host 文件系统。

未来如需实现，Minos 作为 ACP Client 响应 `fs/read_text_file` 和 `fs/write_text_file` 请求，增加审计和审批能力。

## 4. Error Handling

| Scenario | Handling |
|----------|----------|
| `gemini --acp` spawn fail | `MinosError::GeminiConnectFailed`, retry 15×200ms |
| ACP version mismatch | Log warn + close + `UiEventMessage::Error { code: "acp_version_mismatch" }` |
| Auth failure | `UiEventMessage::Error { code: "gemini_auth_failed" }` |
| session/prompt error response | Map to `UiEventMessage::Error { code, message }` |
| Agent process crash | pump reads EOF → `Inbound::Closed` → `ThreadClosed { reason: Crashed }` |
| JSON-RPC error | `MinosError::AcpProtocolError { method, message }` |
| Unknown sessionUpdate variant | `UiEventMessage::Raw { kind: "gemini/{variant}" }` fallback |

## 5. CLI Detection

`minos-cli-detect` 已包含 `gemini` 在 `AgentName::all()` 中。无需改动探测逻辑，`--version` 探测已正常工作。

需新增：检测 `gemini --acp` 支持。方案：在 `AgentDescriptor` 中新增 `supports_acp: bool` 字段，通过尝试 `gemini --acp --help` 或检查版本号 >= 0.40 来判断。

## 6. Testing Strategy

1. **minos-acp-protocol**: Unit tests — all ACP JSON type serialize/deserialize round-trip, validated against ACP schema.json
2. **AcpClient pump**: Unit tests — in-memory duplex pipe simulating Agent, same pattern as CodexClient tests
3. **gemini.rs translator**: Unit tests — construct ACP notification JSON, verify translation output, same pattern as claude.rs/codex.rs tests
4. **Integration tests**: E2E with `gemini --acp` (requires `GEMINI_API_KEY`, CI optional)

## 7. Implementation Phases

### Phase 1 — Minimum Viable (~3-4 days)

- `minos-acp-protocol` crate skeleton: InitializeParams/Response, NewSessionParams/Response, PromptParams/Response, CancelNotification, session/update types
- `AcpClient` stdio pump (reuse CodexClient architecture)
- `gemini_driver.rs`: spawn → initialize → session/new → prompt → read session/update
- `gemini.rs` translator: agent_message_chunk → TextDelta, tool_call → ToolCallPlaced/Completed
- Basic approval: session/request_permission → approval_requests

### Phase 2 — Full ACP v1 Events (~2-3 days)

- All session/update variant translations: plan, thought, mode_change, commands_update, session_info
- session/load for resuming sessions
- session/close graceful shutdown
- session/set_config_option, session/set_mode
- Error handling coverage

### Phase 3 — Production Hardening (~2 days)

- Complete unit test suite
- Integration test (optional, requires API key)
- ACP version negotiation tolerance
- Authentication flow (authenticate method)
- Performance tuning (stdin/stdout buffer size, pump channel capacity)
- ACP support detection in cli-detect

## 8. File Changes Summary

| File | Action | Description |
|------|--------|-------------|
| `crates/minos-acp-protocol/Cargo.toml` | New | Crate manifest |
| `crates/minos-acp-protocol/src/lib.rs` | New | Public exports |
| `crates/minos-acp-protocol/src/types.rs` | New | ACP base types |
| `crates/minos-acp-protocol/src/client_request.rs` | New | Client→Agent request types |
| `crates/minos-acp-protocol/src/client_notification.rs` | New | Client→Agent notification types |
| `crates/minos-acp-protocol/src/server_request.rs` | New | Agent→Client request types |
| `crates/minos-acp-protocol/src/server_notification.rs` | New | Agent→Client notification types |
| `crates/minos-acp-protocol/src/jsonrpc.rs` | New | JSON-RPC 2.0 framing |
| `crates/minos-acp-protocol/tests/round_trip.rs` | New | Round-trip tests |
| `crates/minos-agent-runtime/src/gemini_driver.rs` | New | Gemini ACP instance management |
| `crates/minos-agent-runtime/src/acp_client.rs` | New | ACP JSON-RPC stdio client |
| `crates/minos-agent-runtime/src/lib.rs` | Modify | Add gemini_driver, acp_client modules |
| `crates/minos-ui-protocol/src/gemini.rs` | Rewrite | Full ACP → UiEventMessage translation |
| `crates/minos-ui-protocol/src/lib.rs` | Modify | Export GeminiTranslatorState |
| `Cargo.toml` (workspace) | Modify | Add minos-acp-protocol member |
| `crates/minos-domain/src/lib.rs` | Modify | Add AcpProtocolError variant to MinosError |

## 9. Open Items

- ACP is currently labeled "experimental" in Gemini CLI; monitor stabilization progress
- File system proxy deferred to future phase
- Headless stream-json fallback not implemented (low priority, ACP is preferred)
- A2A server mode not in scope (multi-agent orchestration, not client integration)
