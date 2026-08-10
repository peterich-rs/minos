# Rebuild Minos Backend for Formal Development

> **Authority（2026-08-09）**：本文是 formal cutover **历史纲领**（account bearer / host installation rail / 拆分 `/ws/client`·`/ws/host`）。  
> **协作消息与 bot participant 的现行 SSOT** 以 [architecture-messaging.md](architecture-messaging.md)、[ADR 0021](adr/0021-agent-as-conversation-bot-participant.md)、[agent-participant-delivery](superpowers/specs/2026-08-09-agent-participant-delivery.md)、[global-bot-identity-design](superpowers/specs/global-bot-identity-design.md) 及 2026-08 IM reliability / Hub SSOT specs 为准。  
> 不再声称「唯一主动设计文档」或「superpowers 全体退休」。

## Breaking Change Notice

这个方案是一次有意为之的重置，不是对 MVP 后端的延续性修补。它会带来以下破坏性变化：

- 公开接口从“设备头 + 混合 `/devices` WebSocket + 进程内转发约定”收敛为“account bearer public API + host installation rail + 拆分后的 realtime gateways + 领域命令接口”。
- 生产数据模型从 MVP 的 SQLite/兼容迁移思路切换到正式开发的统一领域模型，不再保留旧 schema 和旧 reply 路径的兼容义务。
- “host = macOS” 不再出现在任何公开契约中。host 是一个跨平台安装形态，平台维度只体现在 installation metadata，允许 macOS / Windows / Linux。
- 历史 spec/plan 文档不再作为任何实现判断依据。

内部客户端和运维侧的迁移步骤应固定为：

1. 以新的 API 契约重建 mobile、web、host daemon 三端的 backend 接入层。公开 `/v1/*` 只接受 account bearer principal；host daemon 只走 `/v1/host/*` 和 `/ws/host`，不再依赖 `x-device-role` 一类 transport header 表达业务身份。
2. 以新的正式开发 schema 初始化数据库，不再兼容 MVP 阶段的历史迁移链和旧 DB 文件。
3. 将实时连接从单一 `/devices` 改为 `/ws/client` 与 `/ws/host` 两条网关，统一先申请短 TTL ticket，再升级连接。
4. 所有设计评审、需求增补、接口争议统一回到本文件与 `docs/adr/`，不再回看 `docs/superpowers/*`。

## Feasibility Assessment

当前仓库已经实现了 auth、pairing、sessions/ingest、projects、social、approvals、realtime 等完整业务面。真实代码分布在 `crates/minos-backend/src/http/`, `crates/minos-backend/src/auth/`, `crates/minos-backend/src/pairing/`, `crates/minos-backend/src/ingest/`, `crates/minos-backend/src/social/`, `crates/minos-backend/src/project/`, `crates/minos-backend/src/realtime.rs`, `crates/minos-backend/src/approval_relay.rs`, `crates/minos-backend/src/host_command_runtime.rs`, `crates/minos-backend/src/store/` 与 `crates/minos-backend/src/http/v1/contract.rs`。

现有代码已经验证了以下事实：

- account-authenticated mobile/web rail 存在且可运行。
- host 与 account 的多链接关系已经在 `account_host_pairings` 中落地，不再是单 peer 假设。
- 当前 `/devices` 同时承担 host 与 account client 的升级入口，确实把不同鉴权语义揉在了一起。
- 旧 `forward_rpc.rs` 已被删除；当前 host 同步响应完全由 `host_commands` + in-process notifier 辅助唤醒承担，权威状态在持久化表而不在进程内 `DashMap`。
- caller-scoped `/v1/me/*` 已退役；正式 API surface 已把 account 与 host rail 切开。

这意味着产品范围已经被代码验证，只是运行时骨架、身份边界、实时模型和契约层仍带有 MVP 阶段的耦合与兼容面。正式开发保留业务范围、重写骨架和契约层，不存在需求层面的阻塞。Fully feasible.

## Current Surface Inventory

- `crates/minos-backend/src/http/mod.rs` — Axum 入口、`BackendState` 组装、HTTP middleware 与路由挂载。
- `crates/minos-backend/src/http/v1/auth.rs` — 账号注册、登录、刷新、注销、ws-ticket 等身份接口；当前仍混用 device rail 与 bearer rail。
- `crates/minos-backend/src/http/v1/pairing.rs` / `host.rs` — caller-scoped host / peer 视图已拆入 pairing 与 host installation 自查询接口。
- `crates/minos-backend/src/http/v1/pairing.rs` — 当前 host 配对码申请、移动端确认、解绑入口。
- `crates/minos-backend/src/http/v1/agent_sessions.rs` — agent session 查询、事件读取与命令入口；旧 thread HTTP surface 已删除。
- `crates/minos-backend/src/http/v1/social.rs` — profile、好友、会话与消息相关 HTTP 面；当前也包含部分 host call path。
- `crates/minos-backend/src/http/v1/projects.rs` — 当前项目列表、创建，以及 canonical `conversations/link`、`agent-sessions/query|link` 入口。
- `crates/minos-backend/src/http/ws_devices.rs` — 当前混合 `/devices` WebSocket 升级与 live session 激活。
- `crates/minos-backend/src/envelope/mod.rs` — 当前 WebSocket 收发循环、Forward/Forwarded/Event 分发。
- `crates/minos-backend/src/session/registry.rs` — 当前进程内在线会话注册表与 reconnect replace 逻辑。
- `crates/minos-backend/src/realtime.rs` — 当前多实例 fan-out、Redis/in-memory 后端、peer target cache。
- `crates/minos-backend/src/approval_relay.rs` — 当前 pending approvals 管理、超时轮询、审批结果回传；审批回传统一走 `host_commands` runtime。
- `crates/minos-backend/src/ingest/mod.rs` — 当前 host 原始事件入库、翻译、向 account peers 广播。
- `crates/minos-backend/src/store/account_host_pairings.rs` — 当前 `(host_device_id, mobile_account_id)` 链接关系实现，已验证多账号链接成立。
- `crates/minos-backend/src/store/raw_events.rs` — 当前 thread 级原始事件存储，后续会演化为 agent turn / stream recovery 的底层来源之一。
- `crates/minos-backend/migrations/sqlite/0001_initial.sql` / `migrations/postgres/0001_initial.sql` — 各方言单一 canonical schema（latest-only，无增量迁移链）。

## Design

### Target Scope

正式开发阶段保留以下产品范围，不再把“是否已经在 MVP 中这样实现过”当成约束：

- 账号体系：注册、登录、刷新、注销、installation 绑定与会话管理。
- Host 配对：host daemon 申请 pairing code，已登录 account principal 确认，建立 `(account_id, host_installation_id)` 链接，支持解绑与列表查询。
- Host 安装信任：host daemon 在 pairing 成功后获得独立的 host installation credential，仅可调用 host rail 接口与 host realtime gateway。
- Agent 会话：启动、发送输入、停止、拉取 turn 流、恢复上下文。
- 审批流：工具调用审批、超时、断线、审计、重试。
- 会话与消息：好友、群聊、agent 参与者、消息读取状态、消息提及。
- 项目：项目创建、conversation 归档到项目、workspace binding、项目级 agent policy。
- 实时层：WS 推送、跨实例 fan-out、重连恢复、presence、delivery、subscription auth。

以下内容明确不继承 MVP 包袱：

- `x-role`、`x-device-role`、`x-device-secret` 这类 transport header 作为公共业务身份入口的做法。
- 为旧客户端保留的兼容路由、兼容 schema、兼容 reply 路径。
- “所有推送事件都写 DB outbox” 的统一化假设。正式开发明确区分 durable 领域事件和 ephemeral stream 事件。
- `session` 作为顶层领域名词。正式开发以 `conversation` 和 `agent_session` 为主；现有 `project_sessions`、`sessions/read` 等旧词仅作为迁移背景存在。
- 将 host 平台硬编码为 macOS。平台差异只留在 installation metadata 和 host runtime 自身，不进入公开契约。

显式不纳入本轮正式开发范围的内容：

- 组织级 host fleet 管理、批量设备编组、跨 account 的管理员授权模型。
- “一台物理主机下多 host installations 编组管理”这类更高一层的资产管理能力。正式开发第一阶段只保证单个 host installation 的配对、凭证、会话与恢复模型清晰可实现。
- 同一 `(account_id, host_installation_id)` 之上的额外“多步扫码编组”流程；重复扫码只定义为幂等 re-link，而不是另一套设备管理产品。

### Technology Selection

- 语言与运行时：Rust + Tokio。
- HTTP / WS 框架：Axum + Tower。
- 主数据库：PostgreSQL 16，使用 `sqlx` 维持一致的 async 数据访问模型。
- 本地开发数据库：SQLite 仅用于开发便利与超轻量测试，不作为正式部署目标。
- 缓存与消息总线：Redis 7，承担 rate-limit bucket、pub/sub、短 TTL realtime ticket、跨实例 fan-out。
- 认证：Argon2id 密码哈希，JWT access token，旋转 refresh token，host installation token，短 TTL 单次使用 realtime ticket。
- 可观测性：OpenTelemetry traces + Prometheus metrics + JSON structured logs。
- 后台任务：默认以进程内 Tokio supervised workers 运行；只有吞吐或隔离要求被证明后，才拆成独立 deployment。
- 契约源：`/v1/*` 的 OpenAPI 与 WS frame JSON schema 作为合同源，CI 只负责 drift gate，不承担合同定义本身。

明确参数：

- access token TTL `<= 15 min`。
- refresh token 使用旋转模型；强制下线依赖 refresh 撤销 + access token 自然过期，不引入 deny list。
- realtime ticket 使用 Redis 保存，默认 TTL 60 秒，单次 consume，可撤销，不建 `ws_tickets` SQL 表。

### Key Design Decisions

1. 采用“模块化单体”而不是拆成多个微服务。
原因：当前团队规模和产品阶段更需要统一交付、事务一致性和调试清晰度，而不是跨服务治理成本。

2. 生产环境以 PostgreSQL 为唯一权威数据源。
原因：账号、审批、会话、项目、社交与 host command 都是多写路径；SQLite 适合开发测试，不适合作为正式开发主库。

3. 保持 POST-first API，不追求严格 RESTful。
原因：Minos 的主路径是命令驱动而不是 CRUD 驱动，诸如 login、confirm pairing、start agent session、respond approval 都更适合显式 command endpoint。

4. 公开 HTTP 身份只接受 `AccountPrincipal`；host daemon 使用独立 `HostInstallationPrincipal`，但仅可进入 `/v1/host/*` 与 `/ws/host`。
原因：这同时解决了两个矛盾：一方面保留 host 的独立后端身份，另一方面杜绝 `Principal::Host` 渗入公开 `/v1/*` handler 再次退化成“按角色分支”的旧模型。browser 只是 account installation kind，不是单独业务 principal。

5. 实时投递明确区分 durable 与 ephemeral 两类事件。
原因：`AccountRegistered`、`ApprovalResolved`、`ConversationMessageAppended` 这类领域事件需要 outbox、审计与 replay；agent text delta、stdout chunk、diff streaming 这类高频 UI 事件不能把 Postgres 当消息队列写。

6. 删除 `forward_rpc.rs`，以持久化 `host_commands` 模型替代“全局 DashMap + oneshot 等待”。
原因：approval 不是唯一需要 host 同步响应的场景；host command 必须可超时、可恢复、可跨实例投递、可审计。in-process waiter 只允许作为优化缓存，不能是真理来源。

7. 领域模型围绕 `conversation`、`agent_session` 与 `project` 收敛，移除 `session` 旧词。
原因：conversation 是用户可见的聊天/协作边界，agent session 是一次执行生命周期；旧 `session` 词义在当前代码里混合了流、会话和项目归档，继续沿用只会把 phase 4/5 反复拖回旧模型。

8. Worker plane 是逻辑 plane，不是 Day 0 的部署承诺。
原因：approval timeout、refresh cleanup、ticket cleanup、outbox dispatch 都是低频后台任务。第一阶段默认以同进程 supervised task 运行，拆分部署是配置选项，不是基础架构义务。

9. API contract 与 observability 一起视为平台能力，而不是 cutover 时补的附属物。
原因：正式开发阶段的问题定位、SDK 生成、权限回归、replay 与 drift gate 都依赖稳定合同和稳定标识。

### Target Runtime Topology

逻辑上拆成五层，但初期默认仍由一个二进制承载：

- Public Control API：`/v1/*`，只接受 `AccountPrincipal`，负责 auth、pairing confirm/revoke/list、projects、conversations、agent session commands、approval respond。
- Host Control API：`/v1/host/*`，分为 bootstrap sub-surface 与 steady-state sub-surface：`/v1/host/pairing/*` 接受 `HostBootstrapPrincipal`，`/v1/host/installations/self` 与 `/v1/host/realtime/ws-ticket` 接受 `HostInstallationPrincipal`。
- Client Gateway：`/ws/client`，负责 account-scoped subscription、resume、delivery。
- Host Gateway：`/ws/host`，负责 host command delivery、agent runtime uplink、host-scoped session control。
- Worker Plane：outbox dispatcher、host command timeout resolver、approval timeout resolver、refresh cleanup、stale session sweeper。

这五层共享同一套 domain/use-case/repository 定义，通过 `AppContext` 连接。部署上允许：

- 本地开发：一个进程同时跑五层。
- 初始生产：一个 deployment 同时跑 public API、host API、client gateway、host gateway 与 supervised workers。
- 负载增长后：按配置拆成 API / gateway / worker deployment，但不改变进程内接口。
- listener 形态默认共用一个 listener；若生产侧需要把 host rail 放到单独 listener 或 VPC 内网，只通过 config 切分 listener，不改 path prefix 与合同。

### Core Interfaces

```rust
pub struct AppContext {
    pub config: Arc<AppConfig>,
    pub repos: Arc<RepositorySet>,
    pub auth: Arc<AuthService>,
    pub host_installations: Arc<HostInstallationService>,
    pub pairing: Arc<PairingService>,
    pub agent_sessions: Arc<AgentSessionService>,
    pub approvals: Arc<ApprovalService>,
    pub conversations: Arc<ConversationService>,
    pub projects: Arc<ProjectService>,
    pub durable_events: Arc<dyn DurableEventStore>,
    pub realtime: Arc<dyn RealtimePublisher>,
    pub host_commands: Arc<dyn HostCommandService>,
    pub jobs: Arc<dyn JobScheduler>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGenerator>,
}

pub enum Principal {
    Account(AccountPrincipal),
    HostBootstrap(HostBootstrapPrincipal),
    HostInstallation(HostInstallationPrincipal),
    InternalWorker(WorkerPrincipal),
}

pub enum DurableEvent {
    AccountRegistered { account_id: String },
    HostLinked { account_id: String, host_installation_id: String },
    AgentSessionStarted { session_id: String, conversation_id: String },
    ApprovalRequested { request_id: String, session_id: String },
    ApprovalResolved { request_id: String, resolution: ApprovalResolution },
    ConversationMessageAppended { conversation_id: String, message_id: String },
    ProjectConversationLinked { project_id: String, conversation_id: String },
}

pub enum StreamEvent {
    AgentTextDelta { session_id: String, turn_id: String, seq: i64 },
    StdoutChunk { session_id: String, turn_id: String, seq: i64 },
    DiffChunk { session_id: String, turn_id: String, seq: i64 },
    ApprovalProgress { request_id: String, seq: i64 },
}

pub trait DurableEventStore: Send + Sync {
  async fn record(&self, tx: &mut DbTx, topic: RealtimeTopic, event: DurableEvent) -> Result<TopicCursor, AppError>;
}

pub trait RealtimePublisher: Send + Sync {
  async fn publish_ephemeral(&self, topic: RealtimeTopic, event: StreamEvent) -> Result<(), AppError>;
}

pub trait HostCommandService: Send + Sync {
    async fn enqueue(&self, command: HostCommand) -> Result<HostCommandReceipt, AppError>;
}
```

### API Conventions

- 成功响应统一使用 `{ "data": ..., "meta": { ... } }` 包络；最少携带 `request_id`，分页型响应额外携带 `next_cursor`。
- 错误响应统一使用 `{ "error": { "code", "message", "request_id", "retry_after_ms"? } }`。
- 所有 list / sync / read 类 POST 接口统一使用 `cursor` 与 `limit` 作为输入字段；响应通过 `meta.next_cursor` 返还下一页游标。
- `POST /v1/agent-sessions/read-turns` 是唯一例外：默认模式按 turn 元数据分页（`after_turn_seq` + `limit`，响应返回 `next_turn_seq`），与 `agent_session:<id>` topic 的 live cursor 共享同一序列空间；当请求带 `turn_id` 时切换为 slice 模式（`after_event_seq` + `limit`，响应返回 `next_event_seq`），用于读取 turn 内的 `agent_turn_events`。
- 所有时间戳统一使用 `*_ms` Unix epoch milliseconds；不混用 ISO8601 作为合同字段。
- 所有公开 ID 统一使用带前缀的 ULID 字符串，例如 `acct_01J...`, `inst_01J...`, `host_01J...`, `conv_01J...`, `sess_01J...`, `turn_01J...`, `apr_01J...`, `cmd_01J...`。
- 公开 ID 默认带前缀 ULID。**Bot 身份**使用全局 `agent_id`（实现上常为 `bot-<uuid>` 的用户配置 bot，见 [global-bot-identity-design](superpowers/specs/global-bot-identity-design.md)）。历史文档中的 `agent_codex` 等稳定 slug 仅为 runtime 种子/兼容别名，**不是**产品主路径上的 bot 用户身份。
- 所有幂等写接口至少接受 `client_request_id`：`/v1/agent-sessions/start`, `/v1/agent-sessions/send-input`, `/v1/conversations/send-message`, `/v1/approvals/respond`, `/v1/pairing/confirm`, `/v1/host/pairing/redeem`。
- OpenAPI 是 `/v1/*` 的合同源，由 Rust request/response types 派生；WS frame 使用 JSON schema 作为合同源。客户端 SDK 从合同生成，CI 只做 drift gate。

### API Shape

公开 account rail：

- `/v1/auth/register`
- `/v1/auth/login`
- `/v1/auth/refresh`
- `/v1/auth/logout`
- `/v1/auth/change-password`
- `/v1/realtime/ws-ticket`
- `/v1/pairing/confirm`
- `/v1/pairing/revoke`
- `/v1/pairing/list-hosts`
- `/v1/agent-sessions/start`
- `/v1/agent-sessions/send-input`
- `/v1/agent-sessions/stop`
- `/v1/agent-sessions/list`
- `/v1/agent-sessions/read-turns`
- `/v1/approvals/respond`
- `/v1/conversations/list`
- `/v1/conversations/sync`
- `/v1/conversations/send-message`
- `/v1/projects/list`
- `/v1/projects/create`
- `/v1/projects/rename`
- `/v1/projects/archive`
- `/v1/projects/conversations/link`
- `/v1/projects/agent-sessions/list|query`
- `/v1/projects/agent-sessions/link`

兼容别名（待剩余旧 caller 清理后删除）：

- `/v1/projects/link-conversation`

host rail：

- `/v1/host/bootstrap/nonce`
- `/v1/host/pairing/request-code`
- `/v1/host/pairing/redeem`
- `/v1/host/realtime/ws-ticket`
- `/v1/host/installations/self`

realtime gateways：

- `/ws/client?ticket=<ticket>`
- `/ws/host?ticket=<ticket>`

显式收敛点：

- 现有 `/v1/me/*` 不进入正式开发合同。account caller 的 host 列表收敛到 `/v1/pairing/list-hosts`；host caller 的自查询收敛到 `/v1/host/installations/self`。
- `client` 字段不出现在 `/v1/realtime/ws-ticket` 请求中。installation kind 已存在于 `device_installations`，后端自行判定。
- `workspace` 不作为 `agent-sessions/start` 的独立坐标对象暴露。项目内会话只传 `project_id`；workspace binding 由 project aggregate 负责。
- `/v1/host/installations/self` 不返回 account email 等 PII；只返回 host 自身 metadata、`link_count` 与 per-link display name 摘要。

示例：HTTP 申请 client realtime ticket，再建立连接。

```http
POST /v1/realtime/ws-ticket
Authorization: Bearer <access-token>
Content-Type: application/json

{
  "installation_id": "inst_01J..."
}
```

```json
{
  "data": {
    "ticket": "wst_01J...",
    "expires_at_ms": 1760000000000,
    "gateway_url": "wss://api.minos.dev/ws/client?ticket=wst_01J..."
  },
  "meta": {
    "request_id": "req_01J..."
  }
}
```

示例：启动 agent session。

```http
POST /v1/agent-sessions/start
Authorization: Bearer <access-token>
Content-Type: application/json

{
  "agent_id": "agent_codex",
  "conversation_id": "conv_01J...",
  "project_id": "proj_01J...",
  "client_request_id": "req_01J..."
}
```

约束：

- `conversation_id` 必填；一个 `agent_session` 必须强引用一个 conversation。
- `project_id` 可选；若传入，则必须与该 conversation 的 `project_id` 一致。
- 若要创建新 conversation，应先调用 conversation create/send-message 路径，再启动 session，而不是让 `agent-sessions/start` 同时承担资源创建。

示例：响应审批请求。

```http
POST /v1/approvals/respond
Authorization: Bearer <access-token>
Content-Type: application/json

{
  "request_id": "apr_01J...",
  "decision": "approve",
  "client_request_id": "req_01J..."
}
```

`/v1/approvals/respond` 的语义固定为：

- 输入只接受 `request_id`、`decision`、`client_request_id`，不再混入 `session_id` / `scope` 等模糊坐标。
- 成功路径先把 `approval_requests` 持久化为 resolved，再入队一个 `host_command` 交给 host daemon 继续执行。
- `ApprovalResolved.resolution` 只允许 `Decided`, `Timeout`, `Disconnect` 三类，禁止把人工响应和自动超时混成同一个无标签事件。

### Pairing Flow Sequence

pairing 三步流必须按以下顺序实现，不能把 bootstrap 阶段偷换成 header-based 临时身份。`pairing_codes.status` 同时承担"配对状态机"与"一次性凭证"两种语义，不再引入独立 `pairing_receipts` 资源：

1. `POST /v1/host/pairing/request-code`
  - 调用方不是 `HostInstallationPrincipal`，而是 `HostBootstrapPrincipal`（详见 Host Bootstrap Proof）。
  - 请求成功后，`pairing_codes.status = 'pending'`，并绑定 `host_installation_id`。
  - backend 在此阶段写入或更新 `device_installations(kind = 'host')` 行，但 host 还未拿到长期 token，也未与 account 建立链接。
2. `POST /v1/pairing/confirm`
  - account rail 消费 pairing code，创建或幂等确认 `host_links(account_id, host_installation_id)`。
  - 该端点把 `pairing_codes.status` 推进为 `'confirmed'`，并写入 `account_id` 字段；不返回额外 receipt 字符串，host 端继续使用同一 pairing code 完成下一步。
3. `POST /v1/host/pairing/redeem`
  - host 以 `(installation_id + nonce + key_signature + pairing_code)` 调用 redeem。
  - backend 校验：(a) Host Bootstrap Proof 一致；(b) `pairing_codes.status = 'confirmed'` 且 `host_installation_id` 匹配；(c) code 未过期。
  - 校验通过后，原子地把 `pairing_codes.status` 推进为 `'redeemed'`，并签发 `host_installation_token`。
4. steady-state
  - 此后 host 仅可用 `HostInstallationPrincipal` 调用 `/v1/host/installations/self` 与 `/v1/host/realtime/ws-ticket`。
  - reconnect 必须重新申请 realtime ticket；旧 ticket 单次 consume 后不可复用。

`pairing_codes.status` 状态机固定为 `pending → confirmed → redeemed`；任意非法跃迁、过期或重复 redeem 都返回 `pairing_code_invalid` 并写审计。

### Host Bootstrap Proof

host 在 pairing 三步流的第 1、第 3 步用持久 installation keypair 证明身份：

- 算法：Ed25519。host 在首次启动时本地生成 keypair，私钥存于 host 自身 secure storage，公钥通过 bootstrap 请求登记到 backend。
- 一次性 nonce：host 必须先调用 `POST /v1/host/bootstrap/nonce`，backend 生成短 TTL（默认 60 秒）单次使用 `bootstrap_nonce` 并写入 Redis；host 在 request-code / redeem 请求中携带该 nonce + 自己的 Ed25519 签名。后端在 nonce consume 时一并验证签名，避免重放。
- TOFU 登记：第一次见到 `(installation_id, public_key)` 时，backend 无条件登记到 `device_installations.public_key` 字段，并写一条 `audit_events`，标记 actor `HostBootstrap`。
- 后续严格匹配：之后任何对同一 `installation_id` 的 bootstrap 请求都必须使用同一 public key 验签；mismatch 返回 `host_bootstrap_proof_invalid` 并按 IP + installation_id 做指数退避。
- 公钥轮换：host 需要换 keypair 时，必须先调用 `/v1/pairing/revoke` 解除所有 link，使 host installation 退化到无 link 状态后，由用户重新走完整三步流。当前阶段不提供"在线 rotate public_key"接口。
- bootstrap proof 的失败码与节流策略：连续 5 次失败后，`(installation_id, ip)` 进入 60 秒退避，返回 `host_bootstrap_throttled`。

bootstrap 请求最小 wire shape 固定为：

```http
POST /v1/host/bootstrap/nonce
Content-Type: application/json

{ "installation_id": "inst_01J..." }
```

```json
{
  "data": {
    "nonce": "nonce_01J...",
    "expires_at_ms": 1760000060000
  },
  "meta": { "request_id": "req_01J..." }
}
```

```http
POST /v1/host/pairing/request-code
Content-Type: application/json

{
  "installation_id": "inst_01J...",
  "nonce": "nonce_01J...",
  "public_key": "ed25519:...",
  "signature": "ed25519-sig:..."
}
```

签名 payload 固定为 `installation_id + ":" + nonce + ":" + path`，path 取请求路径常量（`/v1/host/pairing/request-code` 或 `/v1/host/pairing/redeem`）。`public_key` 仅在 TOFU 登记时使用，后续请求中可省略；backend 始终以已登记的公钥验签。

### Subscription and Resume Protocol

realtime gateway 的订阅模型固定如下：

- `/ws/client` 在握手成功后自动订阅 `account:<account_id>`。
- `/ws/host` 在握手成功后自动订阅 `host:<host_installation_id>`。
- 额外 topic 只能通过显式 `subscribe` frame 建立，不允许“按 ticket 自动订阅所有可见对象”。
- topic 命名固定为：
  - `account:<account_id>`
  - `conversation:<conversation_id>`
  - `project:<project_id>`
  - `agent_session:<session_id>`
  - `host:<host_installation_id>`
- account principal 可订阅与自己账号、membership、project scope 一致的 topic；host principal 只可订阅自己的 `host:*` topic 与由 backend 明确下发的 `agent_session:*` topic。
- 单次 `subscribe` 最多 32 个 topic，单连接最多 128 个 live subscriptions；未授权 topic 返回 `subscription_denied`，超限返回 `subscription_limit_exceeded`。

durable resume cursor 固定为按 topic 维护的 `last_durable_seq`：

```json
{
  "type": "subscribe",
  "topics": [
    "conversation:conv_01J...",
    "agent_session:sess_01J..."
  ],
  "resume_after": {
    "conversation:conv_01J...": 128,
    "agent_session:sess_01J...": 992
  }
}
```

协议规则：

- 每个 durable event 都有 `(topic, topic_seq)`，`topic_seq` 在 topic 内单调递增。
- `agent_session:<session_id>` topic 的 durable 序列与 `agent_turns.turn_seq` 一一对应：每条 turn 提交时同时写一行 `durable_event_log`，`topic_seq = turn_seq`。客户端可以用 `read-turns(after_turn_seq)` 重建 turn 元数据，再以同一游标续接 live。
- `agent_turns` 只承载 turn 级元数据（角色、状态、起止时间、汇总字段）；turn 内的 stream slice（text delta、stdout chunk、diff chunk）由独立的 `agent_turn_events` 表逐 slice 持久化，序列空间是 `(turn_id, event_seq)`，与 topic_seq 不混用。
- gateway 只 replay durable events，不 replay ephemeral stream frames。
- durable retention 默认按“7 天或每 topic 10000 条，以先到者为准”；retention cleaner 必须先 `LEFT JOIN outbox_events` 跳过仍存在 unacked outbox 的行，避免清理 orphan 出 unacked 任务。
- `agent_turn_events` 的 retention 跟随对应 `agent_session` 的关闭时间，默认在 session 结束 7 天后清理；客户端在此期间内可通过 `read-turns(turn_id, after_event_seq)` 拉取 stream cold replay。
- 如果 cursor 落到 retention 之外，gateway 返回 `snapshot_required`，客户端必须走 `/v1/conversations/sync`、`/v1/agent-sessions/read-turns` 等 read API 重建状态。
- 同一 `(principal, installation_id)` 只保留最新一条 live WS 连接；旧连接被关闭并收到 4401，避免重连风暴下同 installation 持续占坑。
- ticket 为单次 consume；任何 reconnect 都必须重新申请 ticket，不能复用旧 ticket 配合 replace 语义。

`subscribe` 控制帧的最小 wire shape 固定为：

```json
{ "type": "subscribe_ack", "topics": ["conversation:conv_01J...", "agent_session:sess_01J..."] }
```

```json
{ "type": "subscription_denied", "topic": "project:proj_01J...", "reason": "forbidden" }
```

```json
{ "type": "subscription_limit_exceeded", "limit": 128, "current": 128 }
```

### Host Command Model

`forward_rpc.rs` 在正式开发阶段被以下模型替代：

- 持久化表：`host_commands(command_id, host_installation_id, agent_session_id?, method, params_json, requested_by_account_id?, status, response_json?, error_json?, deadline_at_ms, created_at_ms, ack_at_ms?, finished_at_ms?)`。
- `agent_session_id` 可空；典型非 session 命令包括 host self diagnostic、日志导出、link-state refresh、token revoke / force close。
- 写入方式：任何需要 host 执行并可能返回结果的动作，都以数据库事务写入 `host_commands`，并与相应的 `approval_requests` / `agent_turns` / `agent_sessions` 状态一并提交。
- 投递方式：dispatcher 把 command 发到 `host:<host_installation_id>` topic，再由 `/ws/host` 送达对应 daemon。
- 响应方式：host 回 ACK / RESULT frame，backend 负责写回 `host_commands`。若上游 HTTP 请求仍在等待，可用 in-process notifier 做短期唤醒，但 notifier 不是权威状态。
- 超时方式：worker 扫描 `host_commands.deadline_at_ms`，把超时命令标记为 failed，并发布相应 durable event。
- 审批方式：`/v1/approvals/respond` 和 timeout/disconnect 自动决策都通过同一 `host_commands` 模型把结果发送回 host，不再存在另一条 approval 专属 transport 旁路。

最小 wire shape 固定为：

```json
{ "type": "host_command_ack", "command_id": "cmd_01J...", "ack_at_ms": 1760000000000 }
```

```json
{
  "type": "host_command_result",
  "command_id": "cmd_01J...",
  "status": "ok",
  "result": { "...": "..." },
  "finished_at_ms": 1760000001000
}
```

```json
{
  "type": "host_command_result",
  "command_id": "cmd_01J...",
  "status": "error",
  "error": { "code": "host_unavailable", "message": "..." },
  "finished_at_ms": 1760000001000
}
```

### Data Model

正式开发的数据模型按领域拆分为以下核心表：

- `accounts`
- `account_credentials`
- `refresh_tokens`
- `device_installations`
- `host_installation_tokens`
- `pairing_codes`
- `host_links`
- `agents`
- `projects`
- `project_members`
- `project_default_agents`
- `conversations`
- `conversation_members`
- `conversation_messages`
- `conversation_reads`
- `message_mentions`
- `agent_sessions`
- `agent_turns`
- `agent_turn_events`
- `approval_requests`
- `host_commands`
- `durable_event_log`
- `outbox_events`
- `audit_events`

#### Relationship Rules

- `device_installations` 是唯一的"安装"维度，覆盖 mobile / browser / host 三类，至少包含 `installation_kind`、`platform`、`public_key`（仅 host 必填）。host 不再单独复制一份设备元数据。
- `device_installations.account_id` 对 mobile / browser 必填；对 host 必须为空。host 的 account 归属只通过 `host_links` 表达。
- `host_links` 的业务键是 `(account_id, host_installation_id)`，并附带 `link_display_name`, `acl_json` 等 per-link 元数据，以及 `linked_via_installation_id` 作为 confirm 时的 caller installation 审计快照（仅用于 host self view 的 per-link display 与审计可读性，不用于鉴权决策）。
- `agents` 是 **全局 bot 目录**（per-owner 可配置数字肉身：runtime / model / reasoning / system prompt 等），主键为全局 `agent_id`；通过 `conversation_agent_members` 拉入多个 conversation，**不**为每个会话克隆 bot。`source=host_runtime` 行仅为 Host 能力种子，不是产品主 bot 目录。权威： [global-bot-identity-design](superpowers/specs/global-bot-identity-design.md)。
- `projects` 内嵌 `workspace_root`（host 端可解析的相对路径或 slug），1:1 表达项目的 workspace 绑定；正式开发第一阶段不引入独立 `project_workspaces` 表，避免对 1:N workspace 形成的复杂权限/选择 UI 做过度设计。
- `agent_sessions.conversation_id` 非空；一个 conversation 可以有多个 agent sessions，但一个 agent session 只属于一个 conversation。
- `agent_sessions.project_id` 可空；若不为空，必须与 `conversations.project_id` 一致。
- `agent_sessions.project_id` 与 `conversations.project_id` 的一致性由 use-case 在事务内校验，不用 DB trigger 表达。
- `agent_turns(turn_id PK, agent_session_id, turn_seq, role, status, started_at_ms, finished_at_ms?)`；`(agent_session_id, turn_seq) UNIQUE` 与 `agent_session:<id>` topic 的 durable 序列一一对应。
- `agent_turn_events(turn_id, event_seq, kind, payload jsonb, created_at_ms)`；主键 `(turn_id, event_seq)`，存放 turn 内所有 stream slice，是 stream cold replay 的唯一来源。
- 旧 `project_sessions` 概念被删除；conversation 通过 `conversations.project_id` 直接归属于 project，agent session 通过 `project_id` 镜像同一归属。

#### Key Index Inventory

- `host_links UNIQUE(account_id, host_installation_id)`
- `pairing_codes UNIQUE(code_hash)`，并建立 `(host_installation_id, status, created_at_ms)` 索引；`status ∈ {pending, confirmed, redeemed, expired}`
- `refresh_tokens UNIQUE(token_hash)`，并建立 `(account_id, installation_id)` 索引
- `host_installation_tokens UNIQUE(token_hash)`，并建立 `(host_installation_id, revoked_at_ms)` 索引
- `agent_sessions (conversation_id, status)` 与 `(project_id, started_at_ms)` 索引
- `agent_turns UNIQUE(agent_session_id, turn_seq)`
- `agent_turn_events PRIMARY KEY(turn_id, event_seq)`，并建立 `(turn_id, created_at_ms)` 索引以支持 retention 清理
- `approval_requests (agent_session_id, state)` 与 `(deadline_at_ms, state)` 索引
- `host_commands (host_installation_id, status, deadline_at_ms)` 索引
- `conversation_messages (conversation_id, created_at_ms)` 索引与 `UNIQUE(conversation_id, message_id)`
- `durable_event_log UNIQUE(topic, topic_seq)` 与 `(topic, created_at_ms)` 索引；表按 `topic_kind` 做 PostgreSQL declarative partitioning（`account` / `conversation` / `project` / `agent_session` / `host` 分区），retention 在分区粒度执行以避免单 topic 高基数下的全表扫描
- `outbox_events (status, available_at_ms)` 复合索引与 `event_id` 外键索引

#### Durable Event Log Shape

`durable_event_log` 的最小字段集固定为：

- `event_id ULID PK`
- `topic`
- `topic_seq`
- `partition_key`
- `payload jsonb`
- `created_at_ms`

该表用于重连 replay 与 retention，不承载 dispatcher claim / ack 状态。

#### Outbox Queue Shape

`outbox_events` 的最小字段集固定为：

- `outbox_id ULID PK`
- `event_id`
- `available_at_ms`
- `attempts`
- `claimed_by`
- `claimed_at_ms`
- `ack_at_ms`
- `dead_at_ms`

该表是 dispatcher 工作队列；acked row 可按队列策略清理，不承担 replay 语义。durable 领域事件与队列任务在同一事务内分别写入 `durable_event_log` 与 `outbox_events`。

`durable_event_log` retention cleaner 与 `outbox_events` 协调规则固定为：

- cleaner 删除 `durable_event_log` 行前必须 `LEFT JOIN outbox_events` 跳过仍有 unacked outbox 的行；
- worker 必须在 retention 上限之前把任何任务推进为 acked 或 dead-letter；超过 retention 仍未 acked 的任务直接进 dead-letter 并写审计；
- dead-letter 行单独保留 30 天，运维侧负责 triage。

#### Stream Slice Persistence

`agent_turn_events` 是 stream slice 的唯一持久来源，与 `agent_turns` 的关系固定为：

- 每条 stream chunk（agent text delta / stdout chunk / diff chunk / tool progress）必须先以 `INSERT` 写入 `agent_turn_events`，再以 `StreamEvent` 走 ephemeral 分发。
- `event_seq` 在 `turn_id` 内单调递增；同一 chunk 不允许覆盖写。
- turn 关闭时，`agent_turns.finished_at_ms` 被写入；后续 `read-turns(turn_id, after_event_seq)` 用于 cold replay。
- `agent_turn_events` 不进 `durable_event_log`；它的恢复语义由 read API 提供，不由 gateway replay 提供。
- retention 默认在对应 `agent_session` 关闭后 7 天清理；session 仍 open 的 turn 不会被清理。

#### Audit Boundary

`audit_events` 只审计以下安全和治理动作：

- register / login / logout / change-password
- pairing confirm / pairing revoke / host token redeem
- approval respond / approval timeout / approval disconnect resolve
- project archive / project membership change
- refresh token revoke / refresh reuse detection

每条审计记录至少带：`account_id?`, `installation_id?`, `actor_principal_kind`, `event_type`, `at_ms`, `metadata jsonb`。

### Security and Identity Model

- 账号登录成功后，客户端得到 access token + refresh token + 当前 `installation_id` 绑定信息。
- 公开 `/v1/*` 一律使用 bearer `AccountPrincipal`；不允许用角色头部、host secret 或 installation kind 拼装公共业务身份。
- host bootstrap 阶段使用持久 installation keypair + server-issued nonce 完成签名证明（详见 Host Bootstrap Proof），不使用 `HostInstallationPrincipal` 或 header-based 临时身份。
- host daemon 在 pairing redeem 之后获得 `host_installation_token`；后续 `/v1/host/installations/self` 与 `/v1/host/realtime/ws-ticket` 只接受这个凭证体系。
- `host_installation_token` 在每次请求时由 middleware 校验 `host_installation_tokens.revoked_at_ms IS NULL`，校验结果可在进程内做 ≤ 5 秒的负缓存以削峰，但缓存命中不能跨过 token 撤销窗口。
- host 本地机密只用于 host 自身 runtime trust、secure storage 或本地配对材料交换，不作为公开 HTTP 入口鉴权方式。
- realtime ticket 只存在于 Redis；若需要审计，写 `audit_events`，不额外建 SQL `ws_tickets` 表。
- refresh token reuse、密码变更、强制下线通过 refresh rotation + token revoke 处理；不引入 access deny list。
- pairing revoke 只移除对应 `host_link`；若该 host installation 仍绑定其他 account，则 token 保持有效并向 host 下发 link-state refresh。若这是最后一个 link，则撤销 `host_installation_tokens` 并向 `/ws/host` 发 `host_force_close` 控制帧。
- browser installation 是 account installation kind 的一种，凭证携带方式（access token 存放、跨标签共享、CSRF 防护）由 phase 2 实现细节决定，不影响公开合同；任何 browser-only 凭证形态都不得反向定义业务 principal 类型。

### Rate Limit and Connection Policy

Redis bucket 限流默认作用于以下路径，阈值由 `config.rs` 配置：

- `/v1/auth/register`
- `/v1/auth/login`
- `/v1/auth/refresh`
- `/v1/auth/change-password`
- `/v1/realtime/ws-ticket`
- `/v1/host/bootstrap/nonce`
- `/v1/host/pairing/request-code`
- `/v1/host/pairing/redeem`
- `/v1/pairing/confirm`
- `/v1/conversations/send-message`
- `/v1/agent-sessions/send-input`
- `/v1/approvals/respond`

连接策略固定为：

- 同一 `(principal, installation_id)` 仅保留最新 WS 连接。
- reconnect replace 是 gateway 契约，而不是当前实现偶然行为。
- 超限连接、重复连接替换、未授权订阅都必须暴露稳定的 close / error code，并纳入 metrics。
- host token 撤销后的强制断连通过 `host_force_close` 控制帧完成，close code 固定为 4401。

最小控制帧：

```json
{ "type": "host_force_close", "reason": "token_revoked", "close_code": 4401 }
```

### Realtime and Async Execution Model

- durable 领域事件以“数据库事务 + `durable_event_log` + `outbox_events` + dispatcher”完成，提供 at-least-once 投递、审计与 replay。
- ephemeral stream 事件直接走 Redis pub/sub 或本地 gateway channel；持久恢复依赖 `agent_turns` / stream slices，不写 `outbox_events`。
- client gateway 只负责 ticket 校验、subscription auth、durable replay 与 live delivery；host gateway 只负责 host command delivery、host uplink 与 host-scoped presence。
- approval timeout、host command timeout、refresh cleanup、stale session recovery 都由 worker plane 处理，不挂在 HTTP 请求路径上。
- 任何需要“先改状态、再推送”的用例，都必须先在事务内写 durable state、`durable_event_log` 与 `outbox_events`，再由 dispatcher fan-out；不允许在事务外直接 `publish_durable`。
- host gateway 必须支持 `host_force_close` 控制帧：当最后一个 `host_link` 被撤销、host token 被撤销或 host 权限范围被收窄时，gateway 收到控制帧后以 4401 主动断开连接，并写审计事件。

## Phased Implementation

### Phase 1: Application Shell and Formal Contracts

**File: `crates/minos-backend/src/lib.rs`**
- 收缩对外导出，只暴露 `app`, `http`, `realtime`, `store`, `jobs`, `telemetry`。
- 取消 crate root 对具体 use-case 的直接拼装。

**File: `crates/minos-backend/src/app/context.rs`**
- 新增 `AppContext`、`RepositorySet`、service wiring。
- 明确 public API / host API / gateways / workers 共用的依赖注入入口。

**File: `crates/minos-backend/src/config.rs`**
- 新增 Postgres、Redis、OTel、worker toggles、rate-limit、retention、gateway caps 配置。
- 明确 worker 是否单独运行只是部署配置，不是代码分叉。

**File: `crates/minos-backend/src/http/mod.rs`**
- Router 只负责 transport mapping，不再直接持有领域拼装细节。
- 把 auth、request-id、error mapping、OpenAPI/JSON schema 暴露固化为平台层能力。

验收：

- contract types 可以稳定导出并驱动 OpenAPI / JSON schema 生成。
- config parse / default tests 与 request-id middleware tests 落地，不把合同验证推迟到 phase 7。

### Phase 2: Identity, Installations, and Realtime Tickets

**File: `crates/minos-backend/src/auth/mod.rs`**
- 拆成 account auth、host bootstrap proof、refresh rotation、host installation token、realtime ticket 子模块。
- 把 rate-limit policy 抽成配置化依赖。

**File: `crates/minos-backend/src/http/v1/auth.rs`**
- 只保留 account bearer public API。
- 移除 header-based business identity 拼装。

**File: `crates/minos-backend/src/http/v1/host.rs`**
- 新增 host rail：bootstrap 子面负责 `pairing/request-code`, `pairing/redeem`；steady-state 子面负责 `realtime/ws-ticket`, `installations/self`。

**Files: `crates/minos-backend/src/http/v1/pairing.rs`, `crates/minos-backend/src/http/v1/host.rs`**
- 正式开发合同中不包含 `/v1/me/*`；caller-scoped 视图分别并入 pairing 和 host self 接口。

**File: `crates/minos-backend/src/http/ws_devices.rs`**
- 在同一模块中承载 `/ws/client` 与 `/ws/host` 的共享 upgrade / activation 实现。
- 退役顶层 `/devices` 公开路由，并把测试 / helpers 迁到 ticket flow。
- 暂时保留 mixed auth 与 `ws_ticket` 兼容分支，留给后续内部清理 slice。

**File: `crates/minos-backend/src/store/auth/*.rs`**
- 新增 `account_credentials`, `device_installations`, `refresh_tokens`, `host_installation_tokens` repos。
- Redis ticket store 只保留在 runtime adapter，不落 SQL `ws_tickets` 表。

验收：

- `/v1/auth/*` 与 `/v1/host/*` 的合同测试可独立跑通。
- `/ws/client` 与 `/ws/host` handshake / duplicate-connection replace 测试落地。
- 未授权 principal 无法跨 rail 调用接口。
- bootstrap proof（Ed25519 签名 + server-issued nonce + TOFU 公钥登记）、pairing redeem 与 steady-state host token 三种凭证不可混用。
- `host_installation_tokens.revoked_at_ms` 校验在每次请求时生效；负缓存窗口不大于 5 秒。

### Phase 3: Pairing and Host Link Domain

**File: `crates/minos-backend/src/pairing/mod.rs`**
- 重写为 `request_code`, `confirm_pairing`, `redeem_host_installation`, `revoke_link`, `list_links` 五个用例。
- pairing confirm 必须同时创建 `host_links` 或保持幂等，不再依赖 transport header 角色判断。

**File: `crates/minos-backend/src/http/v1/pairing.rs`**
- 只保留 account rail 的 confirm / revoke / list-hosts。

**File: `crates/minos-backend/src/http/v1/host.rs`**
- host rail 承担 request-code / redeem。

**File: `crates/minos-backend/src/store/pairing/*.rs`**
- 新增 `pairing_codes` 与 `host_links` repositories。
- pairing code 消费、host token 发放、host link 建立统一走数据库事务。

验收：

- 同一 `(account_id, host_installation_id)` 重复 confirm 是幂等的。
- 多 account 链接同一 host installation 的场景有明确测试，不依赖口头约定。
- host revoke 后 ticket 与 host token 行为符合预期。
- 最后一个 link 被撤销时，host token 撤销与 `/ws/host` force close 有端到端测试。

### Phase 4: Agent Sessions, Approvals, and Ingest

**File: `crates/minos-backend/src/agent_sessions/mod.rs`**
- 新增 agent session 领域模块，定义 session lifecycle、turn、read-turns、resume、shutdown。
- turn 级 durable 与 turn 内 stream slice 分别落 `agent_turns` 和 `agent_turn_events`，两条序列空间独立但共用 read API。

**File: `crates/minos-backend/src/store/agent_turns.rs`**
- 新增 `agent_turns` 与 `agent_turn_events` repositories，提供 turn 提交、stream slice append、cold replay 与 retention 清理语义。

**File: `crates/minos-backend/src/approval_relay.rs`**
- 收敛为 approval domain service adapter，不再直接承担 transport 相关逻辑。
- timeout 与 disconnect 统一转成 `ApprovalResolved` + `host_commands`。

**File: `crates/minos-backend/src/host_commands/mod.rs`**
- 新增 host command 领域模块，承接所有“后端请求 host 并可能等待结果”的路径。

**File: `crates/minos-backend/src/ingest/mod.rs`**
- 改成“持久化原始输入 + 生成 durable 领域事件 + 发出 ephemeral stream”的纯用例。
- 不再直接查询在线 peer 并扇出。

**File: `crates/minos-backend/src/http/v1/agent_sessions.rs`**
- 新增 start / send-input / stop / list / read-turns endpoints。

**File: `crates/minos-backend/src/http/v1/approvals.rs`**
- 新增专属 approvals/respond 入口，从旧 sessions 入口彻底解耦。

验收：

- `agent-sessions/start`、`send-input` 的幂等语义可测。
- `approvals/respond` 对应的 `ApprovalResolved` 与 `host_command` 状态机一致。
- stream replay 通过 `read-turns` 恢复，不要求 gateway 回放高频 delta。
- `read-turns(after_turn_seq)` 与 `agent_session:<id>.topic_seq` 对齐关系（turn 级 durable 序列）有显式测试保护。
- `read-turns(turn_id, after_event_seq)` 能够还原已关闭 session 的 stream slice，且 `agent_turn_events` retention 策略有清理测试。

### Phase 5: Conversations, Social, and Projects

**File: `crates/minos-backend/src/social/mod.rs`**
- 只保留 profile / friendship / conversation 领域聚合。
- agent 作为 conversation member 的模型并入 conversation domain，而不是散落在 HTTP 层。

**File: `crates/minos-backend/src/http/v1/social.rs`**
- 拆成 `profiles.rs`, `friendships.rs`, `conversations.rs`，减小单文件体积。

**File: `crates/minos-backend/src/project/mod.rs`**
- 扩展成 project aggregate，支持 membership、workspace binding、default agent policy。

**File: `crates/minos-backend/src/http/v1/projects.rs`**
- 保持 POST-first，但将 list / create / rename / archive / conversations/link / agent-sessions/query|link 明确分组。
- 旧 `projects/sessions/*` 退化为兼容别名，新的权威 project 归属写入 `agent_sessions.project_id`。

验收：

- conversation membership 与 project membership 权限边界有合同测试。
- `projects/conversations/link` 与 `agent_sessions.project_id` 一致性受测试保护。
- `projects/agent-sessions/query` 与 `/v1/agent-sessions/list(project_id=...)` 一致性受测试保护。
- conversation sync cursor 行为稳定，不把 replay 问题留到 phase 6 再补。

### Phase 6: Realtime Gateway, Outbox, and Workers

**File: `crates/minos-backend/src/realtime/mod.rs`**
- 从单文件 `realtime.rs` 重构成 `client_gateway`, `host_gateway`, `dispatcher_durable`, `dispatcher_stream`, `subscriptions`, `resume`, `pubsub` 子模块。

**File: `crates/minos-backend/src/host_command_runtime.rs`**
- `host_commands` 是 host 请求/响应的唯一权威路径。
- 相关性、响应恢复与超时完全交由 `host_commands` / `approval_requests` / `agent_turns` 处理。

**File: `crates/minos-backend/src/jobs/*.rs`**
- 新增 outbox dispatcher、host command timeout resolver、approval timeout resolver、refresh cleanup、stale session sweeper。

**File: `crates/minos-backend/src/store/outbox.rs`**
- 新增 `outbox_events` repository，提供 claim / ack / retry 语义。

**File: `crates/minos-backend/src/store/durable_event_log.rs`**
- 新增 `durable_event_log` repository，提供按 `(topic, topic_seq)` 读取与 retention 清理语义。

**File: `crates/minos-backend/src/store/host_commands.rs`**
- 新增 `host_commands` repository，提供 enqueue / ack / finish / timeout 语义。

验收：

- 多实例 fan-out、durable replay、subscription auth、host command timeout 都有独立测试。
- 旧 `/devices` 行为没有残留兼容路径。
- reconnect replace、cursor retention miss、unauthorized subscription 都会打 metrics 与 structured log。
- `durable_event_log` 与 `outbox_events` 的事务双写、独立清理策略有测试保护。

### Phase 7: Observability, Verification, and Cutover

**File: `crates/minos-backend/src/telemetry.rs`**
- 升级为 OTel trace + metric + structured logging 统一入口。
- 每条 request、WS session、agent session、approval request、host command 都具备稳定标识。

**File: `crates/minos-backend/tests/**`**
- 重新按 domain 和 transport 分层：auth, host, pairing, agent_sessions, approvals, conversations, realtime, workers。
- 增加多实例 fan-out、worker timeout、resume/replay、permission regression、contract drift 测试。

**File: `.github/workflows/ci.yml`**
- 增加 Postgres + Redis integration matrix。
- 验证 API tests、gateway tests、worker tests、migration drift、OpenAPI/JSON schema drift。

验收：

- PG + Redis CI matrix 持续绿。
- OpenAPI / JSON schema 成为 SDK 生成输入而不是仅在 CI 中被动检查。
- cutover checklist 明确列出 `/v1/me/*`, `/devices`, `forward_rpc.rs`, `session` 旧词的退场状态。

## Architectural Notes

- Semver impact：对所有内部客户端都是破坏性重写，但这是正式开发阶段有意接受的边界清理。
- Object safety / trait coherence：repository、realtime、host command traits 保持 object-safe，方便在 `AppContext` 中注入真实实现与测试替身。
- Side effects：pairing、approval、agent session start/stop 这类 durable 用例必须以"数据库事务 + durable state + durable_event_log + outbox_events"结束；turn 内 stream slice 必须先 INSERT 到 `agent_turn_events` 再发 ephemeral；只有完全 transient 的进度信号（如 typing indicator）才允许跳过持久层。
- Deployment shape：worker plane 是逻辑角色，不与 deployment 数量绑定。
- Explicitly not changed：语言仍然是 Rust，HTTP/WS 框架仍然优先 Axum/Tokio，API 风格仍保持 POST-first。
- New dependencies：PostgreSQL driver/runtime support、Redis client、OpenTelemetry exporter、OpenAPI/JSON schema generation。
- Removed dependencies/patterns：进程内全局 reply-correlation map、兼容路由、混合 `/devices` gateway、transport-role headers 作为公共业务身份入口、公开契约中的 macOS-only host 命名。

## File Change Summary

- `crates/minos-backend/src/app/context.rs` -- 新增正式开发阶段的依赖注入和运行角色上下文。
- `crates/minos-backend/src/auth/mod.rs` -- 拆分 account auth、host bootstrap proof、host installation token、realtime ticket。
- `crates/minos-backend/src/config.rs` -- 增加 Postgres、Redis、OTel、retention、rate-limit 与运行角色配置。
- `crates/minos-backend/src/forward_rpc.rs` -- 删除 MVP 风格的进程内相关性全局状态。
- `crates/minos-backend/src/host_commands/mod.rs` -- 新增 host command 领域与状态机。
- `crates/minos-backend/src/http/mod.rs` -- 仅保留 transport mapping 和通用 middleware；公开 realtime 路由收敛到 `/ws/client` 与 `/ws/host`。
- `crates/minos-backend/src/http/v1/agent_sessions.rs` -- 新增 agent session 命令与 read-turns 入口。
- `crates/minos-backend/src/http/v1/approvals.rs` -- 新增 approval respond 专属入口。
- `crates/minos-backend/src/http/v1/auth.rs` -- 收敛为 account bearer-first 身份接口。
- `crates/minos-backend/src/http/v1/host.rs` -- 新增 host rail 接口。
- `crates/minos-backend/src/http/v1/pairing.rs`, `crates/minos-backend/src/http/v1/host.rs` -- 承接已退役 `/v1/me/*` 的 pairing / host self surface。
- `crates/minos-backend/src/http/v1/pairing.rs` -- 重写为 account-host link 领域接口。
- `crates/minos-backend/src/http/v1/projects.rs` -- 扩展项目领域接口并移除 `session` 旧词。
- `crates/minos-backend/src/http/v1/social.rs` -- 按 profile / friendship / conversation 子域拆分。
- `crates/minos-backend/src/http/ws_devices.rs` -- 复用共享 realtime upgrade / activation；公开入口收敛到 `/ws/client` 与 `/ws/host`，内部 compat auth 分支留待后续清理。
- `crates/minos-backend/src/ingest/mod.rs` -- 改为持久化 + durable event + ephemeral stream，不直接 fan-out。
- `crates/minos-backend/src/jobs/mod.rs` -- 新增 worker plane 入口。
- `crates/minos-backend/src/pairing/mod.rs` -- 重写 pairing code / host link / host redeem 领域逻辑。
- `crates/minos-backend/src/realtime/mod.rs` -- 重构为 client/host gateway、dispatcher、subscriptions、resume、pubsub 子模块。
- `crates/minos-backend/src/social/mod.rs` -- 收敛 profile、friendship、conversation 领域逻辑。
- `crates/minos-backend/src/store/host_commands.rs` -- 新增 host command persistence。
- `crates/minos-backend/src/store/agent_turns.rs` -- 新增 `agent_turns` 与 `agent_turn_events` persistence，承载 turn 元数据与 stream slice cold replay。
- `crates/minos-backend/src/store/durable_event_log.rs` -- 新增 durable replay log persistence。
- `crates/minos-backend/src/store/outbox.rs` -- 新增 durable outbox persistence。
- `crates/minos-backend/src/store/pairing/*.rs` -- 新增 pairing code 与 host link repositories。
- `crates/minos-backend/src/telemetry.rs` -- 升级为正式开发期可观测性入口。
- `crates/minos-backend/tests/**` -- 重新按 domain / transport / worker 分层验证。
- `.github/workflows/ci.yml` -- 补齐 Postgres + Redis + migration drift + contract drift 验证。
