# 共享 Crate 架构文档

> 本文档详细描述所有共享 crate 的职责、关键类型和依赖关系。

## Crate 总览

| Crate | 职责 | 内部依赖 |
|-------|------|---------|
| `minos-domain` | 核心域类型 | 无 |
| `minos-protocol` | 线协议定义 | domain, ui-protocol |
| `minos-transport` | 传输层 | domain, protocol |
| `minos-cli-detect` | CLI agent 检测 | domain |
| `minos-prompt-runtime` | Session 提示词编译 + `minos.teamwork` package SSOT | 无（仅 sha2/serde） |
| `minos-agent-runtime` | Agent 运行时 | domain, codex-protocol, acp-protocol, chat-store, **prompt-runtime** |
| `minos-chat-store` | 聊天持久化 + teamwork MCP | domain, protocol, **prompt-runtime** |
| `minos-acp-protocol` | ACP 协议类型 | 无 |
| `minos-codex-protocol` | Codex 协议类型 | 无 |
| `minos-ui-protocol` | UI 事件协议 | domain |
| `minos-ffi-frb` | FRB 绑定 shim | mobile, domain, protocol, ui-protocol |

---

## 1. `minos-domain` — 核心域类型

**路径**: `crates/minos-domain/`
**特性**: 纯值类型，零 I/O，无 async。几乎所有其他 crate 都依赖它。

### 关键类型

| 模块 | 类型 | 描述 |
|------|------|------|
| `ids` | `DeviceId(Uuid)`, `PairingToken(String)`, `DeviceSecret(String)` | 设备标识、配对令牌（32 字节 base64url 一次性）、设备密钥（Debug/Display 中脱敏） |
| `agent` | `AgentName` (Codex, Claude, Gemini, Opencode, Grok), `AgentStatus`, `AgentDescriptor`, `ModelDiscovery` | 支持的 CLI agent 枚举（runtime SSOT）、健康状态、安装+能力描述符（`display_name` / `supports_model_selection` / `supports_reasoning_effort`）、模型发现策略 |
| `connection` | `ConnectionState` (Disconnected, Pairing, Connected, Reconnecting) | 连接状态 |
| `pairing_state` | `PairingState` (Unpaired, AwaitingPeer, Paired) | 配对状态机 |
| `relay_state` | `RelayLinkState`, `PeerState` | Relay 链路 + 配对双轴状态 |
| `role` | `DeviceRole` (AgentHost, MobileClient, BrowserAdmin) | 设备角色 |
| `error` | `MinosError` (40 变体), `ErrorKind`, `Lang` (Zh, En) | 统一错误类型，内置 i18n |
| `defaults` | `DEV_BACKEND_URL`, `DEV_BACKEND_LISTEN` | 开发默认常量 |

---

## 2. `minos-protocol` — 线协议定义

**路径**: `crates/minos-protocol/`
**特性**: 定义所有跨网络消息的完整 JSON-RPC 2.0 契约。

### 关键模块

| 模块 | 类型 | 描述 |
|------|------|------|
| `rpc` | `MinosRpc` trait (jsonrpsee `#[rpc]`) | 共享服务 trait: pair, health, list_clis, start_agent, send_user_message 等 |
| `messages` | 1000+ 行 DTO | 所有 RPC 方法、HTTP 端点、社交功能的请求/响应类型 |
| `envelope` | `Envelope` (Forward, Forwarded, Event, Ingest), `EventKind` | WebSocket relay 帧格式 |
| `auth` | `AuthRequest/Response`, `RefreshRequest/Response` | 认证 HTTP DTO |
| `realtime` | `ClientFrame`, `ServerFrame`, `DurableEvent` (17 变体), `RealtimeTopic` | Topic-based 实时网关线类型 |
| `local_rpc` | `ListConversationMessagesParams/Response`, `AppendConversationMessageParams`, `StartAgentInConversationRequest`, `RemoveConversationAgentParams/Response` | TUI/Desktop 本地 RPC 类型 |

---

## 3. `minos-transport` — 传输层

**路径**: `crates/minos-transport/`
**特性**: WebSocket 传输客户端和重连退避逻辑。

### 关键类型

| 类型 | 描述 |
|------|------|
| `WsClient` | `jsonrpsee::ws_client::WsClient` 的薄包装 |
| `AuthHeaders` | WS 升级请求的认证头包（X-Device-Id, X-Device-Role, X-Device-Secret, X-Device-Name） |
| `delay_for_attempt(attempt)` | 指数退避: 1s → 2s → 4s → 8s → 16s → 30s |

---

## 4. `minos-cli-detect` — CLI Agent 检测

**路径**: `crates/minos-cli-detect/`
**特性**: 探测本地安装的 CLI agent。

### 关键类型

| 类型 | 描述 |
|------|------|
| `detect_all(runner)` | 探测 `AgentName::all()` 中每个 agent，返回 `Vec<AgentDescriptor>`（能力字段由 domain 填充） |
| `CommandRunner` trait | `which(bin)` + `run(bin, args, timeout)` 异步 trait |
| `capture_user_shell_env()` | 运行 `$SHELL -lic 'env -0'` 捕获用户 shell 环境 |

**CLI 工具**: `minos-detect`（独立二进制）

---

## 5. `minos-prompt-runtime` — Session 提示词编译 + package SSOT

**路径**: `crates/minos-prompt-runtime/`  
**特性**: 纯编译器深模块，无 I/O、无 agent 进程。  
**Consumers**: `minos-agent-runtime`（compile + delivery）、`minos-chat-store`（MCP instructions）、`minos-tui`（skill install body）。

### Canonical package

```text
packages/minos.teamwork/
  package.yaml                          # id / version / schema / token budgets
  fragments/bootstrap.md                # conversation-bound system inject
  fragments/mcp_server_instructions.md  # MCP initialize.instructions
  fragments/skill/SKILL.md              # on-demand skill handbook
```

Rust 只 `include_str!` 上述 artifact；**禁止**在 manager / mcp_server / TUI 再手写第二份。

### 公开接口

| 类型 / 函数 | 描述 |
|-------------|------|
| `SessionContext` | `runtime` + `conversation_bound` + `profile_instructions` |
| `compile_session_context` | 确定性 `SessionContext` → `CompiledPromptBundle` |
| `CompiledPromptBundle` | `bootstrap` / `profile` / `system_instructions` + `PromptProvenance` |
| `PromptProvenance` | package id/version、adapter id、bootstrap/compiled digests |
| `codex_developer_instructions` | Codex `thread/start.developerInstructions` 投递面 |
| `claude_append_system_prompt` | Claude `--append-system-prompt` 投递面 |
| `grok_rules` | Grok `--rules` 投递面 |
| `TEAMWORK_BOOTSTRAP` / `TEAMWORK_MCP_SERVER_INSTRUCTIONS` / `TEAMWORK_SKILL_MD` | package fragments |
| `teamwork_package_digests()` | 规范化 fragment digests + package aggregate |

### 不变量

- **Activation 唯一真相**：`conversation_bound` 决定是否包含 teamwork bootstrap；adapter 禁止再 `if conversation_id`。
- **拼接唯一真相**：bootstrap 与 profile 的顺序/换行/空段丢弃只在 compiler 内完成。
- **Digest 确定性**：同输入 → 同 `compiled_digest`；body 相同而 runtime 不同时 digest 可不同（含 runtime id）。
- **Token budgets**：package.yaml + unit tests 锁定 bootstrap / MCP / skill 字符上限。
- **Gemini / OpenCode**：`PromptRuntime` 枚举已占位；**不**提供投递 helper，直至 Task C capability probe。

后续切片：Task C Gemini/OpenCode 真实投递；Task D `reconcile_host_packages` + session 持久化 digest。

---

## 6. `minos-agent-runtime` — Agent 运行时

**路径**: `crates/minos-agent-runtime/`
**特性**: 核心 runtime，spawn 和监督 CLI agent 子进程。

**设计原则**: 不依赖 `minos-protocol` 的 relay/local RPC schema；只依赖 `minos-ui-protocol` 的 `ArtifactRef` 作为 raw body artifact 引用类型。

### 关键类型

| 类型 | 描述 |
|------|------|
| `AgentRuntimeConfig` | runtime 配置。Codex initialize handshake 默认 5 秒，`thread/start` 默认 30 秒（`thread_start_timeout`），避免冷启动或 workspace 初始化偏慢时误判失败。Teamwork MCP command 优先使用 `MINOS_TEAMWORK_MCP_BIN` / 同目录 `minos-teamwork-mcp`，开发态可回落到 `minos-tui` 或 `minos-daemon` 的 `__minos-teamwork-mcp` hidden sidecar |
| `prompt`（内部） | `compile_for_session` 桥接到 `minos-prompt-runtime`；Codex/Claude/Grok 启动路径只消费 `CompiledPromptBundle` |
| `AgentManager` | 多工作区 agent 管理器。每个工作区一个 `AppServerInstance`，N 个 `SessionHandle` |
| `SessionState` | Starting, Idle, Running, Suspended, Resuming, Closed |
| `PauseReason` | UserInterrupt, CodexCrashed, DaemonRestart, InstanceReaped |
| `CloseReason` | UserClose, TerminalError |
| `RawIngest` | 原始事件转发类型，携带 `RawBody::InlineBytes` 或 `RawBody::Artifact`，不携带 `serde_json::Value` 主体 |
| `RawBody` | raw bytes / artifact ref 数据面 |
| `INLINE_RAW_BODY_THRESHOLD` | 16 KiB，大于等于该阈值由 daemon artifact store 接管 |
| `CodexClient` | Codex app-server JSON-RPC WS 客户端 |
| `ClaudeControlSession`（别名 `ClaudeNdjsonSession`） | Claude CLI **双向** stream-json 控制面：stdin 用户轮次 + `control_response` 审批，stdout NDJSON 事件 / `can_use_tool` |
| `GeminiAcpInstance` | Gemini CLI ACP 协议驱动 |
| `OpencodeServerInstance` | Opencode CLI 驱动 |

### 支持的 Agent 驱动

- **Codex**: 通过 WebSocket 连接 codex app-server，JSON-RPC 2.0 协议
- **Claude**: 通过 CLI stream-json 控制面（`--input-format stream-json` + 常开 stdin）；权限经 `PendingApprovalTarget::ClaudeControl` 回写
- **Gemini**: 通过 ACP（Agent Client Protocol）
- **Opencode**: 通过自定义协议

### 进程管理

- Unix `setpgid` 进程组隔离
- 多工作区支持
- 进程崩溃检测和恢复

---

## 7. `minos-chat-store` — Teamwork 持久化 + MCP sidecar

**路径**: `crates/minos-chat-store/`
**特性**: Host teamwork 委派 SQLite（与 daemon `daemon.sqlite` 同库）、Unix socket MCP handler、stdio MCP sidecar。

### 关键类型

| 类型 | 描述 |
|------|------|
| `TeamworkStore` | 委派 SSOT：create/get/complete/cancel/wait + source delivery 队列 |
| `DelegationSignalBus` | 同 DB path 进程内共享；complete/cancel 唤醒 `wait_delegation`（fallback poll 默认 2s） |
| `TeamworkDelegation` / `TeamworkDelegationStatus` | running/completed/cancelled/failed |
| `mcp_socket::SocketRequest` | sidecar↔daemon 帧请求（含 `WaitDelegation { timeout_ms }`） |
| `mcp_server` | stdio MCP：`read_timeout_for_request` 对 wait 取 `timeout_ms + 5s`；错误码 `-32001` daemon 不可用 / `-32002` socket 关闭 / `-32003` daemon 拒绝 / `-32602` 参数错误 |
| `mcp_handler::McpSocketHandler` | daemon 侧 UDS 服务 |
| `teamwork_mcp` | 工具目录与 permissions（list/delegate/wait/status/cancel/post） |

**数据库**: SQLite WAL。表: `teamwork_conversations`, `teamwork_delegations`, `teamwork_source_deliveries`。默认路径 `$MINOS_HOME/daemon.sqlite`（与 LocalStore 共用）。

---

## 8. `minos-acp-protocol` — ACP 协议

**路径**: `crates/minos-acp-protocol/`
**特性**: ACP (Agent Client Protocol) v1 JSON-RPC 的类型化 Rust 镜像。Gemini CLI 使用。

### 关键类型

| 类型 | 描述 |
|------|------|
| `SessionId`, `StopReason` | 会话标识和停止原因 |
| `ContentBlock` | Text/Image/Audio/Resource/ResourceLink |
| `ToolCallKind`, `ToolCallStatus`, `ToolCallUpdate` | 工具调用类型 |
| Client/Server 请求/通知 | 双向 JSON-RPC 消息 |

纯 `serde` + `serde_json`，无 I/O。

---

## 9. `minos-codex-protocol` — Codex 协议

**路径**: `crates/minos-codex-protocol/`
**特性**: Codex app-server JSON-RPC 协议的类型化 Rust 镜像。

### 关键类型

| 类型 | 描述 |
|------|------|
| `InitializeParams/Response` | 初始化握手 |
| `ThreadStartParams/Response` | 线程启动 |
| `SkillsListResponse` | Skills 列表 |
| `ClientRequest` / `ClientNotification` | 标记 trait 用于类型化分发 |

**代码生成**: 从 JSON Schema 通过 `typify` 工具自动生成。命令: `cargo xtask gen-codex-protocol`

---

## 10. `minos-ui-protocol` — UI 事件协议

**路径**: `crates/minos-ui-protocol/`
**特性**: 定义统一的 `UiEventMessage` 形状，供所有 UI 消费者使用。

### 关键类型

| 类型 | 描述 |
|------|------|
| `UiEventMessage` | 15 变体枚举: SessionOpened, SessionClosed, MessageStarted, MessageCompleted, TextDelta, TextReplace, ReasoningDelta, ReasoningReplace, ToolCallPlaced, ToolCallCompleted, Error, Raw 等 |
| `DisplayPayload` | UI 展示载荷: `Inline`, `StreamingWindow`, `WindowedFinal`。文本 delta、tool args/output 都通过它传递 preview/artifact 信息 |
| `ArtifactRef` | artifact 归属和校验信息: `session_id`, `artifact_id`, `size_bytes`, `sha256`, `media_type` |
| `MessageRole` | User / Assistant / System |
| `SessionEndReason` | UserStopped / AgentDone / Crashed / Timeout / HostDisconnected |
| `CodexTranslatorState` | 有状态 Codex 事件翻译器 |
| `ClaudeTranslatorState` | 有状态 Claude 事件翻译器 |
| `GeminiTranslatorState` | 有状态 Gemini 事件翻译器 |
| `GrokTranslatorState` | 有状态 Grok ACP 事件翻译器 |
| `OpencodeTranslatorState` | 有状态 Opencode 事件翻译器 |

### 翻译函数

`translate_codex()`, `translate_claude()`, `translate_gemini()`, `translate_grok()`, `translate_opencode()` — 每个 CLI agent 的原生事件 → `Vec<UiEventMessage>`

### Grok edit / tool output

Grok file edits (`SearchReplace` / write / `ApplyPatch`) arrive as ACP `ToolCallContent::Diff` plus optional structured `raw_output`. `translate_grok` converts these into unified-diff tool output (not raw `EditsApplied` JSON) so TUI/Desktop can reuse the agent-agnostic diff renderer.

---

## 11. `minos-ffi-frb` — Flutter Rust Bridge 绑定 Shim

**路径**: `crates/minos-ffi-frb/`
**特性**: `flutter_rust_bridge` v2 适配器，暴露 `minos-mobile::MobileClient` 给 Dart。

### 结构

| 组件 | 描述 |
|------|------|
| `api/minos.rs` | `#[frb(...)]` 注解函数，codegen 扫描生成 Dart API |
| `frb_generated.rs` | 自动生成（checked in，CI Dart leg 不需要 Rust toolchain） |

**依赖**: `flutter_rust_bridge = "=2.12.0"`（严格匹配 codegen CLI 版本）
**Codegen**: `just gen-frb` / `cargo xtask gen-frb`（bootstrap 安装 `flutter_rust_bridge_codegen` 2.12.0）
**目标**: `cdylib`（Android）、`staticlib`（iOS）、`rlib`（workspace 内测试）
