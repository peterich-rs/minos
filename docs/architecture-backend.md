# 后端服务 (minos-backend) 架构文档

> 本文档详细描述 `minos-backend` crate 的架构、模块划分和关键逻辑。

## 概述

`minos-backend` 是 Minos 的核心服务端，提供 HTTP REST API、WebSocket 实时网关、后台 Worker 和持久化层。支持单机模式（monolith）或拆分模式（HTTP-only / Worker-only）。

**源码路径**: `crates/minos-backend/`

### 生产部署形态（单 VPS）

| 组件 | 说明 |
|------|------|
| 公网域名 | `https://minos.ainexc.com`（Caddy 终止 TLS） |
| 进程 | 一个 monolith 容器（`MINOS_ENV=prod`） |
| 存储 | PostgreSQL 16 + Redis 7（本机 Docker，仅 loopback） |
| 媒体对象 | 可选 **Cloudflare R2**（`media_blobs` 元数据在 DB，字节在 R2；见 [ops/r2-media.md](ops/r2-media.md)） |
| 镜像 | GHCR 预构建，VPS 只 `docker pull`（见 `deploy/prod/`） |
| 运维手册 | [ops/vps-deploy.md](ops/vps-deploy.md) |

生产强制：`MINOS_STORAGE_MODE=external-sql`、`MINOS_CACHE_BACKEND=redis`、`MINOS_MESSAGE_BUS_BACKEND=redis`、非通配 `MINOS_CORS_ORIGINS`。Agent 不在 VPS 上执行。

## 启动流程 (`src/main.rs`)

1. 解析 CLI 配置（`Config::parse()`，基于 clap）
2. 校验配置（JWT secret 长度、CORS、存储模式）
3. 初始化 tracing（mars-xlog 文件日志 + stdout）
4. 连接数据库（SQLite 或 Postgres），运行 migrations
5. 构建 `RuntimeShell` → `AppContext`（所有服务的组合根）
6. 绑定 TCP 监听器，构建 Axum Router，启动 HTTP 服务
7. 优雅停机：广播 `ServerShutdown`，drain 500ms，关闭数据库

## 配置 (`src/config.rs`)

`Config` struct（clap derive，所有字段支持环境变量覆盖）：

| 字段 | 环境变量 | 默认值 |
|------|---------|--------|
| `listen` | `MINOS_BACKEND_LISTEN` | `127.0.0.1:8787` |
| `storage_mode` | `MINOS_STORAGE_MODE` | `sqlite` |
| `database_url` | `MINOS_DATABASE_URL` | (无) |
| `jwt_secret` | `MINOS_JWT_SECRET` | (必填) |
| `runtime_mode` | `MINOS_RUNTIME_MODE` | `monolith` |
| `cache_backend` | `MINOS_CACHE_BACKEND` | `in-memory` |
| `message_bus_backend` | `MINOS_MESSAGE_BUS_BACKEND` | `inline` |
| `cors_origins` | `MINOS_CORS_ORIGINS` | `*` |

媒体对象存储（环境变量，非 clap 字段；由 `MediaService::from_env` 读取）：

| 环境变量 | 说明 |
|---------|------|
| `MINOS_R2_ACCOUNT_ID` / `MINOS_R2_ACCESS_KEY_ID` / `MINOS_R2_SECRET_ACCESS_KEY` / `MINOS_R2_BUCKET` | 配置完整时使用 Cloudflare R2 |
| `MINOS_R2_ENDPOINT` | 可选；默认 `https://{account_id}.r2.cloudflarestorage.com` |
| `MINOS_MEDIA_LOCAL_DIR` | 无 R2 时的本地目录（开发） |
| `MINOS_MEDIA_MAX_BYTES` | 单对象上限，默认 10 MiB |
| `MINOS_MEDIA_PUBLIC_BASE_URL` | 下载 URL 绝对前缀（可选） |

**运行模式**：`Monolith`（HTTP + Worker 一体）、`HttpOnly`（仅 API）、`WorkerOnly`（仅后台任务）

## HTTP 路由 (`src/http/`)

### 路由表

```
/health/live          GET    存活探针
/health/ready         GET    就绪探针
/health/info          GET    版本信息
/health/jobs          GET    后台任务健康
/metrics              GET    Prometheus 指标
/openapi.json         GET    OpenAPI 规范
/ws/client            GET    WebSocket 升级（移动端/Web）
/ws/host              GET    WebSocket 升级（Host 守护进程）
/v1/auth/*            POST   认证（注册/登录/刷新/登出/改密/Supabase exchange）
/v1/pairing/*         POST   配对确认/撤销/列表
/v1/host/*            POST   Host 引导/配对码/安装令牌
/v1/agent-sessions/*  POST   Agent 会话管理
/v1/approvals/*       POST   审批请求
/v1/conversations/*   POST   对话/消息
/v1/friends/*         POST   好友/好友请求
/v1/profiles/*        POST   个人资料
/v1/projects/*        POST   项目 CRUD
/v1/realtime/*        POST   WS 票据签发
/v1/notifications/*   POST   推送令牌/偏好
/v1/media/*           GET/POST/PUT  附件 blob（R2 / 本地对象存储）
/v1/social/*          POST   Agent 注册/对话成员
```

### 中间件栈（自底向上）

1. `SetRequestIdLayer` — 分配 `x-request-id`
2. `TraceLayer` — 每请求 tracing span
3. `PropagateRequestIdLayer` — 响应中传播请求 ID
4. `record_http_metrics` — Prometheus HTTP 指标
5. CORS layer
6. `touch_account_last_seen` — 更新设备 `last_seen_at`

### Handler 状态

`BackendState` 包裹 `Arc<AppContext>`，实现 `Deref<Target = AppContext>`，所有 handler 可直接访问全部服务。

## 认证模块 (`src/auth/`)

### 子模块

| 文件 | 职责 | 关键类型 |
|------|------|---------|
| `jwt.rs` | JWT 签发/验证 | `Claims`, `sign()`, `verify()`, `sign_ws_ticket()`, `verify_ws_ticket()` |
| `bearer.rs` | Bearer token 提取 | `require()`, `require_account()` |
| `passwords.rs` | Argon2id 密码哈希 | hash/verify |
| `use_case.rs` | 认证业务逻辑 | `AuthUseCase` — supabase_exchange, refresh, logout, ws-ticket |
| `host_bootstrap.rs` | Host Ed25519 初始证明 | `BootstrapNonceStore`（Redis 或 in-memory；`GETDEL` 单次消费） |
| `host_installation.rs` | Host 安装令牌 | `hit_*` 令牌验证 |
| `realtime_ticket.rs` | WS 票据存储 | `RealtimeTicketStore` — 一次性票据 |
| `rate_limit.rs` | 速率限制 | `RateLimiter` — 固定窗口 |

### 认证流程

1. **Supabase 交换** (`POST /v1/auth/supabase`): 校验 IdP JWT → upsert account by `supabase_sub` → 签发 Minos JWT + refresh
3. **Supabase 交换** (`POST /v1/auth/supabase`): 校验 Supabase access JWT（JWKS）→ 按 `sub`/verified email 合并或创建 account → 签发 **Minos** JWT + refresh（不经 device-secret `authenticate()`）
4. **刷新** (`POST /v1/auth/refresh`): 验证 refresh token → 轮转签发新 JWT + refresh token
5. **Bearer 认证**: `Authorization: Bearer <jwt>` → `jwt::verify()` → 提取 account_id/device_id
6. **WS 票据**: Bearer 认证后签发 60s 一次性 JWT → WS 升级时消费

### 速率限制

- 注册: 3次/小时/IP
- 登录: 10次/分钟/email, 5次/分钟/IP
- 刷新: 60次/小时/account

## Host Link（同账户绑定，主路径）

**Primary path** for account↔host binding (D02). Host Link coexists until Phase D cleanup.

| Endpoint | Auth | 作用 |
|----------|------|------|
| `POST /v1/hosts/link` | account bearer + host Ed25519 proof | upsert `host_links` + 签发 `hit_*` |
| `POST /v1/hosts/unlink` | account bearer | 删 link、**始终**撤销 host tokens、kill `/ws/host`、清 peer cache |
| `GET /v1/hosts` | account bearer | 列出本账户 hosts（`online` 来自 connection registry） |

Link proof 签名载荷：`"{installation_id}:{nonce}:v1/hosts/link"`（无 leading slash）。Nonce 经 `POST /v1/host/bootstrap/nonce` 获取；多实例部署时 `BootstrapNonceStore` 走 Redis（与 `RealtimeTicketStore` 同 `MINOS_REDIS_URL`）。

`host already linked elsewhere` → **409** `host_linked_elsewhere`（Host Link 路径单 account↔host）。

实现：`http/v1/hosts.rs` + `HostLinkService::link_host` / `unlink_host`。

## 配对模块 (`src/pairing/`) — QR（遗留，Phase D 删除）

### 核心类型: `HostLinkService`

### QR 配对流程（仍挂载）

1. **Mac 请求配对码**: `POST /v1/hosts/link` → 返回配对码
2. **手机确认**: `POST /v1/hosts/link` 带配对码 → 创建 `host_links` 关联
3. **Mac 赎回**: `POST /v1/hosts/link` → 获得 `hit_*` host 安装令牌
4. **Mac 连接**: 用安装令牌签发 WS ticket → 连接 `/ws/host`

### 安全措施

- 配对码/令牌存储前 SHA-256 哈希
- Host installation token 明文仅返回一次，库中只存 SHA-256
- 禁止自我配对
- 所有变更使用 `BEGIN IMMEDIATE`（SQLite）或 `SERIALIZABLE`（Postgres）事务

## 实时网关 (`src/realtime/`)

### WebSocket 升级流程

1. 客户端发送 `GET /ws/client?ticket=...` 或 `GET /ws/host?ticket=...`
2. 验证 WS ticket JWT + 检查角色匹配 + 消费一次性票据
3. 启动 `run_session()` 主循环

### 会话循环

- 发送 `Hello` 帧（conn_id, server_time_ms, heartbeat_interval_ms）
- 自动订阅默认 topic（Account 或 Host）
- `tokio::select!` 分支：客户端消息 / 推送通道 / 遗留 outbox / 撤销信号
- 关闭码: 4401（认证撤销）、1011（内部错误）、4400（请求错误）

### RealtimeFanout（核心扇出引擎）

- 持有 `SessionRegistry`、`SubscriptionManager`、`StoreHandle`、`MessageBusBackend`
- `fanout_ui_event()` — 推送到特定设备会话
- `fanout_social_message()` — 推送到所有关联账户的移动端会话
- `dispatch_outbox_batch()` — 认领 `social_durable` 车道 outbox，publish 后 ack（不阻塞 host 命令）
- `dispatch_host_command_outbox_batch()` — 认领 `host_command` 车道；publish 后异步等待 host ack（过期 dead_letter，禁止假成功 ack）

### MessageBusBackend（多实例集群）

- `Inline` — 无操作（单实例）
- `Redis` — 通过 Redis pub/sub，支持自动重连

## 数据库层 (`src/store/`)

### 存储: `StoreHandle` 枚举

`Sqlite(SqlitePool)` 或 `Postgres(PgPool)`，通过 `AsStorePool` trait 抽象。

Migrations 为 latest-only 单一初始 schema（`sqlx::migrate!`）：
- SQLite：`crates/minos-backend/migrations/sqlite/0001_initial.sql`
- Postgres：`crates/minos-backend/migrations/postgres/0001_initial.sql`

不保留增量 ALTER 链；schema 变更直接改对应方言的 canonical 文件并 wipe 本地 DB。

### 核心表

| 表 | 用途 |
|----|------|
| `accounts` | 账户（email, supabase_sub, minos_id, display_name）— 无本地密码 |
| `device_installations` | 安装（kind: mobile/browser/desktop/host, public_key, account_id） |
| `pairing_tokens` | 配对令牌（token_hash, issuer_device_id, expires_at） |
| `host_links` | 配对码（code_hash, host_installation_id, status） |
| `host_installation_tokens` | Host 安装令牌 |
| `refresh_tokens` | 刷新令牌 |
| `host_links` | 账户-Host 关联 |
| `friend_requests` | 好友请求 |
| `friendships` | 好友关系 |
| `conversations` | 对话（直接/群组） |
| `chat_messages` | 聊天消息 |
| `agents` | Agent 定义 |
| `projects` | 项目 |
| `agent_sessions` | Agent 会话 |
| `agent_turns` | Agent 轮次 |
| `agent_turn_events` | 轮次流事件 |
| `approval_requests` | 审批请求 |
| `host_commands` | 持久化命令队列 |
| `durable_event_log` | 按 topic 排序的事件日志 |
| `outbox_events` | 分发工作队列（`lane`: `social_durable` \| `host_command`） |
| `raw_events` | Host-local `seq` 的 Agent 原始事件，按 `(host_device_id, session_id, seq)` 幂等 |
| `thread_sync_state` | Host manifest、backend ack 水位、partial history metadata |

### 30 个 Store 子模块

涵盖: accounts, device_installations, tokens, host_links, host_installation_tokens, refresh_tokens, host_links, agent_sessions, agent_turns, agent_turn_events, approval_requests, host_commands, durable_event_log, outbox_events, sessions, raw_events, thread_sync_state, projects, push_tokens, notification_preferences, notification_cooldowns 等。

## Agent 会话管理 (`src/agent_sessions/`)

### 生命周期

1. `POST /v1/agent-sessions/start` — 创建会话（幂等键），关联 conversation + 可选 project
2. Host 在线时通过 `HostIngestLiveBatch` WS 帧发送 chunk；重连时通过 `HostGapManifest` 上报缺口 metadata
3. `POST /v1/agent-sessions/send-input` — 用户输入
4. `POST /v1/agent-sessions/respond-opencode-question` — 提交 opencode `question.asked` 的答案
5. `POST /v1/agent-sessions/stop` — 请求停止
6. `POST /v1/agent-sessions/read-turns` — 读取轮次历史

### Host Ingest Sync

- `/ws/host` 接收 `HostIngestLiveBatch`、`HostGapManifest`、`HostIngestPullResponse`。
- live 和 pull chunk 共用严格幂等写入：同 `(host_device_id, session_id, seq)` 且同 checksum 视为重复；同 key 不同 checksum 报不变量错误，不重新分配 backend seq。
- **未知 formal session**：若 `agent_sessions` 无对应 `session_id`，chunk 被丢弃（不自动从 Desktop-local-only 会话创建 hub 投影）。云端可见 session 必须先经 start API（或等价注册）落库。
- **首次成功 insert 副作用**（golden path）：
  1. `agent_sessions.status` 若为 `pending` 则提升为 `running`
  2. 从 payload 识别 `approval/request` / `approval/timeout`，写入/解决 `approval_requests`（支撑远程 `POST /v1/approvals/respond`）
  3. 若 formal `agent_sessions` 不存在但 host 已 Link：用 chunk 上的 `conversation_id`（或 `host-local-session:{id}`）**自动登记** formal session（必要时 ensure 云端 group conversation），再 accept ingest
  4. **server** 用 `SessionTranslators`（`minos-ui-protocol::translate_*`）把 raw 投成 `UiEventMessage`，同步 formal turn/session 状态并 fanout；host 自带 `projection` 忽略
  4. 向 `agent_session:{id}` 的 `/ws/client` 订阅者 fanout `StreamEvent{kind: ui_event}`
- 旧 envelope 路径的 peer-target 解析：`host_links` → 同 account 下 mobile/browser/desktop installations（带缓存；Host Link/unlink 时 invalidate）
- `HostGapManifest` 落 `thread_sync_state` 后，backend 立即按 manifest range 通过同一条 host WS 发 `PullIngestRange`。
- host 从本地 SQLite 读取 range 并回 `HostIngestPullResponse`；backend 持久化后只按连续 raw event 前缀发送 `PullAck`。
- `agent_sessions.host_device_id` 为空时，新 handler 允许当前 host 认领该 session，避免 session 创建与 host 绑定之间的流式事件窗口丢失。

### Host 命令系统

- 持久化命令队列存储在 `host_commands` 表
- 通过 `durable_event_log` → `outbox_events` 发出
- Host 通过 `HostCommandAck` 确认，`HostCommandResult` 完成
- opencode question 答案通过 `minos_respond_opencode_question` 远端 host command 转发到 daemon
- `HostCommandTimeoutJob` 后台任务处理超时命令

## 状态管理层次

```
RuntimeShell           -- 拥有 AppContext、后台任务、集群监听
  └── AppContext       -- 组合所有服务
        ├── SessionRegistry           -- 内存中的活跃 WS 会话
        ├── SubscriptionManager       -- topic 订阅管理
        ├── HostLinkService            -- 配对业务逻辑
        ├── AuthUseCase               -- 认证业务逻辑
        ├── IngestUseCase             -- 原始事件摄取
        ├── RealtimeFanout            -- 事件扇出引擎
        ├── ApprovalService           -- 审批请求处理
        ├── HostCommandService        -- 持久化命令队列
        ├── AgentSessionService       -- Agent 会话管理
        ├── ProjectService            -- 项目 CRUD
        ├── NotificationService       -- 推送通知
        └── StoreHandle               -- 数据库连接池
```

## 后台任务 (`src/jobs/`)

| 任务 | 用途 |
|------|------|
| `RefreshTokenGcJob` | 清理过期 refresh token |
| `ApprovalTimeoutJob` | 过期审批自动处理 |
| `HostCommandTimeoutJob` | 超时 host 命令标记失败 |
| `RetentionCleanerJob` | 清理旧事件/线程 |
| `SessionLifecycleJob`（`stale_session_sweeper` 模块） | 失联 host → open `agent_sessions` → `failed` + durable end；CompletionWatch TTL → 失败气泡 + remove（**非** COUNT-only） |
| `AuditIndexerJob` | 审计数据索引 |
| `OutboxDispatcherJob` | 分发 `social_durable` 车道 outbox |
| `HostCommandOutboxJob` | 分发 `host_command` 车道（与 social 隔离 claim/lease） |
| `AgentDispatchWorkerJob` | 异步 drain `agent_dispatch_queue`；arm CompletionWatch |

所有任务实现 `Job` trait（`tick()`, `idle_interval()`, `applies_to(runtime_mode)`）。`SessionLifecycleJob` / `AgentDispatchWorkerJob` 需要 `AppContext`（registry + completion_watches）。

## 错误处理 (`src/error.rs`)

`BackendError` 枚举（thiserror derive）：StoreConnect, StoreMigrate, DeviceNotFound, PairingTokenInvalid, EmailTaken, PeerOffline, ForwardRpc 等。

HTTP 错误返回 `ErrorEnvelope`（`code` + `message`），WebSocket 使用关闭码（4400/4401/1011）。

## 集成测试 (`tests/`)

15 个测试文件覆盖：auth 端点、配对流程、agent 会话、社交功能、项目、WS 网关、HTTP 握手、端到端流程、事件摄取、存储、CORS 等。
