# 移动端 (apps/mobile + minos-mobile) 架构文档

> 本文档详细描述 Flutter 移动端应用和 `minos-mobile` Rust crate 的架构。

## 概述

Minos 移动端由 Flutter 应用（Dart UI）和 Rust 核心（通过 flutter_rust_bridge v2 桥接）组成。采用严格的四层架构：UI → Application → Data → Domain。

**产品角色**：纯 **Account 客户端**——`/ws/client` + Hub HTTP 发/收协作消息；**不**拨 `/ws/host`。与 Desktop Account 壳同一人类身份模型：在对话里 @人 / @bot 都是消息；@bot 由 Hub **Agent inbox** 投递到已 link 的 Host runtime，结果以 agent 气泡回时间线（[ADR 0021](adr/0021-agent-as-conversation-bot-participant.md)、[architecture-messaging.md](architecture-messaging.md)）。列表里的 Host「在线」= 该 installation 的 `/ws/host` live（设备/bot 身体），与本机 Account 连接是两层信号。

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
    agent_profile.dart               # 本地 bot cache / draft
    social_message.dart              # 协作消息模型
    minos_core_protocol.dart         # 抽象 Dart 契约（~60 方法）
  infrastructure/                    # 具体实现
    minos_core.dart                  # FRB 实现
    secure_pairing_store.dart        # iOS Keychain 持久化
    social_cache_store.dart          # SQLite cache + outbox SQL
    im_outbox_store.dart             # Outbox 策略
  data/                              # 仓库 + 服务
    cloud/                           # 纯 Dart 云控制面（exchange / hosts mapper）
    repositories/                    # 各功能仓库（含 HostsRepository / SocialRepository）
    services/                        # 服务
  application/                       # Riverpod providers / ViewModels
    auth_provider.dart               # 认证状态
    minos_providers.dart             # 连接 / linked hosts / presence
    social_providers.dart            # Timeline + Inbox + friends
    im_outbox_worker.dart            # 本地 IM outbox drain
    agent_profiles_provider.dart     # 本地 bot cache（compose 选 bot）
    group_agent_provider.dart        # participants / mention roster
    root_route_decision.dart         # 根路由决策
  ui/                                # 功能组织 UI
    theme/                           # Minos design tokens（color/spacing/radius/type）
    core/widgets/                    # 共享交互组件（toast/button/empty/surface 等）
    features/
      shell/                         # 应用壳（消息 / Hosts / 账户）
      messages/                      # Golden-path conversation inbox
      hosts/                         # Linked hosts 列表
      account/                       # 账户 / 退出登录
      auth/                          # 登录注册
      social/                        # 协作 IM 聊天 / 成员
        lib/message_grouping.dart    # 10min 分组 + day divider（对齐 Desktop）
        widgets/                     # Slack/Buzz 全宽行 chrome / row / actions
      debug/                         # 日志与请求追踪
  src/rust/                          # 自动生成的 FRB 代码（勿手改）
```

### FRB 代码生成

- **运行时 / pubspec 钉死版本**: `flutter_rust_bridge = "=2.12.0"`（`minos-ffi-frb`）与 `apps/mobile` pubspec `2.12.0`；codegen CLI 必须同版本。
- **唯一入口**: 仓库根执行 `just gen-frb` 或 `cargo xtask gen-frb`（内部调用 `flutter_rust_bridge_codegen generate --config-file flutter_rust_bridge.yaml`，CWD 为 `apps/mobile` 以便 fvm 解析 Flutter）。
- **安装**: `cargo xtask bootstrap` 安装 `flutter_rust_bridge_codegen` **2.12.0**（`--locked --force`）。
- **产物**: `apps/mobile/lib/src/rust/**` + `crates/minos-ffi-frb/src/frb_generated.rs`（checked-in；改 `api/` mirror 后必须 regen，禁止手改 encode/decode）。

### 状态管理（Riverpod）

使用 Riverpod（`riverpod_generator` 代码生成方式）。

#### 核心 Provider

| Provider | 类型 | 用途 |
|----------|------|------|
| `authControllerProvider` | `@Riverpod(keepAlive: true)` | 认证状态 |
| `connectionStateProvider` | `@Riverpod(keepAlive: true)` | 连接状态（Account `/ws/client`） |
| `pairedMacsProvider` | `AsyncNotifierProvider` | Linked hosts（`GET /v1/hosts`） |
| `hostsRepositoryProvider` | `Provider` | 纯 Dart hosts 列表 + FRB fallback |
| `conversationsProvider` | `AsyncNotifierProvider` | 对话列表（InboxSync） |
| `socialConversationProvider` | `@riverpod` family | 打开会话时间线（TimelineSync） |
| `friendsProvider` | `AsyncNotifierProvider` | 好友列表（Messages 选人） |
| `imOutboxWorkerProvider` | `Provider` | 本地 IM outbox drain |

**模式**: 每个 Provider 是 `AsyncNotifier` / `Notifier`，包裹 Repository。Repository 调用 `MinosCoreProtocol`。UI 只 watch provider。协作发送唯一路径：`SocialConversation.sendMessage` → outbox → `sendChatMessage`。

## 路由系统

### 路由表

| 路由 | 页面 | 用途 |
|------|------|------|
| `/splash` | `_SplashScreen` | 启动加载 |
| `/login` | `LoginPage` | 登录/注册 |
| `/` | `AppShellPage` | 主壳（消息 / Hosts / 账户） |
| `/social` | redirect → `/` | 旧消息入口，兼容跳转到 shell 消息 Tab |
| `/social/chat/:conversationId` | `SocialChatPage` | Conversation 协作 IM（Hub 气泡；Slack/Buzz 全宽左对齐行） |
| `/social/chat/:conversationId/members` | `GroupMembersPage` | 群成员 |
| `/log-viewer` | `LogViewerPage` | 开发者日志 |

**已下线（不再作为产品面）**：`/thread/*`、`/sessions`、`/agent-start`、`/agent-profile/*`、`/project/*`，以及 Agent session transcript / Projects / Agents Hub UI。

### 根路由决策 (`root_route_decision.dart`)

```
AuthBootstrapping / AuthRefreshing → splash
AuthUnauthenticated / AuthRefreshFailed → login
AuthAuthenticated + Connected → projectList
AuthAuthenticated + offline → projectListOffline
```

`createAppRouter` redirect only reads synchronous `authControllerProvider`.
`projectList` and `projectListOffline` both map to shell `/` (offline chrome is
in-shell). Do not seed login-page provider state from widget `initState` —
`LoginPageStateController.build` reads `AuthRefreshFailed` once when the
provider is created.

### App Shell（3 个 Tab）

- **Tab 0 (消息)**: `MessagesPage` — 全部 conversation，按 `lastMessageAtMs` 倒序；可新建 DM / 群 / agent conversation
- **Tab 1 (Hosts)**: `HostsPage` — linked hosts（bot runtime 身体）
- **Tab 2 (账户)**: `AccountPage` — profile / logout / 开发者工具

UI 使用自研 **Minos design tokens**（iOS 向手感），不再依赖 `shadcn_ui`。

### 协作 IM 时间线（`SocialChatPage`）

与 Desktop `MessageList` / `MessageChrome` 对齐的 **Slack/Buzz 全宽左对齐行**（非 iMessage 左右气泡）：

| 能力 | 实现 |
|------|------|
| 行壳 | `ConversationMessageRow` + `ConversationMessageChrome`：头像 gutter + 作者/时间 header + markdown body |
| 分组 | 同作者 10 分钟窗口隐藏 avatar/header（`message_grouping.dart`） |
| 分隔 | 日历日 `ConversationDayDivider`（今天 / 昨天 / 日期） |
| 撤回 | 居中 `ConversationSystemMessage` |
| 交互 | 长按：引用 / 复制 / 重试 / 撤回；失败红 `!` 重试；stick-to-bottom + 跳转最新 FAB |
| 发送 | 唯一主链路：outbox + `sendChatMessage`（Account WS `AppendMessage`） |
| TimelineSync | `SocialConversationState` 维护 `minLoadedSeq` / `maxLoadedSeq` / `hasOlder`；`loadOlder()` → Hub `before_seq`；近顶滚动触发 |
| InboxSync | `ConversationsController` 事件 → 单行 patch + unread bump；**禁止**每事件 `invalidateSelf` / 全量 REST |
| SnapshotRequired | Rust `UiEvent raw(kind=snapshot_required)` → `imSnapshotSyncProvider`：conversation 仅 `ref.exists` 时 range reconcile（不 cold-start / 不 mark-read）；account → inbox hydrate |
| 排序 | durable 仅 `server_order_key`（message_seq）；禁止 `COALESCE(seq, created_at_ms)` |
| parse 失败 | Rust `parse_chat_message` 返回 `None`，不入空壳气泡 |

**注意**：Mobile 不再提供 Agent session transcript 产品面。Bot 结果以 Hub conversation 气泡呈现；执行审批/transcript 若后续恢复，应做成 conversation-scoped Attention，而不是独立 `/thread` 发送主链路。

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

- **认证**: `register`, `login`, `loginWithSupabase`, `refreshSession`, `logout`, `subscribeAuthState`
- **Hosts**: `listPairedHosts`（内部 `GET /v1/hosts`）, `activeHost`, `setActiveHost`, `forgetHost`（`POST /v1/hosts/unlink`）
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

1. **登录/注册**: `LoginPage` → `AuthController` → `AuthRepository`
2. **优先路径（Supabase 已配置）**:
   - `supabase_flutter` email/password → Supabase access token
   - `MobileClient.loginWithSupabase` → `POST /v1/auth/supabase`（`X-Device-Id` + JWT body）
3. **无过渡密码路径**: 未配置 Supabase 时 AuthRepository 直接失败（`SUPABASE_URL` / `ANON_KEY` 必填）
4. 成功后存储 `AuthSession`（access_token, refresh_token, account info）到 Rust + Keychain
5. Rust 发布 `AuthStateFrame::Authenticated`；Dart 映射为 `AuthAuthenticated`
6. 首次 `Authenticated` 后 `resumePersistedSession()` 启动 `/ws/client`
7. **登出**: Minos logout + Keychain auth wipe + best-effort Supabase `signOut`

### 配置（dart-define）

| Define | 用途 |
|--------|------|
| `MINOS_BACKEND_URL` | ws(s)/http(s) hub；纯 Dart cloud client 归一化为 HTTP origin |
| `SUPABASE_URL` | Supabase project URL（必填；空则 AuthRepository 失败） |
| `SUPABASE_ANON_KEY` | Supabase anon key |

### 持久化（iOS Keychain）

- `minos.device_id` — 稳定设备标识
- `minos.access_token` — Bearer token
- `minos.access_expires_at_ms` — Token 过期时间
- `minos.refresh_token` — 刷新令牌
- `minos.account_id` — 账户 UUID
- `minos.account_email` — 账户邮箱

所有 5 个认证字段必须同时存在或同时缺失（原子元组）。

## Linked Hosts（Host Link）

QR pairing is removed (Phase D). Desktop links the Mac with the same account; Mobile only lists hosts:

1. `HostsRepository.listLinkedHosts` prefers pure Dart `GET /v1/hosts` (Keychain bearer)
2. On failure, falls back to FRB `listPairedHosts` (Rust also uses `GET /v1/hosts`)
3. `pairedMacsProvider` drives Hosts UI; empty state points users to Desktop **Link this Mac**
4. If no active host is set, auto-select the first online host (else first listed) as routing target
5. Mobile 不提供独立 Agent session 发送/transcript 产品面；bot 结果以 Hub conversation 气泡呈现
6. Forget host uses `POST /v1/hosts/unlink`

## Agent / Bot 在 Mobile 中的角色

- **Hub bot identity** 是多端 SSOT；`AgentProfile` 本地 JSON 仅作 compose 选 bot 的 cache/draft。
- Messages 可新建 agent conversation（`createAgentConversation` → 建 group + addAgent）。
- 协作发送与 @bot 都走 conversation 时间线；Host 只是 bot runtime 身体（Hosts tab 可见性）。
- Mobile 当前**不**承载 session transcript / approval / opencode question UI。若后续恢复，应做成 conversation-scoped Attention，而不是复活 `/thread` 发送主链路。

## Rust WebSocket 架构

### 核心组件

| 组件 | 文件 | 职责 |
|------|------|------|
| `MobileClient` | `client.rs` | 管理连接 |
| `RealtimeSession` | `realtime/session.rs` | WS 循环 |
| `FrameHandler` | `realtime/frame_handler.rs` | 解析 ServerFrame |
| `SubscriptionManager` | `realtime/subscription.rs` | **desired / pending / confirmed** topics + **applied** seq cursors |
| `ReconnectController` | `reconnect.rs` | 指数退避重连 |
| `MobileHttpClient` | `http.rs` (2736 行) | REST API 客户端 |

### WS 协议

1. 连接: WS 升级带 Bearer token + 设备头
2. Hello: 接收 `conn_id` + `heartbeat_interval_ms`
3. Subscribe: 对 **desired** topics 发送 Subscribe + `resume_after`（applied cursors）；发送成功 → pending；`SubscribeAck` → confirmed。发送失败保持 desired 未 confirmed，下次 desire/reconnect 会重发
4. Durable social：parse → fanout（`topic`/`topic_seq`）→ Dart SQLite/reducer commit → `ackDurableApplied` 才推进 cursor；无 subscriber / parse 失败 hold cursor
5. 主循环: inbound `ServerFrame` / outbound `ClientFrame`
6. Agent realtime: backend 推送 `StreamEvent { kind: "ui_event" }`，Rust 侧反序列化为 `UiEventMessage`，再经 FRB 传给 Dart。

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

## 数据流（端到端协作 IM 消息）

```
用户在 SocialChatPage composer 输入
  → SocialConversation.sendMessage()
      → SocialCacheStore.insertPendingMessageWithOutbox  [TX: sending + outbox]
      → UI 乐观展示
      → ImOutboxWorker.flush()
          → SocialRepository.sendChatMessage()
            → MinosCore.sendChatMessage()
              → MobileClient.send_chat_message() [Rust FRB]
                → WS ClientFrame::AppendMessage
                → wait ChatSendAck / Nack

后端 durable fanout:
  → ServerFrame::DurableEvent (conversation / account topics)
    → RealtimeSession → social_events broadcast
      → SocialConversation / ConversationsController 投影
      → Dart commit cache 后 ackDurableApplied
```