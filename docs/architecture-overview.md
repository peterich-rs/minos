# Minos Architecture Overview

本文是 Minos 后端 + 客户端协同的系统架构总览，作为 `docs/backend-formal-development.md` 的对外/团队入门视角抽出件。它不替代正式开发文档对接口、数据模型与状态机的权威定义；它给出的是**一张图能讲清"东西放在哪儿、谁向谁说话、以什么节奏说话"**的视角。

> Minos 不是通用 IM。它是「手机/Web 远程操控 Mac 上的 codex / claude / gemini」的 AI agent 控制台，但同时具备完整的 IM 形态（账号、好友、会话、群、提及、已读、project 归档）。后端架构必须同时满足"agent 远程命令面"和"实时社交面"两条主线。

---

## 1. 顶层分层

```
客户端：iOS / Android (Flutter)        macOS 状态栏 + Daemon          Web Admin (React)
       │                                │                              │
       │ HTTPS REST  +  WSS subscribe   │ HTTPS REST + WSS uplink      │ HTTPS REST + WSS subscribe
       ▼                                ▼                              ▼
─────────────────────────  Edge / Ingress (TLS, WAF)  ─────────────────────────
       │
       ▼
   API Gateway 层  ──  鉴权 / 限流 / 请求 ID / CORS / OTel 注入
   ┌────────────────────────────┬──────────────────────────────┐
   │  Public API  (/v1/*)       │  Host API  (/v1/host/*)      │
   │  AccountPrincipal only     │  HostBootstrap / HostInst    │
   └────────────────────────────┴──────────────────────────────┘
       │                                │
       ▼                                ▼
   Realtime Gateway 层  ──  短 TTL ticket → WS upgrade → topic 订阅
   ┌────────────────────────────┬──────────────────────────────┐
   │  /ws/client                │  /ws/host                    │
   │  account / project /       │  host_installation /         │
   │  conversation /            │  agent_session 控制面 +      │
   │  agent_session 订阅        │  uplink                      │
   └────────────────────────────┴──────────────────────────────┘
       │                                │
       ▼                                ▼
   Domain / Use-Case 层  ──  AppContext + RepositorySet 注入
   ┌──────────┬──────────┬──────────┬──────────┬──────────┬──────────┐
   │ Auth     │ Pairing  │ Agent    │ Approval │ Convers- │ Project  │
   │ Service  │ Service  │ Session  │ Service  │ ation /  │ Service  │
   │          │ + Host   │ Service  │          │ Social   │          │
   │          │ Link     │          │          │ Service  │          │
   └──────────┴──────────┴──────────┴──────────┴──────────┴──────────┘
       │                                │
       ▼                                ▼
   Persistence / Stream 双轨
   ┌────────────────────────────┬──────────────────────────────┐
   │  Durable plane             │  Ephemeral plane             │
   │  PostgreSQL 16 + sqlx      │  Redis 7 pub/sub             │
   │   • domain tables          │   • stream slice fan-out     │
   │   • durable_event_log      │   • subscription routing     │
   │   • outbox_events          │   • realtime ticket store    │
   │   • audit_events           │   • rate-limit buckets       │
   └────────────────────────────┴──────────────────────────────┘
       │                                │
       ▼                                ▼
   Worker Plane (supervised tokio tasks，可独立部署)
   ┌──────────────┬──────────────┬──────────────┬──────────────┐
   │ Outbox       │ Approval     │ HostCommand  │ Retention &  │
   │ Dispatcher   │ Timeout      │ Timeout      │ Stale Sweep  │
   └──────────────┴──────────────┴──────────────┴──────────────┘
       │
       ▼
   Notification 层
   ┌──────────────┬──────────────┬──────────────┐
   │ APNs (iOS)   │ FCM (Android)│ Email / SMTP │
   └──────────────┴──────────────┴──────────────┘
       │
       ▼
   Observability  ──  OTel traces + Prometheus metrics + JSON logs (Loki/Tempo)
```

---

## 2. 端到端拓扑

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                                                                              │
│   iOS / Android / Web Admin                            macOS Host Daemon     │
│         │                                                       │            │
│         │ ① POST /v1/auth/login                                 │            │
│         │ ② POST /v1/realtime/ws-ticket                         │            │
│         │ ③ WSS  /ws/client?ticket=...                          │            │
│         │       └─ subscribe  account:<id>                      │            │
│         │                     conversation:<id>                 │            │
│         │                     agent_session:<id>                │            │
│         │                     project:<id>                      │            │
│         │                                                       │            │
│         │                              ④ POST /v1/host/bootstrap/nonce       │
│         │                              ⑤ POST /v1/host/pairing/redeem       │
│         │                              ⑥ POST /v1/host/realtime/ws-ticket   │
│         │                              ⑦ WSS  /ws/host?ticket=...           │
│         │                                  └─ subscribe host:<inst_id>      │
│         ▼                                                       ▼            │
│   ┌─────────────────────────────────────────────────────────────────────┐    │
│   │                     API + Realtime Gateway 集群                     │    │
│   │  (Axum + Tokio, N 实例，前面是 LB / Ingress, 无亲和性)              │    │
│   └─────────────────────────────────────────────────────────────────────┘    │
│         │                       ▲                              │             │
│         │ command               │ subscribe / resume           │ host        │
│         │ (start_agent,         │ (topic, last_durable_seq)    │ command     │
│         │  send_input,          │                              │ delivery    │
│         │  approval respond)    │                              │             │
│         ▼                       │                              ▼             │
│   ┌──────────────────────────────────────────────────────────────────────┐   │
│   │                       Domain / Use-Case 层                           │   │
│   │   AccountAuth  Pairing  AgentSession  Approval  Conversation Project │   │
│   └──────────────────────────────────────────────────────────────────────┘   │
│         │  ┌────────────── DB Tx 内强约束 ──────────────┐  │                 │
│         │  │ 1. write domain row(s)                     │  │                 │
│         │  │ 2. append durable_event_log(topic,seq)     │  │                 │
│         │  │ 3. enqueue outbox_events(event_id)         │  │                 │
│         │  │ 4. (host 命令) enqueue host_commands       │  │                 │
│         │  └────────────────────────────────────────────┘  │                 │
│         ▼                                                  ▼                 │
│   ┌──────────────┐    ┌──────────────────────┐    ┌──────────────────┐       │
│   │ PostgreSQL   │    │ Redis 7              │    │ Stream slice     │       │
│   │  • accounts  │    │  • pubsub bus        │    │ ephemeral path   │       │
│   │  • host_links│    │    minos.<topic>     │    │  • text delta    │       │
│   │  • agent_*   │    │  • ws_ticket:<jti>   │    │  • stdout chunk  │       │
│   │  • approval_ │    │  • ratelimit:<key>   │    │  • diff chunk    │       │
│   │    requests  │    │  • presence:<acct>   │    │  写入 agent_turn │       │
│   │  • host_     │    └──────────────────────┘    │  _events 后再发  │       │
│   │    commands  │              ▲                 └──────────────────┘       │
│   │  • durable_  │              │                                            │
│   │    event_log │              │                                            │
│   │  • outbox_   │              │ Outbox Dispatcher (worker)                 │
│   │    events    │──────────────┘  claim → publish ephemeral → ack           │
│   │  • audit_    │                                                           │
│   │    events    │              ▲                                            │
│   └──────────────┘              │                                            │
│         ▲                       │                                            │
│         │                       │                                            │
│   ┌─────┴───────────────────────┴────────────────────────────────────────┐   │
│   │                          Worker Plane                                │   │
│   │  Outbox Dispatcher │ Retention Cleaner │ Approval Timeout            │   │
│   │  HostCommand TO    │ Stale Session     │ Refresh Token GC            │   │
│   │  Push Fanout       │ Audit Indexer                                   │   │
│   └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                    │
│         ▼                                                                    │
│   ┌──────────────┬──────────────┬──────────────┐                             │
│   │ APNs Push    │ FCM Push     │ SMTP Mailer  │  (离线 / 提及 / 审批超时)  │
│   └──────────────┴──────────────┴──────────────┘                             │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘

  横切面：OpenTelemetry Collector ──► Tempo (trace) / Loki (log) / Prom (metric) ──► Grafana
  配置：Vault / SOPS 管 jwt_secret / db_url / redis_url / apns_key / fcm_key
  部署：API Gateway × N，Realtime Gateway × N，Worker × M，PG primary+replica，Redis cluster
```

---

## 3. 核心组件职责

### 3.1 API Gateway 层

- **Public API (`/v1/*`)** ——只接 `AccountPrincipal`（account bearer + installation_id）。账号、配对确认、conversation、agent session 命令、approval 响应、project 全部在此。
- **Host API (`/v1/host/*`)** ——分两个 sub-surface：
  - bootstrap 子面（`HostBootstrapPrincipal`）：`/v1/host/bootstrap/nonce`、`/v1/host/pairing/request-code`、`/v1/host/pairing/redeem`。
  - steady-state 子面（`HostInstallationPrincipal`）：`/v1/host/installations/self`、`/v1/host/realtime/ws-ticket`。
- 全部端点 POST-first，统一响应包络 `{ data, meta }` / `{ error }`。CORS、request-id、rate-limit、tracing、metrics 在 tower 中间件层。
- 输出：OpenAPI 合同 + 类型生成的 SDK；CI 跑 drift gate。

### 3.2 Realtime Gateway 层

- **`/ws/client`** ——账号侧订阅入口。接受 ticket，握手成功自动订阅 `account:<id>`，其他 topic 显式 `subscribe` 帧。
- **`/ws/host`** ——host 侧订阅 + uplink 入口。握手成功自动订阅 `host:<installation_id>`，承担 host_command 投递、agent runtime 上行、`host_force_close` 控制帧。
- **续传协议**：客户端在 `subscribe` 帧带 `resume_after = { topic: last_durable_seq }`；网关从 `durable_event_log` replay，再切到 live；retention 过期返回 `snapshot_required`，由客户端走 read API 重建。
- **重连/抢占**：同一 `(principal, installation_id)` 仅保留最新连接；旧连接关 4401。
- 网关本身**无状态**：订阅表用 Redis 维护，节点重启不影响其他节点。

### 3.3 Domain / Use-Case 层

- 全部以 service 形式注入到 `AppContext`：`AuthService`、`PairingService`、`HostInstallationService`、`AgentSessionService`、`ApprovalService`、`ConversationService`、`ProjectService`。
- 每个 service 持有 `Arc<dyn ...Repository>`，repository trait 定义在 domain 层，实现在 `store/postgres/` 下。
- **不变量**：所有"先改状态、再推送"的用例必须在同一个 DB 事务内：
  1. 写 domain row
  2. 追加 `durable_event_log(topic, topic_seq)`
  3. 入队 `outbox_events`
  4. （如需）入队 `host_commands`

事务外不允许直接 publish。

### 3.4 Persistence 层 — Durable plane

PostgreSQL 16 + sqlx (runtime-tokio + macros + migrate)。所有写路径要求 `transaction.serializable` 或显式 `SELECT ... FOR UPDATE`。

核心表族：

```
accounts / account_credentials / refresh_tokens
device_installations (mobile / browser / host 三类)
host_installation_tokens / pairing_codes / host_links
projects / project_members / project_default_agents
conversations / conversation_members / conversation_messages
conversation_reads / message_mentions
agents (system catalog)
agent_sessions / agent_turns / agent_turn_events
approval_requests / host_commands
durable_event_log / outbox_events
audit_events
```

**双序列模型**：
- `agent_turns(turn_seq UNIQUE per session)` → `topic_seq` 走 `durable_event_log`，与 `agent_session:<id>` topic 对齐。
- `agent_turn_events(turn_id, event_seq PK)` → cold replay 唯一来源，不进 `durable_event_log`，由 `read-turns(turn_id, after_event_seq)` API 暴露。

### 3.5 Persistence 层 — Ephemeral plane

Redis 7 cluster，五类 key：

```
minos:pubsub.<topic>             pubsub channel，跨实例 fan-out
minos:ticket:<jti>               WS ticket，单次 consume，60s TTL
minos:ratelimit:<bucket>:<key>   token bucket，秒/分/小时窗口
minos:presence:<account_id>      Set of (installation_id, gateway_node_id)
minos:subs:<gateway_id>:<conn>   Set of topics this conn subscribes to
```

Redis 不是真理来源；丢数据只影响"实时性"，不影响"正确性"。

### 3.6 Worker Plane

Supervised tokio tasks，按 `runtime_mode` 决定是否启动。

| Worker | 周期 / 触发 | 作用 |
|---|---|---|
| Outbox Dispatcher | tight loop + Redis NOTIFY | 把 `outbox_events.pending` claim → publish 到 Redis pubsub → ack |
| Retention Cleaner | 每 10 分钟 | 清理 `durable_event_log` 与 `agent_turn_events` 过期行（先 LEFT JOIN outbox 跳过 unacked） |
| Approval Timeout Resolver | event-driven + 兜底 | `pending` 超时 → resolve `Timeout`，写 host_command 通知 host |
| Host Command Timeout | 同上 | 标记超时，late reply 走 grace 期 |
| Stale Session Sweeper | 每 5 分钟 | gateway 心跳过期连接清理 |
| Refresh Token GC | 每 1 小时 | revoke / expire 行清理 |
| Push Fanout | 订阅 `account:*` durable | 离线时把 ConversationMessageAppended / ApprovalRequested 转 APNs/FCM |
| Audit Indexer | 每分钟 | 把 audit_events 备份到冷存储（S3） |

### 3.7 Notification 层

- **APNs**：基于 `a2`/`apns2` crate，使用 token-based auth (.p8 key)；topic 默认 `dev.minos.app`。
- **FCM**：基于 `fcm` crate 或 HTTP v1 endpoint；service-account JSON 加载到 secret store。
- **SMTP**：`lettre` crate，仅用于审批超时与异常告警，不参与正常聊天通知。

由 Push Fanout worker 决定何时推送（presence + user preference + cool-down）。

### 3.8 Observability

- **Tracing**：`tracing` + OTel SDK，Span 维度：`http_request`、`ws_session`、`agent_session`、`host_command`、`approval_request`、`outbox_dispatch`。trace_id 注入响应头 `x-request-id`。
- **Metrics**：Prometheus，分类：
  - HTTP：`http_requests_total{route,status}`、`http_request_duration_seconds`
  - WS：`ws_connections{role}`、`ws_close_total{role,reason}`、`ws_subscriptions{topic_kind}`
  - Worker：`outbox_dispatch_total{result}`、`outbox_dispatch_lag_seconds`、`approval_timeout_total`、`host_command_timeout_total`
  - Domain：`agent_sessions_active`、`approvals_pending`、`durable_event_log_size{topic_kind}`
- **Logs**：JSON 格式，包含 `trace_id`、`account_id?`、`installation_id?`、`session_id?`。

---

## 4. 订阅 / 续传契约

### 4.1 Topic 命名

```
account:<account_id>            ── 账号级通知（friend request、host link 变更、push echo）
conversation:<conversation_id>  ── 会话内消息流 + 已读
project:<project_id>            ── project 级 agent session 索引、归档变更
agent_session:<session_id>      ── turn 元数据（durable，topic_seq = turn_seq）
host:<host_installation_id>     ── host command 投递 + force_close
```

权限规则：
- account 可订阅与自己账号 / membership / project scope 一致的 topic。
- host 只可订阅 `host:<self>` 与 backend 明确下发的 `agent_session:*`。
- 单连接最多 128 个 live subscriptions；单次 `subscribe` 最多 32 个 topic。

### 4.2 WS frame 形态

```jsonc
// → 客户端发
{ "type": "subscribe", "topics": [...], "resume_after": { "<topic>": <topic_seq> } }
{ "type": "unsubscribe", "topics": [...] }
{ "type": "ping", "ts": 1760000000000 }

// ← 服务端发
{ "type": "subscribe_ack", "topics": [...] }
{ "type": "subscription_denied", "topic": "...", "reason": "forbidden|limit_exceeded" }
{ "type": "durable_event", "topic": "...", "topic_seq": 128, "kind": "...", "payload": {...} }
{ "type": "stream_event",  "topic": "...", "kind": "agent_text_delta", "seq": 42, "payload": {...} }
{ "type": "snapshot_required", "topic": "..." }
{ "type": "host_force_close", "reason": "token_revoked", "close_code": 4401 }
{ "type": "pong", "ts": 1760000000000 }
```

---

## 5. 配对（Pairing / Host Bootstrap）流

```
host                                                         backend                           account (mobile)
 │                                                              │                                     │
 │ ─① POST /v1/host/bootstrap/nonce  (installation_id) ────────►│                                     │
 │ ◄────────────── { nonce, expires_at_ms }  ───────────────────│                                     │
 │ ─② POST /v1/host/pairing/request-code                        │                                     │
 │      (installation_id, nonce, public_key?, signature) ──────►│  TOFU 登记 + pairing_codes(pending) │
 │ ◄──────────── { pairing_code, expires_at_ms } ──────────────│                                     │
 │   显示二维码 ────────────────────────────────────────────────┼─── 扫码 ────────────────────────────►│
 │                                                              │ ─③ POST /v1/pairing/confirm        │
 │                                                              │      (pairing_code) ◄──────────────┤
 │                                                              │  pairing_codes pending → confirmed │
 │                                                              │  insert host_links(account, host)  │
 │ ─④ POST /v1/host/pairing/redeem                              │                                    │
 │      (installation_id, nonce, pairing_code, signature) ────►│  pairing_codes confirmed → redeemed │
 │ ◄────── { host_installation_token } ─────────────────────────│                                    │
 │                                                              │                                    │
 │ ─⑤ POST /v1/host/realtime/ws-ticket  (host token) ──────────►│                                    │
 │ ◄────── { ticket, gateway_url } ─────────────────────────────│                                    │
 │ ─⑥ WSS /ws/host?ticket=...   subscribe host:<inst_id> ──────►│                                    │
```

签名 payload 固定为 `installation_id + ":" + nonce + ":" + path`。`pairing_codes.status` 状态机 `pending → confirmed → redeemed`，重复使用一律 `pairing_code_invalid` 并写审计。

---

## 6. Agent 命令 / 审批闭环

```
mobile                  backend                                                    host daemon
  │                        │                                                            │
  │ POST /v1/agent-        │  TX:                                                       │
  │ sessions/start ───────►│   write agent_sessions(pending)                            │
  │                        │   write agent_turns(turn_seq=0, role=system)               │
  │                        │   append durable_event_log(agent_session:<id>, seq=0)      │
  │                        │   enqueue outbox_events                                    │
  │                        │   enqueue host_commands(method=start_agent, deadline)      │
  │                        │  COMMIT                                                    │
  │ ◄── 202 + receipt ─────│                                                            │
  │                        │── Outbox Dispatcher → Redis pubsub minos.host:<inst> ─────►│
  │                        │                                                            │ exec codex/claude
  │                        │ ◄── host_command_ack { command_id } ───────────────────────│
  │                        │ ◄── host_command_result { command_id, ok, session_id } ───│
  │                        │  TX: finish host_commands, update agent_sessions(running)  │
  │                        │      write agent_turns / agent_turn_events streaming       │
  │                        │      durable + ephemeral 双发                              │
  │ ◄── stream_event(text  │                                                            │
  │     delta) on /ws ─────│                                                            │
  │                        │                                                            │
  │  审批请求由 host 触发，写 approval_requests + durable_event_log,                    │
  │  POST /v1/approvals/respond → host_commands(method=approval_decision)               │
  │  超时/断线 → Worker Plane 自动 resolve + ApprovalResolved 事件                      │
```

---

## 7. 部署形态

```
                     ┌──────────────────┐
                     │  Cloud LB / TLS  │
                     └─────────┬────────┘
                               │
        ┌──────────────────────┼──────────────────────┐
        ▼                      ▼                      ▼
   ┌─────────┐            ┌─────────┐            ┌─────────┐
   │ API     │ × N        │Realtime │ × N        │ Worker  │ × M
   │ Gateway │            │ Gateway │            │ Plane   │
   │(http-   │            │(http-   │            │(worker- │
   │ only)   │            │ only)   │            │ only)   │
   └─────────┘            └─────────┘            └─────────┘
        │                      │                      │
        └──────────────────────┼──────────────────────┘
                               ▼
                    ┌────────────────────┐
                    │ PostgreSQL 16      │  primary + read replicas + pgBouncer
                    └────────────────────┘
                               │
                    ┌────────────────────┐
                    │ Redis 7 cluster    │  pubsub + ticket + ratelimit
                    └────────────────────┘
                               │
                    ┌────────────────────┐
                    │ APNs / FCM / SMTP  │
                    └────────────────────┘
```

**部署滑块**：

| 阶段 | API | Realtime | Worker | DB | Redis |
|---|---|---|---|---|---|
| 本地开发 | monolith × 1 | (同上) | (同上) | sqlite | inline |
| 单机 prod | monolith × 1 | (同上) | (同上) | PG | redis |
| 中等流量 | http-only × N | (同上 + ws) | worker-only × M | PG primary+replica | redis cluster |
| 高流量 | http-only × N | http-only(ws-only) × N | worker-only × M | PG primary+replica + 分库 | redis cluster + sentinel |

---

## 8. 安全模型

| 凭证 | 谁持有 | 作用域 | TTL | 存储位置 |
|---|---|---|---|---|
| Account access token (JWT) | mobile / browser | Public API + WS ticket | ≤15m | 客户端内存 |
| Account refresh token | 同上 | 旋转换 access | 30d 滑动 | 客户端 secure storage + DB |
| Host installation keypair (Ed25519) | host daemon | bootstrap 阶段签名 | 永久（可重新走完整三步流轮换） | host secure storage / file |
| Host installation token | host daemon | Host API + WS ticket | 长期（直到撤销） | host secure storage |
| WS ticket (JWT) | 任意 client | 单次升级 WS | 60s | 客户端内存 + Redis 单次 consume |
| Bootstrap nonce | host daemon | 单次 bootstrap 请求 | 60s | Redis |

强制下线：account 走 refresh rotation + access 自然过期；host 走 `host_installation_tokens.revoked_at_ms` + `host_force_close` 控制帧。

---

## 9. 核心约束 / 不变量

1. **公开 `/v1/*` 只接 `AccountPrincipal`**。host daemon 不能进入 public API。
2. **所有 durable 用例事务内三件事**：写 domain row、append `durable_event_log`、enqueue `outbox_events`。事务外发布是 bug。
3. **stream slice 必须先 INSERT 再发**：`agent_turn_events` 写入 → `StreamEvent` 发布 → 客户端 cold replay 才能恢复。
4. **`host_commands` 是 host 同步请求的唯一权威**。in-process notifier 仅是优化缓存，不是真理。
5. **同一 `(principal, installation_id)` 只保留最新 WS 连接**。重连必须重新申请 ticket。
6. **平台不写在公共合同里**。host 是跨平台安装形态；macOS/Windows/Linux 仅在 `device_installations.platform` 体现。
7. **客户端续传游标按 topic 维度**：`{ topic: last_durable_seq }`，不是单一序列。

---

## 10. 与既有文档关系

| 文档 | 角色 |
|---|---|
| `docs/architecture-overview.md` （本文） | 架构总览 + 拓扑图 + 分层职责 |
| `docs/backend-formal-development.md` | 接口、数据模型、状态机、phase 划分的权威定义 |
| `docs/backend-implementation-plan.md` | 基于本文 + formal-development 的可执行实施计划，分 phase + slice + 验收 |
| `docs/adr/*.md` | 单点决策记录 |
| `docs/ops/*.md` | 运维 runbook |

后续如有架构层面的破坏性变更，先改本文档与 formal-development，再开新 ADR。
