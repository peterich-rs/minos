# 共享 Crate 架构文档

> 本文档详细描述所有共享 crate 的职责、关键类型和依赖关系。

## Crate 总览

| Crate | 职责 | 内部依赖 |
|-------|------|---------|
| `minos-domain` | 核心域类型 | 无 |
| `minos-protocol` | 线协议定义 | domain, ui-protocol |
| `minos-transport` | 传输层 | domain, protocol |
| `minos-pairing` | 配对状态机 | domain |
| `minos-cli-detect` | CLI agent 检测 | domain |
| `minos-agent-runtime` | Agent 运行时 | domain, codex-protocol, acp-protocol, chat-store |
| `minos-chat-store` | 聊天持久化 | domain, protocol |
| `minos-acp-protocol` | ACP 协议类型 | 无 |
| `minos-codex-protocol` | Codex 协议类型 | 无 |
| `minos-ui-protocol` | UI 事件协议 | domain |
| `minos-ffi-uniffi` | UniFFI 绑定 shim | daemon, domain, protocol, ui-protocol, agent-runtime, pairing |
| `minos-ffi-frb` | FRB 绑定 shim | mobile, domain, protocol, ui-protocol |

---

## 1. `minos-domain` — 核心域类型

**路径**: `crates/minos-domain/`
**特性**: 纯值类型，零 I/O，无 async。几乎所有其他 crate 都依赖它。

### 关键类型

| 模块 | 类型 | 描述 |
|------|------|------|
| `ids` | `DeviceId(Uuid)`, `PairingToken(String)`, `DeviceSecret(String)` | 设备标识、配对令牌（32 字节 base64url 一次性）、设备密钥（Debug/Display 中脱敏） |
| `agent` | `AgentName` (Codex, Claude, Gemini, Opencode), `AgentStatus` (Ok, Missing, Error), `AgentDescriptor` | 支持的 CLI agent 枚举、健康状态、安装描述符 |
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
| `local_rpc` | `ListConversationMessagesParams/Response`, `AppendConversationMessageParams`, `StartAgentInConversationRequest` | TUI 本地 RPC 类型 |

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

## 4. `minos-pairing` — 配对状态机

**路径**: `crates/minos-pairing/`
**特性**: 配对状态机和持久化端口。

### 关键类型

| 类型 | 描述 |
|------|------|
| `Pairing` struct | 状态机: `begin_awaiting()`, `accept_peer()`, `forget()`, `replace()`。非法转换返回 `MinosError::PairingStateMismatch` |
| `TrustedDevice` | device_id, name, host_device_id, paired_at |
| `PairingStore` trait | `load()`/`save()` 持久化端口（iOS Keychain 实现、内存实现用于测试） |

---

## 5. `minos-cli-detect` — CLI Agent 检测

**路径**: `crates/minos-cli-detect/`
**特性**: 探测本地安装的 CLI agent。

### 关键类型

| 类型 | 描述 |
|------|------|
| `detect_all(runner)` | 探测所有四个已知 agent，返回 `Vec<AgentDescriptor>` |
| `CommandRunner` trait | `which(bin)` + `run(bin, args, timeout)` 异步 trait |
| `capture_user_shell_env()` | 运行 `$SHELL -lic 'env -0'` 捕获用户 shell 环境 |

**CLI 工具**: `minos-detect`（独立二进制）

---

## 6. `minos-agent-runtime` — Agent 运行时

**路径**: `crates/minos-agent-runtime/`
**特性**: 核心 runtime，spawn 和监督 CLI agent 子进程。

**设计原则**: 不依赖 `minos-protocol` 的 relay/local RPC schema；只依赖 `minos-ui-protocol` 的 `ArtifactRef` 作为 raw body artifact 引用类型。

### 关键类型

| 类型 | 描述 |
|------|------|
| `AgentRuntimeConfig` | runtime 配置。Codex initialize handshake 默认 5 秒，`thread/start` 默认 30 秒（`thread_start_timeout`），避免冷启动或 workspace 初始化偏慢时误判失败。Teamwork MCP command 优先使用 `MINOS_TEAMWORK_MCP_BIN` / 同目录 `minos-teamwork-mcp`，开发态可回落到 `minos-tui` 或 `minos-daemon` 的 `__minos-teamwork-mcp` hidden sidecar |
| `AgentManager` | 多工作区 agent 管理器。每个工作区一个 `AppServerInstance`，N 个 `ThreadHandle` |
| `ThreadState` | Starting, Idle, Running, Suspended, Resuming, Closed |
| `PauseReason` | UserInterrupt, CodexCrashed, DaemonRestart, InstanceReaped |
| `CloseReason` | UserClose, TerminalError |
| `RawIngest` | 原始事件转发类型，携带 `RawBody::InlineBytes` 或 `RawBody::Artifact`，不携带 `serde_json::Value` 主体 |
| `RawBody` | raw bytes / artifact ref 数据面 |
| `INLINE_RAW_BODY_THRESHOLD` | 16 KiB，大于等于该阈值由 daemon artifact store 接管 |
| `CodexClient` | Codex app-server JSON-RPC WS 客户端 |
| `ClaudeNdjsonSession` | Claude CLI NDJSON 流驱动 |
| `GeminiAcpInstance` | Gemini CLI ACP 协议驱动 |
| `OpencodeServerInstance` | Opencode CLI 驱动 |

### 支持的 Agent 驱动

- **Codex**: 通过 WebSocket 连接 codex app-server，JSON-RPC 2.0 协议
- **Claude**: 通过 NDJSON 流式协议
- **Gemini**: 通过 ACP（Agent Client Protocol）
- **Opencode**: 通过自定义协议

### 进程管理

- Unix `setpgid` 进程组隔离
- 多工作区支持
- 进程崩溃检测和恢复

---

## 7. `minos-chat-store` — 聊天持久化

**路径**: `crates/minos-chat-store/`
**特性**: SQLite 持久化群聊房间、消息、agent 会话和 MCP 命令。

### 关键类型

| 类型 | 描述 |
|------|------|
| `ChatStore` | 主结构（SqlitePool）: open, ensure_room, append_message, upsert_message_by_id, list_messages |
| `ChatRoom` | 聊天房间（room_id, title, workspace_root） |
| `ChatMessage` | 消息（seq, message_id, sender_role, event_type, text） |
| `ChatAgentSession` | Agent-房间绑定 |
| `ChatMcpCommand` | MCP 命令队列（MentionAgent/MentionUser） |
| `ChatMessagePage` | 分页响应 |

**数据库**: SQLite WAL 模式。表: chat_rooms, chat_messages, chat_agent_sessions, chat_mcp_commands。`chat_messages.message_id` 是群聊消息的稳定 upsert key；TUI 对每个 thread turn 只创建一条流式 agent result，后续增量更新原行 body，不推进 sequence。

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
| `UiEventMessage` | 15 变体枚举: ThreadOpened, ThreadClosed, MessageStarted, MessageCompleted, TextDelta, TextReplace, ReasoningDelta, ReasoningReplace, ToolCallPlaced, ToolCallCompleted, Error, Raw 等 |
| `DisplayPayload` | UI 展示载荷: `Inline`, `StreamingWindow`, `WindowedFinal`。文本 delta、tool args/output 都通过它传递 preview/artifact 信息 |
| `ArtifactRef` | artifact 归属和校验信息: `thread_id`, `artifact_id`, `size_bytes`, `sha256`, `media_type` |
| `MessageRole` | User / Assistant / System |
| `ThreadEndReason` | UserStopped / AgentDone / Crashed / Timeout / HostDisconnected |
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

## 11. `minos-ffi-uniffi` — UniFFI 绑定 Shim

**路径**: `crates/minos-ffi-uniffi/`
**特性**: 聚合 FFI surface 供 Swift 消费。产生 `cdylib`（动态）+ `staticlib`（静态）。

### 导出类型（来自上游 crate）

| 来源 | 导出类型 |
|------|---------|
| `minos-daemon` | `DaemonHandle`, observer 协议, `RelayQrPayload`, `PeerRecord`, `Subscription` |
| `minos-domain` | `AgentDescriptor`, `AgentName`, `DeviceId`, `PeerState`, `RelayLinkState` |
| `minos-agent-runtime` | `ThreadState`, `PauseReason`, `CloseReason` |
| `minos-protocol` | `AgentLaunchMode`, `StartAgentRequest`, `SendUserMessageRequest` 等 |
| `minos-ui-protocol` | `ThreadEndReason` |

### 导出函数

- `init_logging()`, `set_debug()`, `today_log_path()` — 日志控制
- `swift_log_debug/info/warn/error()` — Swift → Rust tracing 桥接
- `kind_message(kind, lang)` — 跨语言错误消息查找

---

## 12. `minos-ffi-frb` — Flutter Rust Bridge 绑定 Shim

**路径**: `crates/minos-ffi-frb/`
**特性**: `flutter_rust_bridge` v2 适配器，暴露 `minos-mobile::MobileClient` 给 Dart。

### 结构

| 组件 | 描述 |
|------|------|
| `api/minos.rs` | `#[frb(...)]` 注解函数，codegen 扫描生成 Dart API |
| `frb_generated.rs` | 自动生成（checked in，CI Dart leg 不需要 Rust toolchain） |

**依赖**: `flutter_rust_bridge = "=2.12.0"`（严格匹配 codegen 版本）
**目标**: `cdylib`（Android）、`staticlib`（iOS）、`rlib`（workspace 内测试）
