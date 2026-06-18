# 移动端 (apps/mobile + minos-mobile) 架构文档

> 本文档详细描述 Flutter 移动端应用和 `minos-mobile` Rust crate 的架构。

## 概述

Minos 移动端由 Flutter 应用（Dart UI）和 Rust 核心（通过 flutter_rust_bridge v2 桥接）组成。采用严格的四层架构：UI → Application → Data → Domain。

**Flutter 源码路径**: `apps/mobile/`
**Rust crate 路径**: `crates/minos-mobile/`

## Flutter 应用结构

### 目录布局

```
lib/
  main.dart                          # 入口
  architecture.dart                  # 架构文档（dartdoc）
  domain/                            # 纯 Dart 模型 + 协议
    auth_state.dart                  # 认证状态机
    active_session.dart              # Agent 会话状态机
    agent_profile.dart               # Agent 配置文件
    minos_core_protocol.dart         # 抽象 Dart 契约（~60 方法）
  infrastructure/                    # 具体实现
    minos_core.dart                  # FRB 实现（599 行）
    secure_pairing_store.dart        # iOS Keychain 持久化
  data/                              # 仓库 + 服务
    repositories/                    # 各功能仓库
    services/                        # 服务
  application/                       # Riverpod providers / ViewModels
    auth_provider.dart               # 认证状态
    active_session_provider.dart     # Agent 会话
    thread_events_provider.dart      # 线程事件
    minos_providers.dart             # 连接/配对/设备
    project_providers.dart           # 项目
    social_providers.dart            # 社交
    thread_commands.dart             # 命令门面
    root_route_decision.dart         # 根路由决策
  ui/                                # 功能组织 UI
    core/widgets/                    # 共享交互组件（审批、Agent question sheet 等）
    features/
      shell/                         # 应用壳（Tab 导航）
      chat/                          # Agent 对话
      pairing/                       # QR 配对
      social/                        # 社交（好友/对话）
      projects/                      # 项目
      profile/                       # 个人资料
  src/rust/                          # 自动生成的 FRB 代码
```

### 状态管理（Riverpod）

使用 Riverpod（`riverpod_generator` 代码生成方式）。

#### 核心 Provider

| Provider | 类型 | 用途 |
|----------|------|------|
| `authControllerProvider` | `@Riverpod(keepAlive: true)` | 认证状态 |
| `activeSessionControllerProvider` | `@Riverpod(keepAlive: true)` | Agent 会话状态机 |
| `threadEventsProvider(threadId)` | `@Riverpod(keepAlive: true)` | 线程事件 |
| `connectionStateProvider` | `@Riverpod(keepAlive: true)` | 连接状态 |
| `pairedMacsProvider` | `AsyncNotifierProvider` | 已配对 Mac 列表 |
| `pairingControllerProvider` | `@riverpod` | QR 配对生命周期 |
| `projectListProvider` | `@Riverpod(keepAlive: true)` | 项目 CRUD |
| `conversationsProvider` | `AsyncNotifierProvider` | 对话列表 |
| `friendsProvider` | `AsyncNotifierProvider` | 好友列表 |

**模式**: 每个 Provider 是 `AsyncNotifier` / `Notifier`，包裹 Repository。Repository 调用 `MinosCoreProtocol`。UI 只 watch provider。

## 路由系统

### 路由表

| 路由 | 页面 | 用途 |
|------|------|------|
| `/splash` | `_SplashScreen` | 启动加载 |
| `/login` | `LoginPage` | 登录/注册 |
| `/` | `AppShellPage` | 主壳（3 Tab） |
| `/thread/:threadId` | `ThreadViewPage` | Agent 线程对话 |
| `/thread/new` | `ThreadViewPage` | 新线程 |
| `/agent-start` | `AgentStartPage` | Agent 选择 |
| `/pairing` | `PairingPage` | QR 扫描配对 |
| `/project/:projectId` | `ProjectDetailPage` | 项目详情 |
| `/social` | `SocialHubPage` | 社交中心 |
| `/social/chat/:conversationId` | `SocialChatPage` | 社交聊天 |

### 根路由决策 (`root_route_decision.dart`)

```
AuthBootstrapping / AuthRefreshing → splash
AuthUnauthenticated / AuthRefreshFailed → login
AuthAuthenticated + Connected → projectList
AuthAuthenticated + offline → projectListOffline
```

### App Shell（3 个 Tab）

- **Tab 0 (消息)**: `SocialHubPage` — 对话列表 + 未读数
- **Tab 1 (Agents)**: `AgentsHubTab` — Agent 配置管理
- **Tab 2 (我的)**: 个人信息、社交功能、开发工具、登出、配对

## Rust FFI 桥接

### 架构（依赖反转）

```
Domain (MinosCoreProtocol abstract class)
  ^           ^
  |           |
  |    Infrastructure (MinosCore implements MinosCoreProtocol)
  |           |
  |    Generated FRB API (MobileClient opaque class)
  |           |
  v           v
Rust crate (minos-mobile::MobileClient)
```

### 初始化流程

1. `main.dart` 调用 `MinosCore.init(selfName: 'iPhone', logDir: logDir)`
2. `MinosCore.init` 调用 `RustLib.init()` 加载动态库
3. `resolveClient()` 检查 `SecurePairingStore` 中的持久化状态
4. 如有持久化认证，尝试 `refreshSession()` 然后 `resumePersistedSession()`
5. 否则创建新的 `MobileClient`
6. `MinosCore` 实例通过 `minosCoreServiceProvider.overrideWithValue` 注入

### FRB Surface (`MobileClient`)

`MobileClient` 暴露约 80 个异步方法:

- **认证**: `register`, `login`, `refreshSession`, `logout`, `subscribeAuthState`
- **配对**: `pairWithQrJson`, `forgetHost`, `listPairedHosts`, `activeHost`, `setActiveHost`
- **社交**: `conversations`, `sendChatMessage`, `friends`, `friendRequests`, `searchUsers`
- **项目**: `createProject`, `listProjects`, `updateProject`, `deleteProject`
- **线程**: `listThreads`, `readThread`, `sendUserMessage`, `interruptThread`, `closeThread`
- **Agent 请求**: `sendApprovalDecision`, `respondOpencodeQuestion`
- **实时**: `subscribeState`, `subscribeUiEvents`, `subscribeSocialEvents`
- **生命周期**: `notifyForegrounded`, `notifyBackgrounded`

## 认证流程

### 状态机

```
AuthBootstrapping → AuthUnauthenticated → AuthAuthenticated
                      ^                       |  |
                      |    AuthRefreshFailed ←-+  |
                      |                          |
                      +---- AuthRefreshing ------+
```

### 流程

1. **登录/注册**: `LoginPage` → `AuthController` → `AuthRepository` → `MinosCore.login/register` → `MobileClient.login/register`
2. Rust HTTP POST 到 `/v1/auth/login` 或 `/v1/auth/register`
3. 成功后存储 `AuthSession`（access_token, refresh_token, account info）
4. Rust 发布 `AuthStateFrame::Authenticated` 到 watch channel
5. Dart 的 `AuthController` 映射为 `AuthAuthenticated`
6. 首次 `Authenticated` 后启动 WebSocket

### 持久化（iOS Keychain）

- `minos.device_id` — 稳定设备标识
- `minos.access_token` — Bearer token
- `minos.access_expires_at_ms` — Token 过期时间
- `minos.refresh_token` — 刷新令牌
- `minos.account_id` — 账户 UUID
- `minos.account_email` — 账户邮箱

所有 5 个认证字段必须同时存在或同时缺失（原子元组）。

## 配对流程

1. 用户在 Profile Tab 点击 "添加伙伴" → 导航到 `/pairing`
2. 检查相机权限
3. QR 扫描器（`mobile_scanner`）读取 QR payload
4. 解析 v2 JSON（`host_display_name`, `pairing_token`, `expires_at_ms`）
5. 确认界面显示检测到的信息
6. 用户确认 → `MobileClient.pairWithQrJson()` → HTTP POST `/v1/pairing/confirm`
7. 后端创建 `account_host_pairings` 行
8. 保存设备 ID + 活跃 host → 打开认证 WebSocket
9. 发布 `ConnectionState::Connected`

## Agent 会话管理

### 状态机

```
SessionIdle --send()--> SessionSending --first UI frame--> SessionStreaming
                         |                                            |
                         +--error--> SessionError                     |
                                                                      v
                    SessionStreaming --MessageCompleted--> SessionAwaitingInput
                         |                                            |
                         +--stop()/ThreadClosed--> SessionSuspended   |
                         |                                            |
                         +--error--> SessionError<--send failure------+
                         <-----------send() (follow-up)---------------+
```

### 线程事件 Provider

- 加载初始页: `readThread()`
- 订阅实时 `UiEventFrame` 流
- 基于 seq 水位线去重
- `keepAlive: true`，导航离开不丢失状态
- `UiEventMessage` 文本字段使用 `DisplayPayload`，Dart 通过 `display_payload_preview.dart` 渲染 inline/windowed preview；artifact 引用保留在线程归属内，后续完整展开走 range read API。
- `Raw(kind="opencode/question.asked")` 由 `ThreadViewPage` 解析为 `AgentQuestionRequestData`，通过 `agent_question_sheet.dart` 展示问题和选项；提交后走 `ThreadCommands.respondOpencodeQuestion()` → `ThreadRepository` → `MinosCore` → FRB → `MobileClient.respond_opencode_question()` → `POST /v1/agent-sessions/respond-opencode-question`。

### Agent 配置文件

`AgentProfile` 不可变模型: id, agentId, name, description, runtimeAgent, model, reasoningEffort, environmentVariables, hostDeviceId, workspacePath

持久化为 JSON 文件。

## Rust WebSocket 架构

### 核心组件

| 组件 | 文件 | 职责 |
|------|------|------|
| `MobileClient` | `client.rs` | 管理连接 |
| `RealtimeSession` | `realtime/session.rs` | WS 循环 |
| `FrameHandler` | `realtime/frame_handler.rs` | 解析 ServerFrame |
| `SubscriptionManager` | `realtime/subscription.rs` | Topic + seq cursor |
| `ReconnectController` | `reconnect.rs` | 指数退避重连 |
| `MobileHttpClient` | `http.rs` (2736 行) | REST API 客户端 |

### WS 协议

1. 连接: WS 升级带 Bearer token + 设备头
2. Hello: 接收 `conn_id` + `heartbeat_interval_ms`
3. Subscribe: 订阅 topic 带 `resume_after` cursor
4. 主循环: inbound `ServerFrame` / outbound `ClientFrame`
5. Agent realtime: backend 推送 `StreamEvent { kind: "ui_event" }`，Rust 侧反序列化为 `UiEventMessage`，再经 FRB 传给 Dart。

### 重连策略

- 指数退避: 1s → 2s → 4s → 8s → 16s → 30s 封顶
- 持续成功 60s+ 重置为 1s
- 后台 45s 宽限窗口后暂停
- 前台切换立即重置并恢复
- 连接前检查 token 过期并刷新

## Fake-Peer CLI 工具

开发工具，模拟 `ios-client`，用于无需 iPhone 的端到端测试。

| 子命令 | 用途 |
|--------|------|
| `pair` | 注册 → 配对 → 监听帧 |
| `register` | 严格注册 → 配对（`EmailTaken` 不降级） |
| `smoke-session` | 完整流程: 注册 → 配对 → `send_user_message` → 流式接收 |

## 数据流（端到端聊天消息）

```
用户在 InputBar 输入
  → ThreadCommands.sendUserMessage()
    → ThreadRepository.sendUserMessage()
      → MinosCore.sendUserMessage()
        → MobileClient.send_user_message() [Rust, via FRB]
          → MobileHttpClient.send_agent_input() [HTTP POST]

后端通过 WS 广播:
  → ServerFrame::StreamEvent (topic: agent_session:xxx)
    → RealtimeSession.run() 接收
      → handle_server_frame() → dispatch_event()
        → UiEventFrame broadcast
          → ActiveSessionController._onUiEvent() [Dart]
            → 状态机转换
          → ThreadEvents [Dart]
            → 追加事件列表
              → ThreadViewPage 渲染 DisplayPayload preview 并重建 UI
```

### Agent question 数据流

```
opencode question.asked
  → daemon projection Raw(kind="opencode/question.asked")
  → backend StreamEvent → mobile UiEventFrame
  → ThreadViewPage.showAgentQuestionSheet()
  → 用户选择/输入答案
  → ThreadCommands.respondOpencodeQuestion()
  → POST /v1/agent-sessions/respond-opencode-question
  → host command minos_respond_opencode_question
  → daemon AgentGlue → AgentManager → opencode POST /question/{requestID}/reply
```
