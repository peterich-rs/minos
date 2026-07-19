# Minos Backend Implementation Plan

本文是 `docs/architecture-overview.md` 与 `docs/backend-formal-development.md` 之后的**详细落地实施计划**。它以 RFP 标准编写，每个 slice 给出：

- 改动文件清单（含路径、新增/修改/删除）
- 接口签名 / SQL DDL / wire 帧形态
- 算法或事务流程的伪码
- 显式错误码 + 错误模型
- 必跑测试矩阵（单元 / 集成 / 性能 / 故障注入）
- CI 门禁与 lint 规则
- 回退步骤与失败兜底

> 阅读顺序：先 `architecture-overview.md` → `backend-formal-development.md` → 本文。三者中任何一份的更新都被视作对实施计划的破坏性变更，需要在同一 PR 内同步。

---

## 0. 总则

### 0.1 工程价值观

- **架构总览先固化，再编码**：任何 slice 在 PR 中改 `architecture-overview.md` / `backend-formal-development.md` 必须当作 breaking change，需要更新对应 ADR。
- **每个 slice 必须可独立通过 CI、可独立 revert**：slice 之间允许串行依赖，但绝不允许"同一 PR 跨多个 slice"。
- **测试强制写在 slice 内**："下个 slice 补测试"是反模式。每个 slice 在"验收"段会列出必须新增的测试文件。
- **`thread` / `/v1/me/*` / `X-Device-*` / `forward_rpc` 是退场对象**：任何 slice 不允许新增对它们的引用；删除它们的 slice 在本文中显式列出（P8）。
- **所有 durable 用例必须事务三件套**：写 domain row、append `durable_event_log`、enqueue `outbox_events`。slice 评审会把它当作 mechanical check（自定义 clippy 或 PR template）。

### 0.2 命名约定

- crate 内部模块路径以 `crates/minos-backend/src/` 为基准，省略时默认在该 crate。
- migration 文件命名：
  - SQLite：`migrations/sqlite/NNNN_<slug>.sql`
  - Postgres：`migrations/postgres/NNNN_<slug>.sql`
  - 序列号在两类下独立递增；Postgres 从 `0001_baseline.sql` 起步。
- 测试文件命名：`tests/<area>_<scenario>.rs`，与 module 边界对齐。
- HTTP route 在路由表（`http::formal_route_inventory`）中必须按 `(method, path, surface, auth)` 四元组登记。

### 0.3 依赖与基线

- Rust 1.80+，sqlx 0.8（`postgres` + `sqlite` features 共存），Axum 0.7，Tokio 1。
- Redis 7+，PostgreSQL 16+。
- 新增 crate 依赖（一次性更新到 workspace `Cargo.toml`，本节作为 P0.S0 的产物）：
  - `redis = { version = "0.27", features = ["tokio-comp", "connection-manager", "cluster-async"] }`
  - `deadpool-redis = "0.18"`
  - `apns2 = "0.7"`
  - `fcm = "0.10"`（HTTP v1 接口）
  - `lettre = { version = "0.11", features = ["tokio1-rustls-tls", "smtp-transport"] }`
  - `opentelemetry = "0.27"`
  - `opentelemetry-otlp = { version = "0.27", features = ["tonic"] }`
  - `tracing-opentelemetry = "0.28"`
  - `prometheus = "0.13"`
  - `utoipa = { version = "5", features = ["axum_extras", "chrono", "uuid"] }`
  - `schemars = "0.8"`（WS frame schema 生成）

### 0.4 Phase 总览与依赖图

```
P0 ── P1 ── P2 ── P3 ── P4
              │     │     │
              └──── P5 ── P6
                    │
                    P7
                    │
                    P8 ── P9
```

| Phase | 主题 | 依赖 | 估时（人日） |
|---|---|---|---|
| P0 | 基线对齐 + Postgres-first 切换 + AppContext + docs lint | — | 8 |
| P1 | Durable + Outbox 双写底座 | P0 | 14 |
| P2 | Realtime Gateway 重构 | P1 | 20 |
| P3 | Domain refactor：agent_session / approval / host_command | P2 | 16 |
| P4 | Conversation / Project / Social 收敛 | P3 | 12 |
| P5 | Worker Plane 全员上线 | P3 | 10 |
| P6 | Notification（APNs/FCM/SMTP） | P5 | 10 |
| P7 | Observability（OTel + Prom + 日志） | P5 | 6 |
| P8 | 退场清理（thread/me/X-Device/兼容路由） | P0–P7 | 8 |
| P9 | 多实例 / Postgres-only / SDK / contract drift | P0–P8 | 10 |

每个 phase 内部按 slice 拆分；slice 编号 `P<phase>.S<slice>`。每个 slice 估时单位为半天（4h），细化在每个 slice 的"工作量"段。

### 0.5 错误模型基线

整个后端统一错误响应：

```json
{
  "error": {
    "code": "snake_case_kind",
    "message": "human readable",
    "request_id": "req_01J...",
    "retry_after_ms": 1500
  }
}
```

错误码命名空间（每个 slice 在引入新错误码时必须更新此表，作为 PR review checklist）：

| 命名空间 | 用途 |
|---|---|
| `auth_*` | 鉴权失败：`auth_invalid_credentials` / `auth_token_expired` / `auth_refresh_reuse` / `auth_weak_password` |
| `pairing_*` | 配对：`pairing_code_invalid` / `pairing_state_mismatch` / `pairing_throttled` |
| `host_*` | host rail：`host_bootstrap_proof_invalid` / `host_bootstrap_throttled` / `host_token_revoked` |
| `realtime_*` | WS / ticket：`realtime_ticket_invalid` / `realtime_subscription_denied` / `realtime_subscription_limit_exceeded` / `realtime_snapshot_required` |
| `agent_session_*` | agent session：`agent_session_not_found` / `agent_session_state_invalid` / `agent_session_host_unavailable` |
| `approval_*` | approval：`approval_not_found` / `approval_already_resolved` / `approval_deadline_passed` |
| `host_command_*` | host command：`host_command_timeout` / `host_command_rejected` |
| `conversation_*` | conversation / message：`conversation_forbidden` / `conversation_message_recall_window_passed` |
| `project_*` | project：`project_workspace_conflict` / `project_archived` |
| `validation_*` | 通用入参校验：`validation_missing_field` / `validation_format` |
| `rate_limited` | 限流（统一 code，附 `retry_after_ms`） |
| `internal` | 兜底；记录 trace_id 但不暴露细节 |

HTTP status 映射（粗粒度，service 层不直接生成 status，由 `error_response.rs` 中的映射器转换）：

- 4xx：`auth_*` / `validation_*` / `*_invalid` / `*_not_found` / `*_state_*`
- 409：`*_conflict` / `*_already_resolved`
- 429：`rate_limited` / `*_throttled`
- 5xx：`internal`

### 0.6 通用响应包络

成功：

```json
{
  "data": { ... },
  "meta": {
    "request_id": "req_01J...",
    "next_cursor": null
  }
}
```

`meta.request_id` 来自 tower middleware，复用响应头 `x-request-id`。

### 0.7 Mechanical lint（自定义）

落地 P0.S3 的 `cargo xtask lint-conventions`，包含：

1. **三件套 lint**：扫描 `*Service::*` 内所有 `async fn`，若出现 `BeginTransaction` / `pool.begin().await`，必须同时出现 `durable_event_store.record(` 与 `outbox_repo.enqueue(`。允许通过 `#[lint::durable_skip = "reason"]` 显式豁免。
2. **退场词 lint**：禁用 `thread_id` / `X-Device-` / `/v1/me/` / `forward_rpc` / `paired_with` 在 P3 之后被新增。
3. **错误码 lint**：扫描 `err("..."`，要求出现的 code 必须在 `errors.toml` 注册表中。
4. **路由 lint**：每个 axum `.route(` 必须在 `formal_route_inventory()` 有对应条目。

实现位置：`xtask/src/lints/`。

---

## P0 — 基线对齐 + Postgres-first + AppContext + docs lint

**Phase 目标**：把"运行时存储默认 Postgres、SQLite 仅 dev/test"钉死；建立 `AppContext` / `RepositorySet` 注入边界；建立文档 drift gate。

**Phase 完成定义**：

- `MINOS_STORAGE_MODE=external-sql` 的二进制可以在 docker-compose（PG + Redis）上跑通 `cargo test -p minos-backend --features backend-postgres`。
- 每个 service 都通过 `Arc<dyn Repository>` 注入，不再直接拿 `SqlitePool`。
- CI 增加 docs-lint job，故意让 architecture-overview 和 plan 互相不一致会 red。

### P0.S0 — 依赖基线（半天）

**改动**：

- `Cargo.toml` workspace dependencies 增补 §0.3 列表
- `crates/minos-backend/Cargo.toml`：features 重写为 `default = ["backend-sqlite", "backend-postgres"]`、`backend-sqlite = []`、`backend-postgres = []`、`test-support = []`
- `deny.toml`：把新增 crate 加入白名单；阻止 `tokio-postgres` 直接依赖（强制走 sqlx）

**验收**：

- `cargo build -p minos-backend --no-default-features --features backend-sqlite` 通过
- `cargo build -p minos-backend --no-default-features --features backend-postgres` 通过
- `cargo deny check` 通过

### P0.S1 — Postgres baseline migration（2 天）

**目标**：建立独立于 SQLite 的 PG schema，不再"复用 SQLite 文本"。

**改动**：

- 新增 `crates/minos-backend/migrations/postgres/0001_baseline.sql`
- 把现 `migrations/0001_initial.sql` 移入 `migrations/sqlite/0001_initial.sql`
- 修改 `src/store/mod.rs`：
  - `connect_with_options` 拆为 `connect_sqlite_with_options` 与 `connect_postgres_with_options`
  - 引入 `MigrationVariant::{Sqlite, Postgres}`，`sqlx::migrate!("./migrations/postgres")` 由 feature 选择
- 修改 `xtask`：`backend-db-reset` 接受 `--driver sqlite|postgres`

**Postgres 0001_baseline.sql 完整 DDL（按主题分块）**：

```sql
-- 0. 启用扩展
CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- 1. accounts / credentials
CREATE TABLE accounts (
    account_id        TEXT PRIMARY KEY,
    email             CITEXT NOT NULL UNIQUE,
    minos_id          TEXT UNIQUE,
    display_name      TEXT,
    created_at_ms     BIGINT NOT NULL,
    last_login_at_ms  BIGINT
);

CREATE TABLE account_credentials (
    account_id        TEXT PRIMARY KEY REFERENCES accounts(account_id) ON DELETE CASCADE,
    password_hash     TEXT NOT NULL,
    updated_at_ms     BIGINT NOT NULL
);

-- 2. installations
CREATE TYPE installation_kind AS ENUM ('mobile', 'browser', 'host');

CREATE TABLE device_installations (
    installation_id   TEXT PRIMARY KEY,
    kind              installation_kind NOT NULL,
    platform          TEXT,                 -- 'ios','android','web','macos','windows','linux'
    public_key        TEXT,                 -- only host (Ed25519 base64-url)
    account_id        TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    display_name      TEXT,
    created_at_ms     BIGINT NOT NULL,
    last_seen_at_ms   BIGINT NOT NULL,
    CONSTRAINT installation_kind_account_consistency CHECK (
        (kind IN ('mobile','browser') AND account_id IS NOT NULL AND public_key IS NULL) OR
        (kind = 'host' AND account_id IS NULL AND public_key IS NOT NULL)
    )
);
CREATE INDEX idx_installations_account ON device_installations(account_id) WHERE account_id IS NOT NULL;

-- 3. refresh tokens
CREATE TABLE refresh_tokens (
    token_hash        TEXT PRIMARY KEY,
    account_id        TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    installation_id   TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    issued_at_ms      BIGINT NOT NULL,
    expires_at_ms     BIGINT NOT NULL,
    revoked_at_ms     BIGINT,
    rotated_to_hash   TEXT REFERENCES refresh_tokens(token_hash) ON DELETE SET NULL
);
CREATE INDEX idx_refresh_active ON refresh_tokens(account_id, installation_id) WHERE revoked_at_ms IS NULL;

-- 4. host installation tokens
CREATE TABLE host_installation_tokens (
    token_hash             TEXT PRIMARY KEY,
    host_installation_id   TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    issued_at_ms           BIGINT NOT NULL,
    last_used_at_ms        BIGINT,
    revoked_at_ms          BIGINT
);
CREATE INDEX idx_host_token_active ON host_installation_tokens(host_installation_id) WHERE revoked_at_ms IS NULL;

-- 5. pairing codes
CREATE TYPE pairing_status AS ENUM ('pending','confirmed','redeemed','expired');

CREATE TABLE pairing_codes (
    code_hash                  TEXT PRIMARY KEY,
    host_installation_id       TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    account_id                 TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    linked_via_installation_id TEXT REFERENCES device_installations(installation_id) ON DELETE SET NULL,
    status                     pairing_status NOT NULL,
    client_request_id          TEXT,
    created_at_ms              BIGINT NOT NULL,
    expires_at_ms              BIGINT NOT NULL,
    confirmed_at_ms            BIGINT,
    redeemed_at_ms             BIGINT
);
CREATE INDEX idx_pairing_codes_host_status_created
    ON pairing_codes(host_installation_id, status, created_at_ms DESC);
CREATE INDEX idx_pairing_codes_expires
    ON pairing_codes(expires_at_ms)
    WHERE status IN ('pending','confirmed');

-- 6. host links (多账号 ↔ 多 host installation)
CREATE TABLE host_links (
    pair_id                    TEXT PRIMARY KEY,
    account_id                 TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    host_installation_id       TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    linked_via_installation_id TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    link_display_name          TEXT,
    acl_json                   JSONB NOT NULL DEFAULT '{}'::jsonb,
    paired_at_ms               BIGINT NOT NULL,
    UNIQUE (account_id, host_installation_id)
);
CREATE INDEX idx_host_links_account ON host_links(account_id);
CREATE INDEX idx_host_links_host    ON host_links(host_installation_id);

-- 7. agents catalog（系统目录，slug 主键）
CREATE TABLE agents (
    agent_id      TEXT PRIMARY KEY,           -- 'agent_codex','agent_claude','agent_gemini'
    runtime_kind  TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    description   TEXT,
    enabled       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at_ms BIGINT NOT NULL
);
INSERT INTO agents(agent_id, runtime_kind, display_name, created_at_ms) VALUES
    ('agent_codex',  'codex',  'Codex',  EXTRACT(EPOCH FROM now())*1000),
    ('agent_claude', 'claude', 'Claude', EXTRACT(EPOCH FROM now())*1000),
    ('agent_gemini', 'gemini', 'Gemini', EXTRACT(EPOCH FROM now())*1000);

-- 8. projects
CREATE TABLE projects (
    project_id      TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    workspace_root  TEXT NOT NULL,
    created_at_ms   BIGINT NOT NULL,
    updated_at_ms   BIGINT NOT NULL,
    archived_at_ms  BIGINT,
    UNIQUE(account_id, workspace_root)
);

CREATE TABLE project_members (
    project_id   TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    account_id   TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    role         TEXT NOT NULL CHECK (role IN ('owner','editor','viewer')),
    joined_at_ms BIGINT NOT NULL,
    PRIMARY KEY (project_id, account_id)
);

CREATE TABLE project_default_agents (
    project_id  TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    agent_id    TEXT NOT NULL REFERENCES agents(agent_id),
    priority    INT  NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, agent_id)
);

-- 9. conversations
CREATE TYPE conversation_kind AS ENUM ('direct','group');

CREATE TABLE conversations (
    conversation_id        TEXT PRIMARY KEY,
    kind                   conversation_kind NOT NULL,
    title                  TEXT,
    project_id             TEXT REFERENCES projects(project_id) ON DELETE SET NULL,
    created_by_account_id  TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    direct_account_low     TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    direct_account_high    TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    created_at_ms          BIGINT NOT NULL,
    updated_at_ms          BIGINT NOT NULL,
    CONSTRAINT direct_pair_consistency CHECK (
        (kind = 'direct' AND direct_account_low IS NOT NULL AND direct_account_high IS NOT NULL
                          AND direct_account_low < direct_account_high) OR
        (kind = 'group'  AND direct_account_low IS NULL AND direct_account_high IS NULL)
    )
);
CREATE UNIQUE INDEX idx_conversations_direct_pair
    ON conversations(direct_account_low, direct_account_high)
    WHERE kind = 'direct';
CREATE INDEX idx_conversations_updated_at ON conversations(updated_at_ms DESC);

CREATE TABLE conversation_members (
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    joined_at_ms     BIGINT NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
);
CREATE INDEX idx_conv_members_account ON conversation_members(account_id, joined_at_ms DESC);

CREATE TABLE conversation_messages (
    message_id           TEXT PRIMARY KEY,
    conversation_id      TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    sender_kind          TEXT NOT NULL CHECK (sender_kind IN ('user','agent','system')),
    sender_account_id    TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    sender_agent_id      TEXT REFERENCES agents(agent_id),
    body_json            JSONB NOT NULL,
    reply_to_message_id  TEXT REFERENCES conversation_messages(message_id) ON DELETE SET NULL,
    agent_session_id     TEXT,
    created_at_ms        BIGINT NOT NULL,
    recalled_at_ms       BIGINT,
    CONSTRAINT message_sender_consistency CHECK (
        (sender_kind = 'user'   AND sender_account_id IS NOT NULL AND sender_agent_id IS NULL) OR
        (sender_kind = 'agent'  AND sender_agent_id IS NOT NULL) OR
        (sender_kind = 'system' AND sender_account_id IS NULL AND sender_agent_id IS NULL)
    )
);
CREATE INDEX idx_conv_msgs_conv_created ON conversation_messages(conversation_id, created_at_ms DESC);
CREATE INDEX idx_conv_msgs_session ON conversation_messages(agent_session_id) WHERE agent_session_id IS NOT NULL;

CREATE TABLE conversation_reads (
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    last_read_at_ms  BIGINT NOT NULL,
    updated_at_ms    BIGINT NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
);

CREATE TABLE message_mentions (
    message_id            TEXT NOT NULL REFERENCES conversation_messages(message_id) ON DELETE CASCADE,
    mentioned_account_id  TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, mentioned_account_id)
);

-- 10. agent sessions / turns / turn events
CREATE TYPE agent_session_status AS ENUM ('pending','running','stopping','stopped','ended','failed');
CREATE TABLE agent_sessions (
    session_id            TEXT PRIMARY KEY,
    conversation_id       TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    project_id            TEXT REFERENCES projects(project_id) ON DELETE SET NULL,
    host_installation_id  TEXT REFERENCES device_installations(installation_id) ON DELETE SET NULL,
    agent_id              TEXT NOT NULL REFERENCES agents(agent_id),
    status                agent_session_status NOT NULL,
    started_at_ms         BIGINT NOT NULL,
    ended_at_ms           BIGINT,
    CONSTRAINT session_project_consistency CHECK (
        project_id IS NULL OR project_id IS NOT DISTINCT FROM project_id
    )
);
CREATE INDEX idx_agent_sessions_conv_status ON agent_sessions(conversation_id, status);
CREATE INDEX idx_agent_sessions_project_started
    ON agent_sessions(project_id, started_at_ms DESC) WHERE project_id IS NOT NULL;

CREATE TYPE turn_role   AS ENUM ('user','assistant','tool','system');
CREATE TYPE turn_status AS ENUM ('pending','streaming','completed','failed','canceled');
CREATE TABLE agent_turns (
    turn_id            TEXT PRIMARY KEY,
    agent_session_id   TEXT NOT NULL REFERENCES agent_sessions(session_id) ON DELETE CASCADE,
    turn_seq           BIGINT NOT NULL,
    role               turn_role NOT NULL,
    status             turn_status NOT NULL,
    started_at_ms      BIGINT NOT NULL,
    finished_at_ms     BIGINT,
    summary_text       TEXT,
    usage_json         JSONB,
    UNIQUE (agent_session_id, turn_seq)
);

CREATE TABLE agent_turn_events (
    turn_id        TEXT NOT NULL REFERENCES agent_turns(turn_id) ON DELETE CASCADE,
    event_seq      BIGINT NOT NULL,
    kind           TEXT NOT NULL,
    payload_json   JSONB NOT NULL,
    created_at_ms  BIGINT NOT NULL,
    PRIMARY KEY (turn_id, event_seq)
);
CREATE INDEX idx_turn_events_turn_created ON agent_turn_events(turn_id, created_at_ms);

-- 11. approvals
CREATE TYPE approval_state AS ENUM ('pending','decided','timeout','disconnected');
CREATE TABLE approval_requests (
    request_id        TEXT PRIMARY KEY,
    agent_session_id  TEXT NOT NULL REFERENCES agent_sessions(session_id) ON DELETE CASCADE,
    turn_id           TEXT REFERENCES agent_turns(turn_id) ON DELETE SET NULL,
    method            TEXT NOT NULL,
    params_json       JSONB NOT NULL,
    state             approval_state NOT NULL,
    deadline_at_ms    BIGINT NOT NULL,
    created_at_ms     BIGINT NOT NULL,
    resolved_at_ms    BIGINT,
    resolution_json   JSONB
);
CREATE INDEX idx_approval_session_state  ON approval_requests(agent_session_id, state);
CREATE INDEX idx_approval_deadline_state ON approval_requests(deadline_at_ms, state);

-- 12. host commands
CREATE TYPE host_command_status AS ENUM ('pending','acked','succeeded','failed');
CREATE TABLE host_commands (
    command_id                TEXT PRIMARY KEY,
    host_installation_id      TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    agent_session_id          TEXT REFERENCES agent_sessions(session_id) ON DELETE SET NULL,
    method                    TEXT NOT NULL,
    params_json               JSONB NOT NULL,
    requested_by_account_id   TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    status                    host_command_status NOT NULL,
    response_json             JSONB,
    error_json                JSONB,
    deadline_at_ms            BIGINT NOT NULL,
    created_at_ms             BIGINT NOT NULL,
    ack_at_ms                 BIGINT,
    finished_at_ms            BIGINT
);
CREATE INDEX idx_host_commands_host_status_deadline
    ON host_commands(host_installation_id, status, deadline_at_ms);

-- 13. durable event log（按 topic kind 分区）
CREATE TABLE durable_event_log (
    event_id       TEXT NOT NULL,
    topic          TEXT NOT NULL,
    topic_kind     TEXT NOT NULL,
    topic_seq      BIGINT NOT NULL,
    partition_key  TEXT NOT NULL,
    payload_json   JSONB NOT NULL,
    created_at_ms  BIGINT NOT NULL,
    PRIMARY KEY (topic_kind, event_id),
    UNIQUE (topic_kind, topic, topic_seq)
) PARTITION BY LIST (topic_kind);

CREATE TABLE durable_event_log_account       PARTITION OF durable_event_log FOR VALUES IN ('account');
CREATE TABLE durable_event_log_conversation  PARTITION OF durable_event_log FOR VALUES IN ('conversation');
CREATE TABLE durable_event_log_project       PARTITION OF durable_event_log FOR VALUES IN ('project');
CREATE TABLE durable_event_log_agent_session PARTITION OF durable_event_log FOR VALUES IN ('agent_session');
CREATE TABLE durable_event_log_host          PARTITION OF durable_event_log FOR VALUES IN ('host');

CREATE INDEX idx_durable_topic_created ON durable_event_log (topic, created_at_ms);

-- 14. outbox
CREATE TYPE outbox_status AS ENUM ('pending','claimed','acked','dead');
CREATE TABLE outbox_events (
    outbox_id        TEXT PRIMARY KEY,
    topic_kind       TEXT NOT NULL,
    event_id         TEXT NOT NULL,
    status           outbox_status NOT NULL,
    available_at_ms  BIGINT NOT NULL,
    attempts         INT NOT NULL DEFAULT 0,
    claimed_by       TEXT,
    claimed_at_ms    BIGINT,
    ack_at_ms        BIGINT,
    dead_at_ms       BIGINT,
    last_error_json  JSONB,
    FOREIGN KEY (topic_kind, event_id) REFERENCES durable_event_log(topic_kind, event_id)
);
CREATE INDEX idx_outbox_status_avail ON outbox_events(status, available_at_ms);
CREATE INDEX idx_outbox_event_id ON outbox_events(topic_kind, event_id);

-- 15. audit
CREATE TABLE audit_events (
    audit_id          TEXT PRIMARY KEY,
    actor_kind        TEXT NOT NULL,
    account_id        TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    installation_id   TEXT REFERENCES device_installations(installation_id) ON DELETE SET NULL,
    event_type        TEXT NOT NULL,
    metadata          JSONB,
    at_ms             BIGINT NOT NULL
);
CREATE INDEX idx_audit_at_ms ON audit_events(at_ms DESC);
CREATE INDEX idx_audit_account_at ON audit_events(account_id, at_ms DESC) WHERE account_id IS NOT NULL;

-- 16. push tokens（P6 用，提前在 baseline 建好）
CREATE TYPE push_kind AS ENUM ('apns','fcm');
CREATE TABLE push_tokens (
    token_hash       TEXT PRIMARY KEY,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    installation_id  TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    kind             push_kind NOT NULL,
    locale           TEXT,
    created_at_ms    BIGINT NOT NULL,
    last_used_at_ms  BIGINT NOT NULL,
    revoked_at_ms    BIGINT
);
CREATE INDEX idx_push_tokens_account ON push_tokens(account_id) WHERE revoked_at_ms IS NULL;
```

**关键设计取舍**：

- `durable_event_log` 与 `outbox_events` 共用复合主键 `(topic_kind, event_id)`，因为分区表外键必须包含分区键。`event_id` 单独是 ULID，`topic_kind` 落到分区。
- `host_links` 默认 `acl_json` 为 `'{}'::jsonb`，后续 P3 引入 ACL 时不需要 ALTER。
- `conversations.direct_account_low < direct_account_high` 在 CHECK 约束里强制有序，避免重复 pair。
- `agent_sessions.host_installation_id` 可空：允许"先创建 session、再 dispatch 到具体 host"的拓扑，在 P3.S1 落定。

**测试**：

- 新增 `tests/migration_postgres.rs`：用 `testcontainers-rs` 启动 PG16，跑全部 migration，断言：
  - 所有表都存在 + 行计数为 0（除 `agents` 预填三行）
  - `pairing_status` enum 取值集合等于 `{pending,confirmed,redeemed,expired}`
  - `durable_event_log` 5 个分区都已创建
- 新增 `tests/migration_sqlite.rs`：保留现有行为，断言 SQLite migration 顺利

**CI 改动**：

- `.github/workflows/ci.yml` 增加 `services: postgres:16-alpine` + `redis:7-alpine`
- 新增 step：`cargo test -p minos-backend --features backend-postgres` 必跑

**回退**：

- 任何阶段失败：删除 `migrations/postgres/`，把 `migrations/sqlite/` 重命名回 `migrations/`，恢复 `store::connect` 旧签名。

**工作量**：4 单位（2 天）

### P0.S2 — `AppContext` + `RepositorySet` + `DbTx` 抽象（2 天）

**目标**：service 注入 `Arc<dyn Repository>`；事务由 `DbTx` 抽象统一表示，跨 SQLite / PG 复用同一 service 代码。

**新增文件**：

- `src/app/mod.rs`
- `src/app/context.rs`
- `src/app/repositories.rs`
- `src/app/tx.rs`

**`DbTx` 抽象**：

```rust
// src/app/tx.rs
use sqlx::{PgConnection, SqliteConnection};

pub enum DbTx<'a> {
    Postgres(sqlx::Transaction<'a, sqlx::Postgres>),
    Sqlite(sqlx::Transaction<'a, sqlx::Sqlite>),
}

impl<'a> DbTx<'a> {
    pub async fn commit(self) -> Result<(), AppError> { /* ... */ }
    pub async fn rollback(self) -> Result<(), AppError> { /* ... */ }
}

#[async_trait]
pub trait Storage: Send + Sync {
    async fn begin(&self) -> Result<DbTx<'_>, AppError>;
}
```

`Storage` impl by `StoreHandle` (sqlite or postgres)；service 拿到 `Arc<dyn Storage>` 后调用 `begin()` 拿到 `DbTx`，repository 方法签名固定为 `&self, tx: &mut DbTx<'_>, ...`。

**`Repository` trait 一组（节选）**：

```rust
// src/app/repositories.rs

#[async_trait]
pub trait AccountsRepository: Send + Sync {
    async fn create(&self, tx: &mut DbTx<'_>, input: NewAccount) -> Result<AccountRow, AppError>;
    async fn find_by_email(&self, tx: &mut DbTx<'_>, email: &str) -> Result<Option<AccountRow>, AppError>;
    async fn find_by_id(&self, tx: &mut DbTx<'_>, id: &str) -> Result<Option<AccountRow>, AppError>;
    async fn touch_last_login(&self, tx: &mut DbTx<'_>, id: &str, at_ms: i64) -> Result<(), AppError>;
    async fn update_password_hash(&self, tx: &mut DbTx<'_>, id: &str, hash: &str, at_ms: i64) -> Result<(), AppError>;
}

#[async_trait]
pub trait InstallationsRepository: Send + Sync {
    async fn upsert(&self, tx: &mut DbTx<'_>, input: UpsertInstallation) -> Result<InstallationRow, AppError>;
    async fn find(&self, tx: &mut DbTx<'_>, id: &str) -> Result<Option<InstallationRow>, AppError>;
    async fn touch_last_seen(&self, tx: &mut DbTx<'_>, id: &str, at_ms: i64) -> Result<(), AppError>;
}

#[async_trait]
pub trait RefreshTokensRepository: Send + Sync {
    async fn insert(&self, tx: &mut DbTx<'_>, plaintext: &str, account_id: &str, installation_id: &str, ttl_ms: i64) -> Result<RefreshTokenRow, AppError>;
    async fn rotate(&self, tx: &mut DbTx<'_>, old_plaintext: &str, new_plaintext: &str, ttl_ms: i64) -> Result<RefreshTokenRow, AppError>;
    async fn find_active(&self, tx: &mut DbTx<'_>, plaintext: &str) -> Result<Option<RefreshTokenRow>, AppError>;
    async fn revoke_all_for_account(&self, tx: &mut DbTx<'_>, account_id: &str, at_ms: i64) -> Result<u64, AppError>;
}

#[async_trait]
pub trait DurableEventStore: Send + Sync {
    async fn record(&self, tx: &mut DbTx<'_>, event: DurableEvent) -> Result<TopicCursor, AppError>;
    async fn read_after(&self, topic: &RealtimeTopic, after_seq: i64, limit: u32) -> Result<Vec<DurableEventRow>, AppError>;
}

#[async_trait]
pub trait OutboxRepository: Send + Sync {
    async fn enqueue(&self, tx: &mut DbTx<'_>, topic_kind: &str, event_id: &str, available_at_ms: i64) -> Result<(), AppError>;
    async fn claim(&self, worker_id: &str, batch: u32) -> Result<Vec<OutboxRow>, AppError>;
    async fn ack(&self, outbox_id: &str, at_ms: i64) -> Result<bool, AppError>;
    async fn retry(&self, outbox_id: &str, available_at_ms: i64, last_error: &serde_json::Value) -> Result<bool, AppError>;
    async fn dead_letter(&self, outbox_id: &str, at_ms: i64, last_error: &serde_json::Value) -> Result<bool, AppError>;
}

#[async_trait]
pub trait HostCommandsRepository: Send + Sync {
    async fn enqueue(&self, tx: &mut DbTx<'_>, command: NewHostCommand) -> Result<(), AppError>;
    async fn ack(&self, command_id: &str, at_ms: i64) -> Result<bool, AppError>;
    async fn finish_succeeded(&self, command_id: &str, response: &serde_json::Value, at_ms: i64) -> Result<bool, AppError>;
    async fn finish_failed(&self, command_id: &str, error: &serde_json::Value, at_ms: i64) -> Result<bool, AppError>;
    async fn list_open_past_deadline(&self, now_ms: i64, batch: u32) -> Result<Vec<HostCommandRow>, AppError>;
}
```

**`AppContext`**：

```rust
// src/app/context.rs
pub struct AppContext {
    pub config: Arc<AppRuntimeConfig>,
    pub storage: Arc<dyn Storage>,
    pub repos: Arc<RepositorySet>,
    pub realtime_publisher: Arc<dyn RealtimePublisher>,
    pub host_command_dispatcher: Arc<dyn HostCommandDispatcher>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGenerator>,
    pub jwt_secret: Arc<JwtSecret>,
    pub redis: Arc<deadpool_redis::Pool>,
    pub auth_rate_limits: Arc<AuthRateLimits>,
    // services
    pub auth: Arc<AuthService>,
    pub pairing: Arc<PairingService>,
    pub agent_sessions: Arc<AgentSessionService>,
    pub approvals: Arc<ApprovalService>,
    pub conversations: Arc<ConversationService>,
    pub projects: Arc<ProjectService>,
    pub host_commands: Arc<HostCommandService>,
    pub notifications: Arc<NotificationService>,
}

pub struct RepositorySet {
    pub accounts: Arc<dyn AccountsRepository>,
    pub credentials: Arc<dyn AccountCredentialsRepository>,
    pub installations: Arc<dyn InstallationsRepository>,
    pub refresh_tokens: Arc<dyn RefreshTokensRepository>,
    pub host_tokens: Arc<dyn HostInstallationTokensRepository>,
    pub pairing_codes: Arc<dyn PairingCodesRepository>,
    pub host_links: Arc<dyn HostLinksRepository>,
    pub agents: Arc<dyn AgentsRepository>,
    pub projects: Arc<dyn ProjectsRepository>,
    pub conversations: Arc<dyn ConversationsRepository>,
    pub conversation_messages: Arc<dyn ConversationMessagesRepository>,
    pub agent_sessions: Arc<dyn AgentSessionsRepository>,
    pub agent_turns: Arc<dyn AgentTurnsRepository>,
    pub agent_turn_events: Arc<dyn AgentTurnEventsRepository>,
    pub approvals: Arc<dyn ApprovalsRepository>,
    pub host_commands: Arc<dyn HostCommandsRepository>,
    pub durable_events: Arc<dyn DurableEventStore>,
    pub outbox: Arc<dyn OutboxRepository>,
    pub audits: Arc<dyn AuditRepository>,
    pub push_tokens: Arc<dyn PushTokensRepository>,
}
```

**测试策略**：

- 每个 repository 写两个 impl：`postgres::AccountsRepository` 与 `sqlite::AccountsRepository`
- 共享一组 `tests/repositories/<name>_contract.rs`，参数化 `[(sqlite, ...), (postgres, ...)]` 跑同一组断言

**验收**：

- `BackendState` 内部仅持 `Arc<AppContext>` 与 cors/version 元信息
- `cargo test -p minos-backend` 在 sqlite 和 postgres 两个 feature flag 下都绿
- `cargo doc --no-deps -p minos-backend` 能编译，所有 repository trait 有完整 rustdoc

**回退**：

- 若 PG 集成不稳定：保留 `Storage::Sqlite` 路径，`AppContext::compose_postgres` 退化为编译期 panic 占位

**工作量**：4 单位（2 天）

### P0.S3 — Docs lint + 错误码注册表 + 路由 inventory drift（1 天）

**改动**：

- `xtask/src/lints/docs.rs`：解析 `docs/architecture-overview.md`、`docs/backend-formal-development.md`、`docs/backend-implementation-plan.md`，断言：
  - topic 名集合三方一致
  - HTTP 路径集合（架构 overview 中表格 ↔ formal 中 list ↔ plan 中 §0.5 错误码命名空间没有矛盾）
  - 表名集合（architecture §3.4 ↔ formal Data Model）一致
- `crates/minos-backend/src/error/registry.rs`：硬编码所有合法 error code，作为单元测试断言"`err(code, ...)` 调用的 code 必须在注册表中"。
- `xtask/src/lints/route_inventory.rs`：解析 `formal_route_inventory()` 与 `axum::Router::routes()` 实际挂载，差集为 0

**验收**：

- 故意删 `architecture-overview.md` 中的某个 topic：`cargo xtask lint-docs` 退非 0
- 故意 push 一条 `err("foobar", ...)` 但未注册：`cargo test -p minos-backend` 退非 0

**工作量**：2 单位（1 天）

### P0 完成定义

- [ ] CI 在 PG + Redis 矩阵下绿
- [ ] `MINOS_STORAGE_MODE=external-sql --database-url postgres://...` 启动并 `/health/ready` 200
- [ ] `cargo xtask lint-docs` 与 `cargo xtask lint-conventions` 通过
- [ ] 所有 repository trait + `AppContext` 落地

### P0 回退总结

- 若 PG 集成阻断超过 3 天：把 PG 路径作为 `--feature backend-postgres`，默认 off；改用 SQLite 完成 P1–P3 的逻辑设计，再回头补 PG。
- 文档 drift gate 出现误报：先暂时设置为 warning（非 fatal），并开 issue 跟踪规则修复。

---

## P1 — Durable + Outbox 双写底座

**Phase 目标**：把 `durable_event_log` 与 `outbox_events` 从"只有 schema"升级到"所有 durable 用例必须双写"，并提供 dispatcher worker。

**Phase 完成定义**：

- 所有 register / pair / approval / agent session / message 路径都遵守事务三件套
- Outbox dispatcher worker 在多实例下不重复 ack，故障注入下能正确回退/dead-letter
- 客户端可通过 `read-turns` 重建 stream slice；`durable_event_log` 可通过 read API 重建 topic 状态

### P1.S1 — `RealtimeTopic` / `DurableEvent` 类型 + `DurableEventStore` 实现（2 天）

**新增文件**：

- `src/realtime/topic.rs`
- `src/realtime/event.rs`
- `src/store/postgres/durable_event_log.rs`
- `src/store/sqlite/durable_event_log.rs`

**`RealtimeTopic`**：

```rust
// src/realtime/topic.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RealtimeTopic {
    Account(String),
    Conversation(String),
    Project(String),
    AgentSession(String),
    Host(String),
}

impl RealtimeTopic {
    pub fn kind(&self) -> TopicKind { /* ... */ }
    pub fn topic_string(&self) -> String { /* "conversation:conv_01J..." */ }
    pub fn partition_key(&self) -> &str { /* the inner id */ }
    pub fn parse(s: &str) -> Result<Self, TopicParseError> { /* ... */ }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicKind {
    Account, Conversation, Project, AgentSession, Host,
}
```

**`DurableEvent`**：

```rust
// src/realtime/event.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DurableEvent {
    AccountRegistered      { account_id: String, at_ms: i64 },
    AccountPasswordChanged { account_id: String, at_ms: i64 },
    HostLinked             { account_id: String, host_installation_id: String, pair_id: String, at_ms: i64 },
    HostUnlinked           { account_id: String, host_installation_id: String, at_ms: i64 },
    AgentSessionStarted    { session_id: String, conversation_id: String, project_id: Option<String>, host_installation_id: String, agent_id: String, at_ms: i64 },
    AgentSessionEnded      { session_id: String, status: String, at_ms: i64 },
    AgentTurnAppended      { session_id: String, turn_id: String, turn_seq: i64, role: String, status: String, at_ms: i64 },
    ApprovalRequested      { request_id: String, session_id: String, method: String, deadline_at_ms: i64, at_ms: i64 },
    ApprovalResolved       { request_id: String, session_id: String, resolution: ApprovalResolution, at_ms: i64 },
    ConversationMessageAppended  { conversation_id: String, message_id: String, sender: SenderRef, at_ms: i64 },
    ConversationMessageRecalled  { conversation_id: String, message_id: String, at_ms: i64 },
    ProjectConversationLinked    { project_id: String, conversation_id: String, at_ms: i64 },
    ProjectArchived              { project_id: String, at_ms: i64 },
    HostForceClose               { host_installation_id: String, reason: String, at_ms: i64 },
}

impl DurableEvent {
    pub fn topic(&self) -> RealtimeTopic { /* derive from variant */ }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ApprovalResolution {
    Decided    { decision: serde_json::Value },
    Timeout,
    Disconnect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SenderRef {
    User  { account_id: String },
    Agent { agent_id: String, session_id: Option<String> },
    System,
}
```

**`DurableEventStore` PG 实现的关键算法**：

```rust
async fn record(&self, tx: &mut DbTx<'_>, event: DurableEvent) -> Result<TopicCursor, AppError> {
    let topic = event.topic();
    let topic_str = topic.topic_string();
    let topic_kind = topic.kind().as_str();

    // 1. 拿同 topic 的 advisory lock，保证 topic 内串行
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(&topic_str)
        .execute(&mut **tx.as_postgres())
        .await?;

    // 2. 计算下一个 topic_seq
    let next_seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(topic_seq), 0) + 1
           FROM durable_event_log
          WHERE topic_kind = $1 AND topic = $2"
    )
    .bind(topic_kind)
    .bind(&topic_str)
    .fetch_one(&mut **tx.as_postgres())
    .await?;

    // 3. 写入
    let event_id = self.ids.new_event_id();
    let payload = serde_json::to_value(&event)?;
    let now_ms = self.clock.now_ms();
    sqlx::query(
        "INSERT INTO durable_event_log (event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(&event_id)
    .bind(&topic_str)
    .bind(topic_kind)
    .bind(next_seq)
    .bind(topic.partition_key())
    .bind(&payload)
    .bind(now_ms)
    .execute(&mut **tx.as_postgres())
    .await?;

    Ok(TopicCursor { event_id, topic, topic_seq: next_seq })
}
```

**测试矩阵**：

- `tests/durable_event_log_concurrency.rs`：100 个 `tokio::spawn` 同 topic 并发 record → 序列严格 1..=100，无空洞
- `tests/durable_event_log_partition.rs`：分别 record account/conversation/project/agent_session/host → 落到正确分区表（`SELECT tableoid::regclass`）
- `tests/durable_event_log_read_after.rs`：record N 条 → `read_after(topic, after=k, limit=L)` 返回正确切片
- `tests/durable_event_log_serialization.rs`：每个 `DurableEvent` variant 轮转 `serde_json` 等价性

**性能基线测试**：

- `cargo bench` 新增 `bench_durable_record` 目标：单 PG 实例（local docker），单连接、4096 条 record 总耗时 < 10s（即 ≥ 400 ops/s）。低于此值需要先优化 advisory lock 粒度。

**验收**：

- `record` 必须接 `&mut DbTx`，无法在事务外调用（编译期约束）
- 所有测试绿
- bench 不退步

**工作量**：4 单位（2 天）

### P1.S2 — `RealtimePublisher` trait + Redis 实现（1.5 天）

**新增文件**：

- `src/realtime/publisher.rs`
- `src/realtime/redis_publisher.rs`
- `src/realtime/inline_publisher.rs`（dev/test）

**Trait**：

```rust
#[async_trait]
pub trait RealtimePublisher: Send + Sync {
    async fn publish_durable(&self, event: &DurableEventEnvelope) -> Result<(), AppError>;
    async fn publish_ephemeral(&self, topic: &RealtimeTopic, event: &StreamEventFrame) -> Result<(), AppError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableEventEnvelope {
    pub topic: String,
    pub topic_seq: i64,
    pub event_id: String,
    pub payload: DurableEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEventFrame {
    pub kind: StreamEventKind,
    pub seq: Option<i64>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamEventKind {
    AgentTextDelta,
    StdoutChunk,
    DiffChunk,
    ToolProgress,
    PresenceUpdate,
}
```

**Redis 实现**：

```rust
async fn publish_durable(&self, env: &DurableEventEnvelope) -> Result<(), AppError> {
    let payload = serde_json::to_string(env)?;
    let channel = format!("minos.durable.{}", env.topic);
    let mut conn = self.pool.get().await?;
    redis::cmd("PUBLISH").arg(&channel).arg(&payload)
        .query_async(&mut *conn).await?;
    Ok(())
}
```

**测试**：

- `tests/redis_publisher_roundtrip.rs`：用 testcontainers Redis，启动 subscriber → publish → 收到 payload
- `tests/inline_publisher_for_singlenode.rs`：assert inline 模式只在本地 channel 触发回调

**验收**：

- Redis publisher 在 connection drop 后能自动重连（`deadpool-redis` 默认行为，加测试用例验证）

**工作量**：3 单位（1.5 天）

### P1.S3 — Outbox dispatcher worker（2.5 天）

**新增文件**：

- `src/jobs/mod.rs`
- `src/jobs/outbox_dispatcher.rs`

**核心算法**：

```rust
pub struct OutboxDispatcher {
    repos: Arc<RepositorySet>,
    publisher: Arc<dyn RealtimePublisher>,
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    worker_id: String,
    config: OutboxDispatcherConfig,
    notify: Notify,           // backpressure: NOTIFY 后立即唤醒 claim
}

#[derive(Debug, Clone)]
pub struct OutboxDispatcherConfig {
    pub batch_size: u32,           // default 64
    pub max_attempts: u32,         // default 8
    pub base_backoff_ms: u64,      // default 200
    pub max_backoff_ms: u64,       // default 60_000
    pub jitter_ratio: f64,         // default 0.2
    pub idle_sleep_ms: u64,        // default 200
}

impl OutboxDispatcher {
    pub async fn run(self: Arc<Self>) {
        loop {
            let claimed = match self.repos.outbox.claim(&self.worker_id, self.config.batch_size).await {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(?error, "outbox claim failed");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            if claimed.is_empty() {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(self.config.idle_sleep_ms)) => {},
                    _ = self.notify.notified() => {},
                }
                continue;
            }

            for row in claimed {
                self.dispatch_one(row).await;
            }
        }
    }

    async fn dispatch_one(&self, row: OutboxRow) {
        let event = match self.repos.durable_events.get_by_id(&row.topic_kind, &row.event_id).await {
            Ok(Some(e)) => e,
            Ok(None) => {
                let _ = self.repos.outbox.dead_letter(&row.outbox_id, self.clock.now_ms(),
                    &json!({"kind":"orphan","message":"event not found"})).await;
                return;
            }
            Err(error) => {
                self.requeue(&row, error).await;
                return;
            }
        };

        let envelope = DurableEventEnvelope {
            topic: event.topic.clone(),
            topic_seq: event.topic_seq,
            event_id: event.event_id.clone(),
            payload: serde_json::from_value(event.payload_json).unwrap(),
        };

        match self.publisher.publish_durable(&envelope).await {
            Ok(()) => {
                let _ = self.repos.outbox.ack(&row.outbox_id, self.clock.now_ms()).await;
                metrics::OUTBOX_DISPATCH_TOTAL.with_label_values(&["ok"]).inc();
            }
            Err(error) => {
                self.requeue(&row, error).await;
            }
        }
    }

    async fn requeue(&self, row: &OutboxRow, error: AppError) {
        let next_attempts = row.attempts + 1;
        if next_attempts >= self.config.max_attempts {
            let _ = self.repos.outbox.dead_letter(&row.outbox_id, self.clock.now_ms(),
                &error_to_json(&error)).await;
            metrics::OUTBOX_DISPATCH_TOTAL.with_label_values(&["dead"]).inc();
            return;
        }
        let backoff = self.compute_backoff(next_attempts);
        let _ = self.repos.outbox.retry(&row.outbox_id,
            self.clock.now_ms() + backoff as i64,
            &error_to_json(&error)).await;
        metrics::OUTBOX_DISPATCH_TOTAL.with_label_values(&["retry"]).inc();
    }

    fn compute_backoff(&self, attempts: u32) -> u64 {
        let base = self.config.base_backoff_ms.saturating_mul(1u64 << attempts.min(10));
        let capped = base.min(self.config.max_backoff_ms);
        let jitter_range = (capped as f64 * self.config.jitter_ratio) as u64;
        let jitter = rand::thread_rng().gen_range(0..=jitter_range.max(1));
        capped.saturating_sub(jitter_range / 2).saturating_add(jitter)
    }
}
```

**Claim SQL（关键，避免重复）**：

```sql
-- PG: 用 FOR UPDATE SKIP LOCKED + UPDATE
WITH cte AS (
  SELECT outbox_id
    FROM outbox_events
   WHERE status = 'pending'
     AND available_at_ms <= $1
   ORDER BY available_at_ms
   LIMIT $2
   FOR UPDATE SKIP LOCKED
)
UPDATE outbox_events o
   SET status = 'claimed',
       attempts = o.attempts + 1,
       claimed_by = $3,
       claimed_at_ms = $1
  FROM cte
 WHERE o.outbox_id = cte.outbox_id
RETURNING o.outbox_id, o.topic_kind, o.event_id, o.attempts, o.available_at_ms;
```

**测试矩阵**：

| 测试 | 场景 | 断言 |
|---|---|---|
| `outbox_basic.rs` | enqueue 1 → dispatch → ack | row.status='acked' 且 publisher 被调用 1 次 |
| `outbox_retry.rs` | publisher 3 次失败后成功 | attempts==4，最终 acked，metric `retry` += 3，`ok` += 1 |
| `outbox_dead_letter.rs` | publisher 持续失败 | 第 8 次后 status='dead'，dead_at_ms 写入 |
| `outbox_concurrent.rs` | enqueue 1000 + 启动 5 个 dispatcher | 总 ack 数 == 1000，无重复 |
| `outbox_backpressure.rs` | enqueue 5000 持续高速 | dispatcher claim QPS 维持，无饥饿 |
| `outbox_orphan.rs` | enqueue 一个 event_id 不存在的 outbox 行 | 直接 dead-letter |

**故障注入**：

- `tests/outbox_publisher_panic.rs`：publisher impl 在第 N 次调用 panic，断言 dispatcher loop 不死、retry 路径触发

**Metrics**：

```
outbox_dispatch_total{result="ok|retry|dead"}  Counter
outbox_dispatch_lag_seconds                    Histogram (now - available_at_ms)
outbox_claim_batch_size                        Histogram
outbox_in_flight                               Gauge (claimed - acked)
```

**回退**：

- 若高并发下 dead-letter 大量出现：把 `max_attempts` 临时升到 32，并引入"dead-letter 自动重试"周期（每小时把 dead 行 reset 到 pending，最多重试一次）。

**工作量**：5 单位（2.5 天）

### P1.S4 — 事务三件套接入到现有 service（3 天）

**目标**：把现 `auth/use_case.rs` / `pairing/mod.rs` / `social/mod.rs` / `ingest/mod.rs` / `approval_relay.rs` 中的写路径都改造为 "tx + record + outbox.enqueue"。

**改造模板（以 register 为例）**：

```rust
impl AuthService {
    pub async fn register(&self, input: RegisterInput) -> Result<AuthSession, AuthError> {
        self.rate_limits.check_register_per_ip(&input.client_ip)?;
        let mut tx = self.storage.begin().await?;

        // 1. domain 写入
        let account = self.repos.accounts.create(&mut tx, NewAccount {
            account_id: self.ids.new_account_id(),
            email: input.email.clone(),
            display_name: None,
            created_at_ms: self.clock.now_ms(),
        }).await?;
        self.repos.credentials.upsert(&mut tx, &account.account_id, &hash_password(&input.password)?, self.clock.now_ms()).await?;

        let installation = self.repos.installations.upsert(&mut tx, UpsertInstallation::for_mobile(...)).await?;

        let access = self.mint_access_token(&account, &installation)?;
        let refresh = self.repos.refresh_tokens.insert(&mut tx, &gen_refresh()?, &account.account_id, &installation.installation_id, REFRESH_TTL_MS).await?;

        // 2. durable event
        let cursor = self.repos.durable_events.record(&mut tx, DurableEvent::AccountRegistered {
            account_id: account.account_id.clone(),
            at_ms: self.clock.now_ms(),
        }).await?;

        // 3. outbox enqueue
        self.repos.outbox.enqueue(&mut tx, "account", &cursor.event_id, self.clock.now_ms()).await?;

        tx.commit().await?;
        // 通知 dispatcher（best-effort）
        self.outbox_notifier.notify_one();

        Ok(AuthSession { access, refresh, account_id: account.account_id, expires_in: ACCESS_TTL_MS / 1000 })
    }
}
```

**改造清单**：

| Service / 方法 | 写哪些 row | DurableEvent | Topic |
|---|---|---|---|
| `AuthService::register` | accounts, credentials, installations, refresh_tokens | AccountRegistered | account:<id> |
| `AuthService::change_password` | credentials, refresh_tokens(全 revoke) | AccountPasswordChanged | account:<id> |
| `PairingService::confirm_pairing` | pairing_codes, host_links | HostLinked | account:<id> + host:<id> |
| `PairingService::revoke_link` | host_links, host_installation_tokens(可选 revoke) | HostUnlinked + (HostForceClose if last) | account:<id> + host:<id> |
| `ConversationService::send_message` | conversation_messages, conversation_reads, message_mentions | ConversationMessageAppended | conversation:<id> |
| `AgentSessionService::start` | agent_sessions, agent_turns, host_commands | AgentSessionStarted + AgentTurnAppended | agent_session:<id> + host:<id> |
| `AgentSessionService::on_turn_complete` | agent_turns(update) | AgentTurnAppended | agent_session:<id> |
| `AgentSessionService::end` | agent_sessions(update) | AgentSessionEnded | agent_session:<id> |
| `ApprovalService::record_request` | approval_requests | ApprovalRequested | agent_session:<id> |
| `ApprovalService::respond` | approval_requests(update), host_commands | ApprovalResolved | agent_session:<id> |

**Mechanical lint**（在 P0.S3 已落地）跑通三件套规则。

**测试矩阵**：

- 每个 service 方法增加 `tests/<service>_durable.rs`：
  - 成功路径：断言 `outbox_events` +1，`durable_event_log` +1，topic 与 cursor 正确
  - 失败路径（input 校验失败）：断言事务全部回滚，没有 outbox 行
  - 故障注入：让 publisher 失败 → 断言事务依然 commit，dispatcher 后续重试

**验收**：

- `grep -r "publish_durable\|publish_ephemeral" src/` 在 service 层只出现在事务结束之后
- mechanical lint 通过

**工作量**：6 单位（3 天）

### P1.S5 — `read-turns(after_event_seq)` cold replay 接口（1 天）

**改动**：

- `src/http/v1/agent_sessions.rs::read_turns` 已存在双模式签名，改造其 turn-events 模式实际从 `agent_turn_events` 拉取
- 新增分页 cursor 规则：`limit ≤ 200`；超过返回 `validation_format`

**验收**：

- 集成测试：写入 1000 条 stream slice → `read_turns(turn_id, after_event_seq=k, limit=L)` 返回正确切片
- 测试 retention：手动 mark `agent_session.ended_at_ms` 然后调用 retention cleaner（P5），断言 7 天前的 events 被清理且 read_turns 返回 `realtime_snapshot_required`

**工作量**：2 单位（1 天）

### P1 完成定义

- [ ] 所有 durable 用例事务三件套
- [ ] Outbox dispatcher 在多实例下不重复
- [ ] 单元 + 集成 + 故障注入测试矩阵全绿
- [ ] Metrics + tracing span 覆盖 outbox 路径


---

## P2 — Realtime Gateway 重构

**Phase 目标**：把当前"`/devices` 混合鉴权 + 隐式 device 扇出"的运行时，重写为"`/ws/client` + `/ws/host` + topic 订阅 + 短 TTL ticket + durable replay + ephemeral pubsub"。本 phase 的产出是客户端 / host daemon 接入 backend 的唯一通道；MVP 兼容路径在 P8 退场，本 phase 暂保留并加 deprecate metrics。

**Phase 完成定义**：

- 公共流量 100% 走 `/ws/client` / `/ws/host`，并基于 ticket 鉴权，无 `X-Device-*` 头部
- WS 协议帧符合 architecture-overview §4.2（subscribe / subscribe_ack / durable_event / stream_event / snapshot_required / host_force_close / ping / pong）
- 客户端断网重连用 `resume_after = { topic: last_durable_seq }` 能恢复 durable，cold replay 走 read API
- 同 `(principal, installation_id)` 抢占替换有明确 close code（4401）和 metrics
- Outbox dispatcher（P1.S3）的 `publish_durable` 真正驱动 client/host gateway 把帧推到客户端

**Phase 输出依赖**：P1 已落地（durable_event_log、outbox、publisher trait）。

### P2.S1 — Redis ticket store + 单次 consume + ticket policy（1 天）

**目标**：ticket 不再依赖 SQL `ws_tickets` 表，统一存 Redis；签发与 consume 走原子操作；ticket payload 既携带 principal 又携带"声明的 installation_id"，以便后续重连抢占识别。

**新增/改动文件**：

- `src/auth/realtime_ticket.rs`（重写）
- `src/store/redis/ticket.rs`（新）
- `src/http/v1/realtime.rs`（client：`/v1/realtime/ws-ticket`）
- `src/http/v1/host.rs`（host：`/v1/host/realtime/ws-ticket` 改造）

**Ticket payload（JWT, HS256，复用 `jwt_secret`）**：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsTicketClaims {
    pub jti: String,            // ticket id, ULID, 与 Redis key 对齐
    pub sub: String,            // account_id (client) 或 host_installation_id (host)
    pub principal: PrincipalKind,   // "account" | "host_installation"
    pub installation_id: String,
    pub iat: i64,
    pub exp: i64,               // 默认 iat + 60s
    pub gateway_url_hint: Option<String>,
}
```

**Redis key**：

- `minos:ticket:<jti>` → JSON payload，TTL 60s
- consume 用 Lua 原子脚本：

```lua
-- KEYS[1] = "minos:ticket:<jti>"
-- ARGV[1] = expected principal kind
-- ARGV[2] = expected installation_id (optional, "" if not enforced)
local v = redis.call('GET', KEYS[1])
if not v then return cjson.encode({ok=false, reason="not_found"}) end
local payload = cjson.decode(v)
if payload.principal ~= ARGV[1] then return cjson.encode({ok=false, reason="principal_mismatch"}) end
if ARGV[2] ~= "" and payload.installation_id ~= ARGV[2] then
  return cjson.encode({ok=false, reason="installation_mismatch"})
end
redis.call('DEL', KEYS[1])
return cjson.encode({ok=true, payload=payload})
```

**Trait**：

```rust
#[async_trait]
pub trait RealtimeTicketStore: Send + Sync {
    async fn issue(&self, claims: WsTicketClaims) -> Result<String, AppError>;     // returns signed JWT
    async fn consume(&self, jti: &str, expected: ExpectedPrincipal) -> Result<WsTicketClaims, TicketConsumeError>;
}

#[derive(Debug, thiserror::Error)]
pub enum TicketConsumeError {
    #[error("realtime_ticket_invalid")]   NotFoundOrConsumed,
    #[error("realtime_ticket_invalid")]   PrincipalMismatch,
    #[error("realtime_ticket_invalid")]   InstallationMismatch,
    #[error("realtime_ticket_invalid")]   SignatureInvalid,
    #[error("realtime_ticket_invalid")]   Expired,
    #[error("internal: {0}")]             Internal(#[from] AppError),
}
```

**HTTP**：

- `POST /v1/realtime/ws-ticket`（account bearer 必须）
  - 请求：`{ "installation_id": "inst_..." }`（必须等于 access token 中绑定的 installation；不一致返回 `auth_invalid_installation`）
  - 响应：`{ "data": { "ticket": "...", "expires_at_ms": ..., "gateway_url": "wss://.../ws/client?ticket=..." } }`
- `POST /v1/host/realtime/ws-ticket`（host token 必须）
  - 请求：`{}`（installation_id 从 token 中取）
  - 响应：同上但 `gateway_url` 指向 `/ws/host`

**测试**：

- `tests/realtime_ticket_issue_consume.rs`：sign → consume 一次成功 → 二次失败 `NotFoundOrConsumed`
- `tests/realtime_ticket_principal_guard.rs`：account ticket 用 host consume → `PrincipalMismatch`
- `tests/realtime_ticket_expired.rs`：把系统时钟拨到 `exp+1` → `Expired`
- `tests/realtime_ticket_installation_guard.rs`：installation_id 不一致 → `InstallationMismatch`

**验收**：

- `MINOS_CACHE_BACKEND=redis`、`MINOS_REDIS_URL=...` 在 prod 校验中已强制；ticket store 直接复用同一 Redis 池
- ticket 失败统一映射 HTTP 401 + `realtime_ticket_invalid`
- metrics：`realtime_ticket_issue_total{principal}` / `realtime_ticket_consume_total{result}`

**工作量**：2 单位（1 天）

### P2.S2 — Subscription manager + topic 鉴权（1.5 天）

**目标**：每个 WS 连接维护 `HashSet<RealtimeTopic>`；新增 `authorize_subscription(principal, topic, deps)` 把权限判断从 gateway 抽出来，便于测试。

**新增文件**：

- `src/realtime/subscription.rs`
- `src/realtime/auth.rs`

**核心数据结构**：

```rust
pub struct ConnectionId(pub Uuid);

pub struct ConnectionState {
    pub conn_id: ConnectionId,
    pub principal: ConnectionPrincipal,
    pub installation_id: String,
    pub topics: parking_lot::RwLock<HashSet<RealtimeTopic>>,
    pub revoke: tokio::sync::Notify,
    pub created_at_ms: i64,
    pub last_pong_at_ms: AtomicI64,
}

pub enum ConnectionPrincipal {
    Account { account_id: String },
    Host    { host_installation_id: String },
}

pub struct SubscriptionManager {
    by_topic: DashMap<RealtimeTopic, HashSet<ConnectionId>>,
    by_conn:  DashMap<ConnectionId, Arc<ConnectionState>>,
}

impl SubscriptionManager {
    pub fn add_connection(&self, conn: Arc<ConnectionState>);
    pub fn remove_connection(&self, conn_id: ConnectionId);
    pub fn add_topics(&self, conn_id: ConnectionId, topics: &[RealtimeTopic]) -> Vec<RealtimeTopic>;   // newly subscribed
    pub fn remove_topics(&self, conn_id: ConnectionId, topics: &[RealtimeTopic]);
    pub fn fanout_targets(&self, topic: &RealtimeTopic) -> Vec<Arc<ConnectionState>>;
}
```

**鉴权规则（必须有单元测试）**：

```rust
#[async_trait]
pub trait SubscriptionAuthorizer: Send + Sync {
    async fn authorize(
        &self,
        principal: &ConnectionPrincipal,
        topic: &RealtimeTopic,
    ) -> Result<(), SubscriptionDenied>;
}

#[derive(Debug, thiserror::Error)]
pub enum SubscriptionDenied {
    #[error("realtime_subscription_denied")]            Forbidden,
    #[error("realtime_subscription_limit_exceeded")]    LimitExceeded,
    #[error("realtime_subscription_invalid_topic")]     InvalidTopic,
}
```

具体规则：

| Topic | account 允许条件 | host 允许条件 |
|---|---|---|
| `account:<id>` | `id == self.account_id` | 拒绝 |
| `conversation:<id>` | 是该 conversation member | 拒绝 |
| `project:<id>` | 是该 project 的任一 role member | 拒绝 |
| `agent_session:<id>` | 是 session.conversation 的 member（或 project member 当 session 已 link） | 仅当 session.host_installation_id == self.host_installation_id |
| `host:<id>` | 拒绝 | `id == self.host_installation_id` |

实现：

- `authorize` 内部调用 repos：`conversation_members`、`project_members`、`agent_sessions::find` 等
- 返回 `Forbidden`：HTTP/WS 帧用 `realtime_subscription_denied`
- 单连接限制：32 topics/subscribe，128 live；超出返回 `realtime_subscription_limit_exceeded`

**测试**：

- `tests/subscription_authorizer.rs`：覆盖每条规则的"允许 / 拒绝"两路（account 自身 vs 他人；host 越权订 conversation 等）
- `tests/subscription_manager.rs`：并发 add_topics / remove_connection 不死锁、`fanout_targets` 正确

**Metrics**：

- `realtime_subscriptions_total{topic_kind}` Counter（新增订阅）
- `realtime_subscriptions_active{topic_kind}` Gauge（live 数）
- `realtime_subscription_denied_total{reason,topic_kind}` Counter

**工作量**：3 单位（1.5 天）

### P2.S3 — `/ws/client` + `/ws/host` gateway 重写（4 天）

**目标**：

- `upgrade_client` / `upgrade_host` 仅做 ticket consume → 升级
- 新增 `realtime::session::run_session(conn, ws, ctx)` 替换旧 `envelope::run_session`
- 实现 subscribe / unsubscribe / ping / pong / resume / replay / live forwarding
- 跨实例 fan-out：每个 gateway 进程在启动时订阅 `minos.durable.*` Redis pubsub；收到消息后查 `SubscriptionManager` 决定本地推送

**新增文件**：

- `src/realtime/gateway/mod.rs`
- `src/realtime/gateway/client.rs`
- `src/realtime/gateway/host.rs`
- `src/realtime/gateway/session.rs`
- `src/realtime/gateway/listener.rs`
- `src/realtime/wire.rs`（WS 帧 enum + JSON Schema 派生）

**Wire frame（JSON, JSON Schema 派生用 `schemars`）**：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Subscribe   { topics: Vec<String>, resume_after: Option<HashMap<String, i64>>, client_request_id: Option<String> },
    Unsubscribe { topics: Vec<String> },
    Ping        { ts: i64 },
    HostCommandAck    { command_id: String, ack_at_ms: i64 },                               // host only
    HostCommandResult { command_id: String, status: String, result: Option<Value>, error: Option<Value>, finished_at_ms: i64 }, // host only
    HostStreamEvent   { topic: String, kind: String, payload: Value },                      // host uplink (agent text delta etc.)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Hello              { conn_id: String, server_time_ms: i64, heartbeat_interval_ms: i64 },
    SubscribeAck       { topics: Vec<String>, client_request_id: Option<String> },
    SubscriptionDenied { topic: String, reason: String },
    SubscriptionLimitExceeded { limit: usize, current: usize },
    DurableEvent       { topic: String, topic_seq: i64, kind: String, payload: Value, event_id: String },
    StreamEvent        { topic: String, kind: String, seq: Option<i64>, payload: Value },
    SnapshotRequired   { topic: String, last_known_seq: i64, retention_floor_seq: i64 },
    HostForceClose     { reason: String, close_code: u16 },
    Pong               { ts: i64, server_time_ms: i64 },
    Error              { code: String, message: String, request_id: String },
}
```

**Session 主循环（伪码）**：

```rust
pub async fn run_session(
    ws: WebSocket,
    state: BackendState,
    upgrade: UpgradedConnection,        // 携带 principal / installation_id / conn_id
) {
    let (mut tx, mut rx) = ws.split();
    let conn = Arc::new(ConnectionState::new(&upgrade));
    state.subscription_mgr.add_connection(Arc::clone(&conn));

    // 1. Hello
    send(&mut tx, ServerFrame::Hello {
        conn_id: conn.conn_id.to_string(),
        server_time_ms: state.clock.now_ms(),
        heartbeat_interval_ms: 25_000,
    }).await;

    // 2. 自动订阅默认 topic
    let default_topic = match &conn.principal {
        ConnectionPrincipal::Account{account_id} => RealtimeTopic::Account(account_id.clone()),
        ConnectionPrincipal::Host{host_installation_id} => RealtimeTopic::Host(host_installation_id.clone()),
    };
    state.subscription_mgr.add_topics(conn.conn_id, &[default_topic.clone()]);

    // 3. 接 outbox dispatcher 的本地 mpsc
    let mut outbox_rx = state.gateway_listener.subscribe_for_conn(conn.conn_id);

    loop {
        tokio::select! {
            // 读客户端
            msg = rx.next() => match msg {
                None | Some(Err(_)) => break,
                Some(Ok(WsMessage::Close(_))) => break,
                Some(Ok(WsMessage::Ping(_))) => {/* axum auto pong */}
                Some(Ok(WsMessage::Text(text))) => handle_client_frame(&state, &conn, &mut tx, text).await,
                Some(Ok(WsMessage::Binary(_))) => send_error(&mut tx, "validation_format", "binary frames not allowed").await,
                _ => {}
            },

            // outbox dispatcher 推下来的 durable / stream 帧
            push = outbox_rx.recv() => match push {
                Some(frame) => send(&mut tx, frame).await,
                None => break,
            },

            // 心跳超时
            _ = tokio::time::sleep(HEARTBEAT_TIMEOUT) => {
                tracing::warn!(conn_id=%conn.conn_id, "ws heartbeat timeout");
                break;
            },

            // 抢占 / 强制断开
            _ = conn.revoke.notified() => {
                send(&mut tx, ServerFrame::HostForceClose { reason: "superseded".into(), close_code: 4401 }).await;
                break;
            },
        }
    }

    state.subscription_mgr.remove_connection(conn.conn_id);
    state.presence.remove(&conn).await;
}
```

**`handle_client_frame::Subscribe`（带 resume）伪码**：

```rust
for topic_str in &req.topics {
    let topic = RealtimeTopic::parse(topic_str).map_err(|_| ServerFrame::SubscriptionDenied {...})?;
    state.subscription_authorizer.authorize(&conn.principal, &topic).await?;
}

let newly = state.subscription_mgr.add_topics(conn.conn_id, &topics);
send(&mut tx, ServerFrame::SubscribeAck { topics: newly.iter().map(|t| t.topic_string()).collect(), client_request_id: req.client_request_id }).await;

// resume replay
for topic in &newly {
    let after = req.resume_after.as_ref().and_then(|m| m.get(topic.topic_string().as_str()).copied()).unwrap_or(-1);
    let retention_floor = state.repos.durable_events.retention_floor(topic).await?;
    if after >= 0 && after < retention_floor {
        send(&mut tx, ServerFrame::SnapshotRequired { topic: topic.topic_string(), last_known_seq: after, retention_floor_seq: retention_floor }).await;
        continue;
    }
    let mut next_after = after;
    loop {
        let batch = state.repos.durable_events.read_after(topic, next_after, 256).await?;
        if batch.is_empty() { break; }
        for row in &batch {
            send(&mut tx, ServerFrame::DurableEvent { topic: row.topic.clone(), topic_seq: row.topic_seq, kind: row.kind(), payload: row.payload.clone(), event_id: row.event_id.clone() }).await;
        }
        next_after = batch.last().unwrap().topic_seq;
        if batch.len() < 256 { break; }
    }
}
```

**Listener / fan-out**：

- 每个 gateway 进程启动一个 `GatewayListener`，订阅 Redis channel `minos.durable.*` 与 `minos.stream.*`
- 收到 payload → 解析出 `topic` → `subscription_mgr.fanout_targets(&topic)` → 把 `ServerFrame` 投到每个连接的 mpsc

**抢占（presence）**：

- Redis Set `minos:presence:<account_id>:<installation_id>` 元素：`<gateway_node_id>:<conn_id>`
- 新连接 add 时，先发一个 `PUBLISH minos.control.<account_id>.<installation_id> "SUPERSEDED:<new_conn_id>"`
- 监听本进程订阅的 control channel；收到不属于自己的 superseded → `revoke.notify_one()`

**Host 端额外职责**：

- host 上行 `HostCommandAck` / `HostCommandResult` 由 `HostCommandService::on_inbound_frame` 处理（写 `host_commands` 表 + 通知等待 HTTP 请求）
- host 上行 `HostStreamEvent` 由 `AgentSessionService::on_host_stream` 处理：
  1. 写 `agent_turn_events`（slice append）
  2. 调用 `RealtimePublisher::publish_ephemeral(topic, frame)`

**测试矩阵**：

| 测试 | 场景 | 断言 |
|---|---|---|
| `ws_client_handshake.rs` | 合法 ticket → 升级 → 收 Hello → 发 Subscribe | SubscribeAck 帧返回；`subscription_active` Gauge += 1 |
| `ws_ticket_invalid.rs` | 无 ticket / 错 ticket | HTTP 401 + 错误码 |
| `ws_subscribe_denied.rs` | 订他人 conversation | SubscriptionDenied |
| `ws_subscribe_limit.rs` | 一次 33 个 topic | SubscriptionLimitExceeded |
| `ws_resume_within_retention.rs` | resume_after 落在 retention 内 | 收到所有 missed durable，按 topic_seq 递增 |
| `ws_resume_beyond_retention.rs` | resume_after 落在 retention 外 | 收到 SnapshotRequired |
| `ws_supersede.rs` | 同 installation 第二次握手 | 旧连收 4401 |
| `ws_outbox_to_socket.rs` | outbox dispatcher publish → gateway 收到 → 客户端收到 | DurableEvent 帧到达；topic_seq 与 DB 一致 |
| `ws_host_command_ack.rs` | host 上行 ack/result | host_commands 行被更新；HTTP waiter 被唤醒 |
| `ws_host_stream_to_client.rs` | host 上行 stream slice → 写 agent_turn_events → publish | client 端收到 StreamEvent 帧 |
| `ws_heartbeat_timeout.rs` | 客户端不发心跳 30s | 服务端关闭连接，`ws_close_total{reason="heartbeat_timeout"}` += 1 |
| `ws_force_close_on_token_revoke.rs` | host token revoke | host 收到 HostForceClose + 4401 |

**性能基线**：

- 单实例 1k 连接，总 push QPS ≥ 5k 不丢；P99 push 延迟 < 50ms（local docker）

**回退**：

- 若 fan-out QPS 不达标：把 `GatewayListener` 改为按 topic_kind 分 channel（`minos.durable.account` / `...conversation` ...），减小 hot channel 压力

**工作量**：8 单位（4 天）

### P2.S4 — Stream slice 写入 + ephemeral 帧路径（1 天）

**目标**：把 P1.S5 的 cold replay 接口与 P2.S3 的 live 推送串起来，建立 "host uplink → INSERT slice → publish ephemeral" 的统一流水线。

**改动**：

- `src/agent_sessions/use_case.rs`：新增 `record_turn_event(turn_id, kind, payload)`，事务内 INSERT `agent_turn_events`，事务外（or 同一 service 调用方）调用 `publish_ephemeral`
- `src/realtime/wire.rs`：`StreamEvent` 帧 schema 与 `agent_turn_events.kind` 对齐枚举
- `src/realtime/gateway/host.rs::on_host_stream_event`：批量缓冲 + 串行写入，避免单 turn 高频写引起锁等待

**关键约束**：

- INSERT 与 publish 不在同一事务（slice 量大 + 重要性中等）：先 INSERT，失败直接 drop；INSERT 成功后 publish 失败仅记 metric `stream_publish_failed_total`，cold replay 兜底
- slice rate 限制：每 turn 1k events/s 上限；超出按 trailing 丢弃 + 记 metric

**测试**：

- `tests/stream_slice_roundtrip.rs`：发 1000 slice → DB 有 1000 行 + 客户端收 1000 个 StreamEvent
- `tests/stream_slice_publish_failure.rs`：故意 publish 失败 → DB 仍有写入 → cold replay 能拿到全部 slice

**工作量**：2 单位（1 天）

### P2.S5 — Mixed-auth `/devices` deprecation 标记（0.5 天）

**目标**：保留 `/devices` 兼容路径但加可观测，确认 P8 之前没有新流量回流。

**改动**：

- `src/http/ws_devices.rs::upgrade`：进入即记 `ws_legacy_connect_total{principal_kind}` Counter
- 启动期日志：若 `MINOS_ALLOW_LEGACY_DEVICES_WS=false`（默认 false in prod），则注册路由前 panic
- 文档：`docs/ops/ws-deprecation.md` 简介 + 时间表

**验收**：

- prod 配置下 `/devices` 路径不可达
- dev 配置下保留，metric 上报

**工作量**：1 单位（0.5 天）

### P2 完成定义

- [ ] `/ws/client` + `/ws/host` 全 ticket 入口
- [ ] subscribe / resume / replay / SnapshotRequired / supersede / force_close 协议帧实测通过
- [ ] outbox → publisher → listener → gateway → 客户端 端到端链路有专门集成测试
- [ ] host 上行 ack / result / stream slice 全量入库 + 推送
- [ ] `/devices` 加 deprecation metric，prod 默认禁用

### P2 回退总结

- 若 outbox→listener→gateway 高峰下出现重复推送：在 `GatewayListener` 增加去重表（`(conn_id, event_id)` LRU，10s 窗口）
- 若 ticket consume Lua 脚本在 Redis cluster 出问题：退化到 `GETDEL` + 客户端重试

---

## P3 — Domain refactor：agent_session / approval / host_command

**Phase 目标**：把"agent 远程命令面"完整跑在新模型上：agent_session / agent_turn / agent_turn_event 是唯一权威；approval 走专门 service；host 同步请求统一 `host_commands` 表 + dispatcher。删除 `approval_relay.rs` / `host_command_runtime.rs` 内的 in-memory 真理来源（保留 in-process notifier 仅作优化缓存）。

**Phase 完成定义**：

- `/v1/agent-sessions/start|send-input|stop|list|read-turns` 全部按新合同
- `/v1/approvals/respond` 与超时 / 断线 worker 统一通过 `host_commands` 通知 host
- 删除 `crates/minos-backend/src/approval_relay.rs` 与 `host_command_runtime.rs`，改名后的 `HostCommandService` 与 `ApprovalService` 接管
- `thread_id` 在 `pending_approvals` 表回填到 `agent_session_id`（数据迁移在 P3.S5）
- 旧 `EventKind::ApprovalRequested` / `EventKind::Unpaired` / `EventKind::IngestCheckpoint` 不再由后端主动发出（保留 protocol enum 仅供 client cleanup 时识别）

**Phase 输出依赖**：P2 的 host gateway 帧入口已可用（HostCommandAck / Result）。

### P3.S1 — `AgentSessionService` 重写（3 天）

**新增文件**：

- `src/agent_sessions/mod.rs`
- `src/agent_sessions/use_case.rs`
- `src/agent_sessions/dto.rs`
- `src/store/postgres/agent_sessions.rs`、`agent_turns.rs`、`agent_turn_events.rs`
- `src/store/sqlite/agent_sessions.rs`（同上 sqlite 镜像）

**Service 接口**：

```rust
pub struct StartAgentSessionInput {
    pub conversation_id: String,
    pub project_id: Option<String>,
    pub agent_id: String,
    pub host_installation_id: Option<String>,    // 缺省按 host_links 选默认
    pub initial_user_message: Option<String>,
    pub client_request_id: String,               // 必填，用于 idempotency
    pub caller_account_id: String,
}

pub struct StartAgentSessionOutput {
    pub session_id: String,
    pub conversation_id: String,
    pub host_installation_id: String,
    pub started_at_ms: i64,
    pub initial_turn_id: Option<String>,
    pub host_command_id: String,
}

pub struct SendInputInput {
    pub session_id: String,
    pub text: String,
    pub mentions: Vec<String>,                   // account_ids
    pub client_request_id: String,
    pub caller_account_id: String,
}

pub struct ReadTurnsInput {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub after_turn_seq: Option<i64>,
    pub after_event_seq: Option<i64>,
    pub limit: u32,        // ≤ 200
    pub caller_account_id: String,
}

pub struct ReadTurnsOutput {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub turns: Vec<TurnMetaDto>,
    pub events: Vec<TurnEventDto>,
    pub next_turn_seq: Option<i64>,
    pub next_event_seq: Option<i64>,
}

#[async_trait]
pub trait AgentSessionService: Send + Sync {
    async fn start(&self, input: StartAgentSessionInput) -> Result<StartAgentSessionOutput, AgentSessionError>;
    async fn send_input(&self, input: SendInputInput) -> Result<SendInputOutput, AgentSessionError>;
    async fn stop(&self, input: StopInput) -> Result<(), AgentSessionError>;
    async fn list(&self, input: ListInput) -> Result<ListOutput, AgentSessionError>;
    async fn read_turns(&self, input: ReadTurnsInput) -> Result<ReadTurnsOutput, AgentSessionError>;
    async fn on_host_turn_started(&self, frame: HostTurnStarted) -> Result<(), AgentSessionError>;
    async fn on_host_turn_event(&self, frame: HostTurnEvent) -> Result<(), AgentSessionError>;
    async fn on_host_turn_completed(&self, frame: HostTurnCompleted) -> Result<(), AgentSessionError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AgentSessionError {
    #[error("agent_session_not_found")]              NotFound,
    #[error("agent_session_state_invalid")]          StateInvalid,
    #[error("agent_session_host_unavailable")]       HostUnavailable,
    #[error("conversation_forbidden")]               ConversationForbidden,
    #[error("validation_missing_field: {0}")]        ValidationMissing(&'static str),
    #[error("validation_format: {0}")]               ValidationFormat(&'static str),
    #[error(transparent)] Internal(#[from] AppError),
}
```

**`start` 算法（伪码）**：

```rust
pub async fn start(&self, input: StartAgentSessionInput) -> Result<...> {
    self.rate_limits.check_start_per_account(&input.caller_account_id)?;
    self.validate(&input)?;

    let mut tx = self.storage.begin().await?;

    // 1. 校验 caller 是 conversation member
    if !self.repos.conversations.is_member(&mut tx, &input.conversation_id, &input.caller_account_id).await? {
        tx.rollback().await?;
        return Err(AgentSessionError::ConversationForbidden);
    }

    // 2. 选择 host_installation_id
    let host_id = match input.host_installation_id {
        Some(id) => {
            self.repos.host_links.assert_linked(&mut tx, &input.caller_account_id, &id).await?;
            id
        }
        None => self.repos.host_links.pick_default_host(&mut tx, &input.caller_account_id).await?
            .ok_or(AgentSessionError::HostUnavailable)?,
    };

    // 3. project 一致性
    if let Some(project_id) = &input.project_id {
        let conv_proj = self.repos.conversations.project_id(&mut tx, &input.conversation_id).await?;
        if conv_proj.as_ref() != Some(project_id) {
            tx.rollback().await?;
            return Err(AgentSessionError::ValidationFormat("project_id mismatch"));
        }
    }

    // 4. idempotency：若同一 (caller, client_request_id) 已存在，直接返回
    if let Some(existing) = self.repos.agent_sessions.find_by_idempotency(&mut tx, &input.caller_account_id, &input.client_request_id).await? {
        tx.commit().await?;
        return Ok(existing.into());
    }

    let session_id = self.ids.new_session_id();
    let started_at_ms = self.clock.now_ms();

    // 5. agent_sessions row
    self.repos.agent_sessions.insert(&mut tx, NewAgentSession {
        session_id: session_id.clone(),
        conversation_id: input.conversation_id.clone(),
        project_id: input.project_id.clone(),
        host_installation_id: Some(host_id.clone()),
        agent_id: input.agent_id.clone(),
        status: AgentSessionStatus::Pending,
        started_at_ms,
        idempotency_key: Some(input.client_request_id.clone()),
        idempotency_account_id: Some(input.caller_account_id.clone()),
    }).await?;

    // 6. 可选 initial user turn
    let initial_turn_id = if let Some(text) = &input.initial_user_message {
        let turn_id = self.ids.new_turn_id();
        self.repos.agent_turns.insert(&mut tx, NewAgentTurn {
            turn_id: turn_id.clone(),
            session_id: session_id.clone(),
            turn_seq: 0,
            role: TurnRole::User,
            status: TurnStatus::Completed,
            started_at_ms,
            finished_at_ms: Some(started_at_ms),
            summary_text: Some(text.clone()),
            usage_json: None,
        }).await?;
        Some(turn_id)
    } else {
        None
    };

    // 7. durable + outbox
    let cursor = self.repos.durable_events.record(&mut tx, DurableEvent::AgentSessionStarted {
        session_id: session_id.clone(),
        conversation_id: input.conversation_id.clone(),
        project_id: input.project_id.clone(),
        host_installation_id: host_id.clone(),
        agent_id: input.agent_id.clone(),
        at_ms: started_at_ms,
    }).await?;
    self.repos.outbox.enqueue(&mut tx, "agent_session", &cursor.event_id, started_at_ms).await?;

    // 8. host_command
    let command_id = self.ids.new_command_id();
    self.repos.host_commands.enqueue(&mut tx, NewHostCommand {
        command_id: command_id.clone(),
        host_installation_id: host_id.clone(),
        agent_session_id: Some(session_id.clone()),
        method: "agent_session.start".into(),
        params_json: json!({
            "session_id": session_id,
            "agent_id": input.agent_id,
            "project_id": input.project_id,
            "conversation_id": input.conversation_id,
            "initial_user_message": input.initial_user_message,
        }),
        requested_by_account_id: Some(input.caller_account_id.clone()),
        deadline_at_ms: started_at_ms + DEFAULT_START_DEADLINE_MS,
        created_at_ms: started_at_ms,
    }).await?;

    tx.commit().await?;
    self.outbox_notifier.notify_one();

    Ok(StartAgentSessionOutput {
        session_id, conversation_id: input.conversation_id, host_installation_id: host_id,
        started_at_ms, initial_turn_id, host_command_id: command_id,
    })
}
```

**`send_input`**：

- 校验 caller 是 session.conversation member
- session.status 必须 ∈ {pending, running}
- INSERT 新 turn (role=user, status=completed) → durable AgentTurnAppended → outbox → host_command method=`agent_session.send_input`

**`stop`**：

- session.status → 'stopping'
- host_command method=`agent_session.stop`，超时则 worker 强制 → 'stopped'

**`on_host_turn_started` / `on_host_turn_event` / `on_host_turn_completed`**：

- host gateway 收到帧后调用
- 写 `agent_turns` / `agent_turn_events`
- `on_host_turn_completed` 内事务三件套，发 `AgentTurnAppended` durable

**HTTP 改造**：

- `src/http/v1/agent_sessions.rs`：
  - `POST /v1/agent-sessions/start`
  - `POST /v1/agent-sessions/send-input`
  - `POST /v1/agent-sessions/stop`
  - `POST /v1/agent-sessions/list`（按 caller，支持 conversation_id / project_id / status filter）
  - `POST /v1/agent-sessions/read-turns`（双模式，复用 P1.S5 实现，但调用走 service）

- request/response 类型用 `utoipa::ToSchema` 派生 OpenAPI

**测试矩阵**：

| 测试 | 场景 | 断言 |
|---|---|---|
| `start_basic.rs` | 合法输入 | session 行 + initial turn 行 + host_command 行 + durable + outbox 全部存在 |
| `start_idempotent.rs` | 同 caller + client_request_id 调两次 | 第二次返回相同 session_id，host_command 不重复入队 |
| `start_forbidden.rs` | caller 不在 conversation | conversation_forbidden |
| `start_no_host.rs` | caller 没 host_link | agent_session_host_unavailable |
| `send_input_idempotent.rs` | 同 client_request_id 两次 | 仅一条新 turn |
| `send_input_state_invalid.rs` | session 已 stopped | agent_session_state_invalid |
| `read_turns_metadata.rs` | after_turn_seq=k | 返回正确 turns + next_turn_seq |
| `read_turns_events.rs` | turn_id+after_event_seq | 返回正确 events |
| `read_turns_retention_miss.rs` | 落到 retention 外 | realtime_snapshot_required |
| `host_inbound_turn_event.rs` | host gateway 推 stream slice | DB 写入 + StreamEvent 推到 client |
| `host_inbound_turn_completed.rs` | host 完成 turn | DurableEvent.AgentTurnAppended 推到 client |

**Idempotency 设计**：

- `agent_sessions.idempotency_account_id, idempotency_key` 复合 UNIQUE INDEX（partial index：仅 NOT NULL 时唯一）。schema 在 P0.S1 baseline 中预留：

```sql
ALTER TABLE agent_sessions ADD COLUMN idempotency_account_id TEXT;
ALTER TABLE agent_sessions ADD COLUMN idempotency_key TEXT;
CREATE UNIQUE INDEX idx_agent_sessions_idempotency
    ON agent_sessions(idempotency_account_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
```

> 注：上述 `ALTER TABLE` 实际写在 P0.S1 baseline 一开始，不另起 migration；本节文字提示读者 schema 已含此字段。

**工作量**：6 单位（3 天）

### P3.S2 — `HostCommandService` 与 dispatcher（2 天）

**目标**：替换 `host_command_runtime.rs`；提供"enqueue → dispatcher 推 → host ack/result → finish"完整路径；in-memory `Notify` 仅作 HTTP waiter 唤醒缓存。

**新增文件**：

- `src/host_commands/mod.rs`
- `src/host_commands/use_case.rs`
- `src/host_commands/dispatcher.rs`
- `src/host_commands/inbound.rs`
- `src/store/postgres/host_commands.rs`、`src/store/sqlite/host_commands.rs`

**Trait**：

```rust
#[async_trait]
pub trait HostCommandService: Send + Sync {
    async fn enqueue_in_tx(&self, tx: &mut DbTx<'_>, cmd: NewHostCommand) -> Result<(), AppError>;
    async fn await_result<R: DeserializeOwned>(&self, command_id: &str, timeout: Duration) -> Result<R, HostCommandError>;
    async fn on_inbound_ack(&self, command_id: &str, at_ms: i64) -> Result<(), AppError>;
    async fn on_inbound_result(&self, command_id: &str, payload: HostCommandResultPayload) -> Result<(), AppError>;
    async fn force_close_host(&self, host_installation_id: &str, reason: &str) -> Result<(), AppError>;
}

#[derive(Debug, thiserror::Error)]
pub enum HostCommandError {
    #[error("host_command_timeout")]                    Timeout,
    #[error("host_command_rejected")]                   Rejected { code: String, message: String },
    #[error("host_command_host_unavailable")]           HostUnavailable,
    #[error(transparent)] Internal(#[from] AppError),
}
```

**Dispatcher（独立 worker，类似 outbox dispatcher）**：

- 周期扫 `host_commands.status='pending'` 且 `deadline_at_ms > now`
- 对每个 row：调用 `RealtimePublisher::publish_durable` 发 host command 帧到 `host:<inst>` topic（即 outbox 已经覆盖；host_commands 这里不再独立扇出，而是把命令封装为 DurableEvent，写入 durable_event_log + outbox）
- 优势：复用 outbox 重试/dead-letter；劣势：需要一个 `DurableEvent::HostCommandIssued` 变体 + host gateway 端能识别该事件并执行

**等价方案**（最终选用）：把 host_commands 也走 outbox：

```rust
// 在 enqueue_in_tx 内部
let cursor = durable_events.record(tx, DurableEvent::HostCommandIssued {
    command_id: cmd.command_id.clone(),
    host_installation_id: cmd.host_installation_id.clone(),
    method: cmd.method.clone(),
    params: cmd.params_json.clone(),
    deadline_at_ms: cmd.deadline_at_ms,
    at_ms: cmd.created_at_ms,
}).await?;
outbox.enqueue(tx, "host", &cursor.event_id, cmd.created_at_ms).await?;
```

> 这把 host command 的可靠投递交给 outbox dispatcher；HostCommandService 自身只关心：1) DB 状态机 2) HTTP waiter 唤醒 3) 超时 worker（P5）。

**HTTP waiter（in-memory notifier）**：

```rust
struct PendingCommand {
    command_id: String,
    deadline_at_ms: i64,
    sender: oneshot::Sender<Result<Value, HostCommandError>>,
}

pub struct HostCommandService {
    pending: DashMap<String, PendingCommand>,
    /* repos, ids, clock, ... */
}

pub async fn await_result<R>(&self, command_id: &str, timeout: Duration) -> Result<R, HostCommandError> {
    let (tx, rx) = oneshot::channel();
    self.pending.insert(command_id.to_string(), PendingCommand { command_id: command_id.into(), deadline_at_ms: ..., sender: tx });
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(Ok(value))) => Ok(serde_json::from_value(value).map_err(|e| AppError::from(e))?),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_)) | Err(_) => {
            self.pending.remove(command_id);
            // 不主动改 DB；timeout worker 会基于 deadline 处理
            Err(HostCommandError::Timeout)
        }
    }
}

pub async fn on_inbound_result(&self, command_id: &str, payload: HostCommandResultPayload) -> Result<(), AppError> {
    let now = self.clock.now_ms();
    match payload.status.as_str() {
        "ok" => self.repos.host_commands.finish_succeeded(command_id, &payload.result.unwrap_or(Value::Null), now).await?,
        "error" => self.repos.host_commands.finish_failed(command_id, &payload.error.unwrap_or(Value::Null), now).await?,
        _ => return Err(AppError::Validation("invalid host_command result status".into())),
    };
    if let Some((_, pending)) = self.pending.remove(command_id) {
        let _ = pending.sender.send(payload.into_result());
    }
    Ok(())
}
```

**测试**：

- `host_commands_enqueue.rs`：enqueue → DB 行 + durable + outbox
- `host_commands_ack.rs`：on_inbound_ack 后 status='acked'，ack_at_ms 写入
- `host_commands_result_ok.rs`：on_inbound_result(ok) → 'succeeded' + waiter 收到 result
- `host_commands_result_error.rs`：on_inbound_result(error) → 'failed' + waiter 收到 Rejected
- `host_commands_timeout_no_inbound.rs`：waiter await timeout，DB 由 timeout worker（P5）转 'failed'
- `host_commands_late_reply.rs`：waiter 已超时后 host 回 ack/result → 仍写 DB（不 panic）
- `host_commands_dispatcher_via_outbox.rs`：模拟 outbox 推到 host gateway → host 端收到 HostCommandIssued 帧

**工作量**：4 单位（2 天）

### P3.S3 — `ApprovalService` 重写（1.5 天）

**新增文件**：

- `src/approvals/mod.rs`
- `src/approvals/use_case.rs`
- `src/store/postgres/approval_requests.rs`、`src/store/sqlite/approval_requests.rs`

**Trait**：

```rust
#[async_trait]
pub trait ApprovalService: Send + Sync {
    async fn record_request(&self, input: RecordApprovalInput) -> Result<(), ApprovalError>;
    async fn respond(&self, input: RespondInput) -> Result<RespondOutput, ApprovalError>;
    async fn resolve_timeout(&self, request_id: &str) -> Result<bool, ApprovalError>;
    async fn resolve_disconnect_for_account(&self, account_id: &str) -> Result<u32, ApprovalError>;
}

pub struct RespondInput {
    pub request_id: String,
    pub decision: serde_json::Value,
    pub client_request_id: String,
    pub caller_account_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    #[error("approval_not_found")]            NotFound,
    #[error("approval_already_resolved")]     AlreadyResolved,
    #[error("approval_deadline_passed")]      DeadlinePassed,
    #[error("conversation_forbidden")]        Forbidden,
    #[error(transparent)] Internal(#[from] AppError),
}
```

**`record_request` 来源**：host gateway 收到 `host_command_result` 中携带的 approval payload 后调用（典型场景：host 的 codex agent 执行 tool call 触发审批）。

**`respond` 算法**：

1. 校验 approval 行存在 + state='pending' + caller 是 session.conversation member
2. 事务内：
   - UPDATE approval_requests SET state='decided', resolved_at_ms=now, resolution_json={...}
   - DurableEvent::ApprovalResolved
   - outbox.enqueue
   - host_commands.enqueue method='approval.decision'，params 含 request_id + decision
3. commit + waiter notify（如有）

**`resolve_timeout`**：worker 触发；同上但 resolution=Timeout，host_command params 携带 `auto_decision`（按 method 选自动决策，参考现有 `auto_reject_decision`）

**`resolve_disconnect_for_account`**：worker 触发；遍历 host_links → 若所有 account session 都断线，且 host 上有 pending approval，则 `Disconnect` resolve

**测试**：覆盖 record / respond / state-invalid / forbidden / timeout / disconnect 五条路径，断言 DB + durable + outbox + host_command 全 ok

**工作量**：3 单位（1.5 天）

### P3.S4 — `ingest/mod.rs` 改造（1 天）

**目标**：保留协议翻译能力，但去掉直接 fan-out；改为只产出 `agent_turns` / `agent_turn_events` + DurableEvent。

**改动**：

- `src/ingest/mod.rs`：删除内部对 `RealtimeFanout` 的依赖；接受 `Arc<AgentSessionService>`
- `src/ingest/translate.rs`：保留
- `src/ingest/use_case.rs`：旧入口改为内部 helper，对外只暴露 `IngestService`

**测试**：

- `ingest_message_roundtrip.rs`：模拟 host 发 codex 协议 → 翻译成 turn event → 落库 → 客户端通过订阅收到

**工作量**：2 单位（1 天）

### P3.S5 — 旧表/字段迁移：`pending_approvals` → `approval_requests`（0.5 天）

**目标**：把现有 `pending_approvals.thread_id` 数据回填到 `approval_requests`，然后 drop 旧表。

**Migration（postgres，0002_approval_migration.sql）**：

```sql
INSERT INTO approval_requests (request_id, agent_session_id, turn_id, method, params_json, state, deadline_at_ms, created_at_ms, resolved_at_ms, resolution_json)
SELECT
    p.request_id,
    -- thread_id 在历史数据里其实是 session_id 的别名（旧 codex 路径）
    p.thread_id AS agent_session_id,
    p.turn_id,
    p.method,
    p.params_json::jsonb,
    CASE
        WHEN p.resolved_at_ms IS NULL THEN 'pending'::approval_state
        WHEN p.resolution = 'user_decision' THEN 'decided'::approval_state
        WHEN p.resolution = 'timeout' THEN 'timeout'::approval_state
        WHEN p.resolution = 'disconnected' THEN 'disconnected'::approval_state
        ELSE 'decided'::approval_state
    END,
    p.timeout_at_ms,
    p.created_at_ms,
    p.resolved_at_ms,
    NULL::jsonb
FROM pending_approvals p
ON CONFLICT (request_id) DO NOTHING;

DROP TABLE pending_approvals;
```

> 注意：`pending_approvals` 在 baseline 中并不存在（baseline 已是新表），此 migration 仅用于已上线 prod 实例的回填。在 dev/test 环境通过 baseline 初始化时直接跳过。

**测试**：

- `migration_p3s5.rs`：手动构造旧 `pending_approvals` 数据 → 跑 migration → 校对 `approval_requests` 行内容等价

**工作量**：1 单位（0.5 天）

### P3 完成定义

- [ ] `agent_sessions / agent_turns / agent_turn_events` 是 agent 命令面唯一权威
- [ ] `host_commands` 是后端 → host 同步请求唯一权威；in-memory 仅缓存等待
- [ ] `ApprovalService` 把 respond / timeout / disconnect 三路统一通过 `host_commands`
- [ ] `approval_relay.rs` / `host_command_runtime.rs` 删除（或改为薄 deprecated wrapper，发 lint warning）
- [ ] 已退役 `/v1/me/*` / `/v1/threads/*` / `/devices` 调用点没有回归

### P3 回退总结

- 若 idempotency 性能不达预期：把 `idempotency_key` 改为带 expire 的 cache 优先 → DB 复核
- 若 host_command 通过 outbox 投递引入额外延迟：保留备选实现（直接 publish_ephemeral 到 `host:<id>`），通过 config flag 切换

---

## P4 — Conversation / Project / Social 收敛

**Phase 目标**：把现有 2738 行 `social.rs` 拆分为 conversation / profile / friend / project 四个独立 service；把 conversation 的"agent 作为 sender"模型并入；把 project 升级为带 membership / default agents / archive 的 aggregate。

**Phase 完成定义**：

- `src/social/mod.rs` 退役（保留 reexport stub，仅供 P8 删除）
- `/v1/conversations/*` / `/v1/profiles/*` / `/v1/friends*` / `/v1/projects/*` 各自模块；handler 文件 ≤ 600 行
- conversation send-message 与 agent send-message 在领域层合一
- project 支持 archive；agent_session 必须与 project_id 一致校验在领域层（P3.S1 已有，P4 仅补全 project 侧 aggregate）

**Phase 输出依赖**：P3 的 AgentSessionService、HostCommandService。

### P4.S1 — Profile / Friend service（1 天）

**新增文件**：

- `src/profiles/mod.rs`、`src/profiles/use_case.rs`、`src/store/postgres/profiles.rs`
- `src/friends/mod.rs`、`src/friends/use_case.rs`、`src/store/postgres/friendships.rs`
- `src/http/v1/profiles.rs`、`src/http/v1/friends.rs`

**Profile service**：

- `get_self`、`set_minos_id`、`set_display_name`、`search_by_minos_id`、`bulk_load(account_ids)`
- minos_id 校验复用 `validate_minos_id` 但抽到 `profiles::validate`
- DurableEvent `AccountProfileUpdated { account_id, fields_changed }` → topic `account:<id>`

**Friend service**：

- `create_request`、`accept`、`reject`、`cancel`、`list`、`list_friends`
- 事务三件套：`FriendRequestCreated` / `FriendRequestResolved` → topic `account:<from>` & `account:<to>`

**测试**：

- `profile_basic.rs`、`profile_minos_id_validation.rs`、`profile_search.rs`
- `friend_request_flow.rs`、`friend_request_state_guard.rs`

**工作量**：2 单位（1 天）

### P4.S2 — `ConversationService` 重写（2 天）

**新增文件**：

- `src/conversations/mod.rs`
- `src/conversations/use_case.rs`
- `src/store/postgres/conversations.rs`
- `src/store/postgres/conversation_messages.rs`
- `src/store/postgres/conversation_members.rs`
- `src/http/v1/conversations.rs`（重写）

**Trait（节选）**：

```rust
#[async_trait]
pub trait ConversationService: Send + Sync {
    async fn ensure_direct(&self, input: EnsureDirectInput) -> Result<ConversationDto, ConversationError>;
    async fn create_group(&self, input: CreateGroupInput) -> Result<ConversationDto, ConversationError>;
    async fn list(&self, input: ListInput) -> Result<ListOutput, ConversationError>;
    async fn list_members(&self, input: ListMembersInput) -> Result<ListMembersOutput, ConversationError>;
    async fn add_members(&self, input: AddMembersInput) -> Result<(), ConversationError>;
    async fn remove_member(&self, input: RemoveMemberInput) -> Result<(), ConversationError>;
    async fn add_agent(&self, input: AddAgentInput) -> Result<(), ConversationError>;
    async fn remove_agent(&self, input: RemoveAgentInput) -> Result<(), ConversationError>;
    async fn send_message(&self, input: SendMessageInput) -> Result<SendMessageOutput, ConversationError>;
    async fn recall_message(&self, input: RecallInput) -> Result<(), ConversationError>;
    async fn list_messages(&self, input: ListMessagesInput) -> Result<ListMessagesOutput, ConversationError>;
    async fn mark_read(&self, input: MarkReadInput) -> Result<(), ConversationError>;
    async fn agent_send_message(&self, input: AgentSendMessageInput) -> Result<SendMessageOutput, ConversationError>;
}
```

**`send_message` 算法**：

```rust
1. self.rate_limits.check_send_message(caller_account_id)
2. 校验 caller 是 conversation member
3. 校验 reply_to_message_id（若有）属于本 conversation
4. 提取 mentions：解析 @minos_id 或 explicit account_ids
5. tx.begin
   - INSERT conversation_messages(sender_kind='user', sender_account_id, body_json={ "type": "text", "text": ... })
   - UPDATE conversations.updated_at_ms
   - INSERT message_mentions
   - DurableEvent::ConversationMessageAppended → topic conversation:<id>
   - 对每个 mention：DurableEvent::MentionedInConversation → topic account:<mentioned>
   - outbox.enqueue
6. tx.commit
```

**`agent_send_message`**：

- 由 AgentSessionService 调用（不暴露 HTTP）
- sender_kind='agent', sender_agent_id, agent_session_id
- DurableEvent 同上但 SenderRef::Agent

**Recall 窗口校验**：5 分钟（与现有 `RECALL_WINDOW_MS` 对齐），过窗返回 `conversation_message_recall_window_passed`

**测试矩阵**：

| 测试 | 场景 | 断言 |
|---|---|---|
| `direct_conv_idempotent.rs` | 两次 ensure_direct(A,B) | 同一 conversation_id |
| `group_create.rs` | create_group([A,B,C]) | 3 个 member 行；A 是 owner |
| `send_message_basic.rs` | 普通文本 + reply_to | message + durable + outbox + updated_at_ms |
| `send_message_mention.rs` | @user1 @user2 | 两条 message_mentions + 两个 account topic durable |
| `send_message_forbidden.rs` | 非 member 发送 | conversation_forbidden |
| `recall_within_window.rs` | 5 分钟内 recall | recalled_at_ms set |
| `recall_window_passed.rs` | 6 分钟后 recall | conversation_message_recall_window_passed |
| `add_agent_member.rs` | add_agent(agent_codex) | conversation_agent_members 行；durable AgentJoinedConversation |
| `agent_send_message_via_session.rs` | AgentSessionService 触发 agent send | sender_kind='agent', agent_session_id 写入 |

**工作量**：4 单位（2 天）

### P4.S3 — `ProjectService` 扩展（1.5 天）

**改动文件**：

- `src/projects/mod.rs`（重写）
- `src/projects/use_case.rs`
- `src/store/postgres/projects.rs`、`project_members.rs`、`project_default_agents.rs`
- `src/http/v1/projects.rs`（重写）

**Trait**：

```rust
#[async_trait]
pub trait ProjectService: Send + Sync {
    async fn create(&self, input: CreateInput) -> Result<ProjectDto, ProjectError>;
    async fn rename(&self, input: RenameInput) -> Result<ProjectDto, ProjectError>;
    async fn archive(&self, input: ArchiveInput) -> Result<(), ProjectError>;
    async fn list(&self, input: ListInput) -> Result<ListOutput, ProjectError>;
    async fn invite_member(&self, input: InviteMemberInput) -> Result<(), ProjectError>;
    async fn remove_member(&self, input: RemoveMemberInput) -> Result<(), ProjectError>;
    async fn link_conversation(&self, input: LinkConversationInput) -> Result<(), ProjectError>;
    async fn list_agent_sessions(&self, input: ListAgentSessionsInput) -> Result<ListAgentSessionsOutput, ProjectError>;
    async fn set_default_agents(&self, input: SetDefaultAgentsInput) -> Result<(), ProjectError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("project_not_found")]              NotFound,
    #[error("project_archived")]               Archived,
    #[error("project_workspace_conflict")]     WorkspaceConflict,
    #[error("project_forbidden")]              Forbidden,
    #[error("validation_format: {0}")]         ValidationFormat(&'static str),
    #[error(transparent)] Internal(#[from] AppError),
}
```

**`link_conversation` 一致性**：

- 在 tx 内：UPDATE conversations.project_id ← project_id；同时校验所有该 conversation 现有 agent_sessions.project_id 与新 project_id 一致或为 NULL（NULL 一并 backfill）
- DurableEvent::ProjectConversationLinked

**`archive`**：

- 写 archived_at_ms；durable
- 后续业务逻辑（agent_sessions/start 等）必须校验 `project_archived`

**HTTP**：路径与 architecture-overview / formal-development 对齐；`/v1/projects/list-conversations`、`/v1/projects/agent-sessions/query` 等。`/v1/projects/threads/*` 兼容别名删除（在 P8）。

**工作量**：3 单位（1.5 天）

### P4.S4 — `social.rs` 拆解 + 迁移文件清单（0.5 天）

**改动**：

- 把现有 `src/social/mod.rs` 中：
  - profile / minos_id / display_name → `src/profiles/`
  - friend / friend_request → `src/friends/`
  - conversation / message / mention / read → `src/conversations/`
  - agent member / agent dispatch → `src/conversations/agents.rs`
- 保留 `src/social/mod.rs` 仅 reexport（标记 `#[deprecated]`）
- `src/store/social.rs`（2738 行）拆为 `src/store/postgres/{profiles,friendships,conversations,conversation_messages,conversation_members}.rs`，`store/sqlite` 镜像
- `src/http/v1/social.rs` 拆解为 `profiles.rs` / `friends.rs` / `conversations.rs`（< 600 行/文件）

**测试**：

- 现有 `tests/v1_social.rs` 全数通过；按拆分后路径分批替换为 `v1_profiles.rs` / `v1_friends.rs` / `v1_conversations.rs`

**工作量**：1 单位（0.5 天）

### P4 完成定义

- [ ] social.rs 仅是 deprecated reexport
- [ ] 四组 service 各自独立、各自有 < 600 行的 HTTP handler
- [ ] conversation send-message 在领域层与 agent send-message 一体
- [ ] project archive / link_conversation 一致性受 tx 保护
- [ ] 旧 `tests/v1_social.rs` 拆分后全量绿

### P4 回退总结

- 若 conversations.send_message 在 mention 多账号下出现 outbox 写入热点：mention 派生的 `account:<id>` durable 事件改为合并成单条 fan-out 帧，由 push fanout worker 在 P6 拆分到具体收件人
- 若 social.rs 拆解期间出现接口回归：保留 `MINOS_USE_LEGACY_SOCIAL=true` 配置位，临时回退到旧 module（仅限灰度，正式开发前必须关闭）



---

## P5 — Worker Plane 全员上线

**Phase 目标**：把所有后台任务从"零散散布在 service 内部 + spawn"集中到统一的 `jobs` 模块；统一调度器、统一指标、统一可在 `worker-only` 模式下独立部署。

**Phase 完成定义**：

- 所有 worker 通过 `JobRegistry` 注册 + `JobSupervisor` 启动；`MINOS_RUNTIME_MODE=worker-only` 启动后唯一活动是 worker plane（HTTP 端口不监听）
- 每个 worker 有：心跳、metrics、tracing span、可观测的健康检查 endpoint
- worker 故障（panic）能自动重启，且重启次数受限（指数退避，最大 5 次/15 分钟）
- 所有数据库操作避免长事务（>5s 的 worker 必须分批 + commit per batch）

**Phase 输出依赖**：P3（HostCommandService / ApprovalService）、P1（OutboxRepository）、P2（RealtimePublisher / GatewayListener）。

### P5.S0 — Job framework 基础设施（1 天）

**新增文件**：

- `src/jobs/mod.rs`
- `src/jobs/registry.rs`
- `src/jobs/supervisor.rs`
- `src/jobs/job_trait.rs`
- `src/jobs/health.rs`

**核心 trait**：

```rust
#[async_trait]
pub trait Job: Send + Sync + 'static {
    /// Unique job name, used for metrics labels and logs.
    fn name(&self) -> &'static str;

    /// Should this job run under the current runtime mode?
    /// e.g. some jobs run only on `worker-only`, some run on `monolith` too.
    fn applies_to(&self, mode: RuntimeMode) -> bool;

    /// One iteration of the job. Implementations should return:
    ///  - `Ok(JobOutcome::Idle)`        when there is no work; supervisor sleeps
    ///  - `Ok(JobOutcome::DidWork(n))`  when n items were processed
    ///  - `Err(JobError::Transient)`    transient errors; supervisor backs off
    ///  - `Err(JobError::Fatal)`        fatal; supervisor stops the job and pages
    async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError>;

    /// Configurable poll interval when idle (default 1s; some jobs override).
    fn idle_interval(&self) -> Duration { Duration::from_secs(1) }

    /// Soft deadline per tick (default 30s).
    fn tick_deadline(&self) -> Duration { Duration::from_secs(30) }

    /// If true, supervisor only runs ONE concurrent tick at any time (default).
    fn singleton_tick(&self) -> bool { true }
}

pub enum JobOutcome { Idle, DidWork(u32) }

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("transient: {0}")] Transient(String),
    #[error("fatal: {0}")]     Fatal(String),
}

pub struct JobContext {
    pub app: Arc<AppContext>,
    pub clock: Arc<dyn Clock>,
    pub worker_id: String,
}
```

**`JobSupervisor` 调度算法**：

```rust
pub struct JobSupervisor {
    jobs: Vec<Arc<dyn Job>>,
    handles: Vec<JoinHandle<()>>,
    health: Arc<JobHealthRegistry>,
}

impl JobSupervisor {
    pub fn start(jobs: Vec<Arc<dyn Job>>, ctx: Arc<AppContext>, mode: RuntimeMode) -> Self {
        let health = Arc::new(JobHealthRegistry::default());
        let mut handles = Vec::new();
        for job in jobs.iter().filter(|j| j.applies_to(mode)) {
            let job_cl = Arc::clone(job);
            let ctx_cl = Arc::clone(&ctx);
            let health_cl = Arc::clone(&health);
            let h = tokio::spawn(async move {
                run_job_loop(job_cl, ctx_cl, health_cl).await;
            });
            handles.push(h);
        }
        Self { jobs, handles, health }
    }

    pub async fn shutdown(self) {
        for h in self.handles { h.abort(); }
    }
}

async fn run_job_loop(job: Arc<dyn Job>, ctx: Arc<AppContext>, health: Arc<JobHealthRegistry>) {
    let mut backoff = ExponentialBackoff::new(Duration::from_secs(1), Duration::from_secs(60), 0.2);
    let mut consecutive_fatal = 0u32;
    loop {
        let started = Instant::now();
        let job_ctx = JobContext { app: Arc::clone(&ctx), clock: Arc::clone(&ctx.clock), worker_id: format!("{}/{}", ctx.instance_id, job.name()) };
        let result = tokio::time::timeout(job.tick_deadline(), job.tick(&job_ctx)).await;
        let elapsed = started.elapsed();
        metrics::JOB_TICK_DURATION.with_label_values(&[job.name()]).observe(elapsed.as_secs_f64());
        match result {
            Ok(Ok(JobOutcome::Idle)) => {
                health.record_ok(job.name());
                metrics::JOB_TICK_TOTAL.with_label_values(&[job.name(), "idle"]).inc();
                tokio::time::sleep(job.idle_interval()).await;
                backoff.reset();
            }
            Ok(Ok(JobOutcome::DidWork(n))) => {
                health.record_ok(job.name());
                metrics::JOB_TICK_TOTAL.with_label_values(&[job.name(), "ok"]).inc_by(u64::from(n));
                backoff.reset();
            }
            Ok(Err(JobError::Transient(msg))) => {
                health.record_transient(job.name(), &msg);
                metrics::JOB_TICK_TOTAL.with_label_values(&[job.name(), "transient"]).inc();
                tokio::time::sleep(backoff.next()).await;
            }
            Ok(Err(JobError::Fatal(msg))) => {
                consecutive_fatal += 1;
                health.record_fatal(job.name(), &msg);
                metrics::JOB_TICK_TOTAL.with_label_values(&[job.name(), "fatal"]).inc();
                if consecutive_fatal >= 5 {
                    tracing::error!(job = job.name(), "job exceeded fatal-retry budget; stopping");
                    return;
                }
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
            Err(_elapsed) => {
                health.record_timeout(job.name());
                metrics::JOB_TICK_TOTAL.with_label_values(&[job.name(), "timeout"]).inc();
                tokio::time::sleep(backoff.next()).await;
            }
        }
    }
}
```

**Health endpoint**：

- `GET /health/jobs` → JSON：每个 job 的 `last_ok_at_ms`、`last_error`、`consecutive_failures`
- 用于 Kubernetes liveness probe（worker-only deployment）

**Metrics**：

```
job_tick_total{job, result}        Counter
job_tick_duration_seconds{job}     Histogram
job_last_success_age_seconds{job}  Gauge (now - last_ok_at_ms)
job_consecutive_failures{job}      Gauge
```

**测试**：

- `tests/jobs_supervisor_basic.rs`：fake job 返回 Idle / DidWork / Transient / Fatal / Timeout，断言 supervisor 行为符合表述
- `tests/jobs_supervisor_fatal_budget.rs`：5 次连续 Fatal 后 supervisor 停止该 job，但不影响其他 job
- `tests/jobs_health_endpoint.rs`：HTTP 拉 `/health/jobs` 返回正确 JSON

**工作量**：2 单位（1 天）

### P5.S1 — `OutboxDispatcherJob` 接入 framework（0.5 天）

**目标**：把 P1.S3 的 dispatcher 改造成符合 `Job` trait 的形态，去掉裸 `tokio::spawn`。

**改动**：

- `src/jobs/outbox_dispatcher.rs`：实现 `Job for OutboxDispatcherJob`
  - `tick()`：调用 `repos.outbox.claim(...)`；返回 `DidWork(n)` 或 `Idle`
  - `applies_to(mode)`：`mode.runs_supervised_workers()`
  - `idle_interval()`：可配置（默认 200ms，对外通过 `MINOS_OUTBOX_IDLE_MS`）
- `src/jobs/mod.rs::default_jobs`：把 dispatcher 加入注册表

**测试**：复用 P1.S3 的测试，确认 framework 化之后行为一致

**工作量**：1 单位（0.5 天）

### P5.S2 — `ApprovalTimeoutJob`（1 天）

**新增文件**：`src/jobs/approval_timeout.rs`

**职责**：定期扫 `approval_requests.state='pending' AND deadline_at_ms <= now`，逐条调用 `ApprovalService::resolve_timeout`。

**SQL（PG）**：

```sql
SELECT request_id
  FROM approval_requests
 WHERE state = 'pending'
   AND deadline_at_ms <= $1
 ORDER BY deadline_at_ms
 LIMIT $2
 FOR UPDATE SKIP LOCKED;
```

> 实际实现把行 lock 让 `ApprovalService::resolve_timeout` 内部处理；此处 SELECT 仅为候选拉取。

**`tick()` 算法**：

```rust
async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError> {
    let now = ctx.clock.now_ms();
    let candidates = self.repos.approvals.list_expired_pending(now, BATCH).await
        .map_err(|e| JobError::Transient(e.to_string()))?;
    if candidates.is_empty() { return Ok(JobOutcome::Idle); }
    let mut count = 0;
    for request_id in candidates {
        match self.approvals.resolve_timeout(&request_id).await {
            Ok(true) => count += 1,
            Ok(false) => { /* 已被别人 resolve */ }
            Err(error) => {
                tracing::warn!(?error, request_id, "approval timeout resolve failed");
                metrics::APPROVAL_TIMEOUT_FAILED.inc();
            }
        }
    }
    Ok(JobOutcome::DidWork(count as u32))
}
```

**事件驱动唤醒**：`ApprovalService::record_request` 在 commit 后调用 `approval_notifier.notify_one()`；`ApprovalTimeoutJob::tick` 在 idle 时 `select! { _ = sleep(idle), _ = approval_notifier.notified() }`。

**Metrics**：

```
approval_timeout_resolved_total          Counter
approval_timeout_failed_total            Counter
approval_pending_oldest_age_seconds      Gauge (now - min(created_at_ms))
```

**测试**：

- `approval_timeout_basic.rs`：插入 deadline 已过的 pending → tick → state='timeout'
- `approval_timeout_no_double_resolve.rs`：两个并发 worker 抢同一行 → 只一个成功
- `approval_timeout_late_arrival.rs`：tick 后 1s host 才回复 → host_command 仍写入；不再 resolve 一次

**工作量**：2 单位（1 天）

### P5.S3 — `HostCommandTimeoutJob`（0.5 天）

**新增文件**：`src/jobs/host_command_timeout.rs`

**职责**：扫描 `host_commands.status IN ('pending','acked') AND deadline_at_ms <= now`；调用 `HostCommandService::on_timeout(command_id)` 把 status → 'failed', error_json={kind:'timeout'}；通过 in-memory notifier 通知等待者。

**特殊处理**：

- "late reply 宽限期"：标 timeout 的同时记 `timed_out_at_ms`，30s 内若 host 回复则仍写 result（覆盖 error_json，将 status 重新转回 'succeeded'）
- 这样允许 client 看到一个先失败再成功的语义；`HostCommandService::await_result` 返回 timeout 后不再回收 → late reply 仅写 DB，不 race waiter

**SQL（PG, claim+update）**：

```sql
WITH cte AS (
  SELECT command_id
    FROM host_commands
   WHERE status IN ('pending','acked')
     AND deadline_at_ms <= $1
   ORDER BY deadline_at_ms
   LIMIT $2
   FOR UPDATE SKIP LOCKED
)
UPDATE host_commands h
   SET status = 'failed',
       error_json = jsonb_build_object('kind','timeout','timeout_ms', $1 - h.created_at_ms),
       finished_at_ms = $1
  FROM cte
 WHERE h.command_id = cte.command_id
RETURNING h.command_id;
```

**Metrics**：

```
host_command_timeout_total          Counter
host_command_inflight               Gauge (status IN ('pending','acked'))
```

**测试**：

- `host_command_timeout_basic.rs`：deadline 过期 → tick → 'failed' + waiter 收到 Timeout
- `host_command_late_reply_after_timeout.rs`：tick 后 host 回复 → 写入 result_json，status 维持 'failed' 但 metadata `late_resolution = true`（视产品决定是否覆盖；本计划默认不覆盖）

**工作量**：1 单位（0.5 天）

### P5.S4 — `RetentionCleanerJob`（1 天）

**新增文件**：`src/jobs/retention_cleaner.rs`

**职责**：

1. `durable_event_log` retention：按 topic_kind 分批删除 `created_at_ms <= now - retention_window` 且没有 unacked outbox 引用的行
2. `agent_turn_events` retention：删除已 ended session 关闭超过 7 天的 events

**配置（默认值，配置位 `MINOS_RETENTION_*`）**：

| 项 | 默认 |
|---|---|
| `account` durable | 30 天 / 5000 条/topic（取较短） |
| `conversation` durable | 30 天 / 10000 条/topic |
| `project` durable | 90 天 |
| `agent_session` durable | 14 天 |
| `host` durable | 7 天 |
| `agent_turn_events` | session ended 后 7 天 |

**`durable_event_log` 清理 SQL（PG，分批）**：

```sql
WITH expired AS (
  SELECT d.topic_kind, d.event_id
    FROM durable_event_log d
   WHERE d.topic_kind = $1
     AND d.created_at_ms <= $2
     AND NOT EXISTS (
         SELECT 1 FROM outbox_events o
          WHERE o.topic_kind = d.topic_kind
            AND o.event_id   = d.event_id
            AND o.ack_at_ms  IS NULL
     )
   LIMIT $3
)
DELETE FROM durable_event_log d
 USING expired e
 WHERE d.topic_kind = e.topic_kind
   AND d.event_id   = e.event_id
RETURNING d.topic_kind, d.event_id;
```

> 单批默认 1000 行；`tick` 内分批 commit，避免长事务导致 vacuum 压力。

**`agent_turn_events` 清理 SQL（PG）**：

```sql
WITH expired_turns AS (
    SELECT t.turn_id
      FROM agent_turns t
      JOIN agent_sessions s ON s.session_id = t.agent_session_id
     WHERE s.ended_at_ms IS NOT NULL
       AND s.ended_at_ms <= $1
     LIMIT $2
)
DELETE FROM agent_turn_events e
 USING expired_turns x
 WHERE e.turn_id = x.turn_id;
```

**Per-topic 行数上限**：另一支 SQL：

```sql
WITH ranked AS (
  SELECT event_id, topic_seq, ROW_NUMBER() OVER (PARTITION BY topic ORDER BY topic_seq DESC) AS rn
    FROM durable_event_log
   WHERE topic_kind = $1
)
DELETE FROM durable_event_log d
 USING ranked r
 WHERE d.event_id = r.event_id
   AND r.rn > $2
   AND NOT EXISTS (SELECT 1 FROM outbox_events o WHERE o.topic_kind = $1 AND o.event_id = d.event_id AND o.ack_at_ms IS NULL);
```

**Metrics**：

```
retention_deleted_total{table, topic_kind}        Counter
retention_skipped_unacked_total{topic_kind}       Counter
retention_age_oldest_seconds{topic_kind}          Gauge
```

**测试**：

- `retention_age_window.rs`：插入 1000 行 created_at_ms 跨越窗口前后 → tick → 仅窗口外被删
- `retention_unacked_skip.rs`：有 unacked outbox → 不删
- `retention_per_topic_cap.rs`：单 topic 超 cap → 删除最老的多余行
- `retention_agent_turn_events.rs`：session 关闭 8 天后 → events 删除；6 天后 → 保留

**工作量**：2 单位（1 天）

### P5.S5 — `StaleSessionSweeperJob`（1 天）

**新增文件**：`src/jobs/stale_session_sweeper.rs`

**职责**：

1. 清理 Redis presence 过期条目：每个 gateway 进程在 `Hello` 时把 `(account, installation, conn_id)` 写入 `minos:presence:<account>:<installation>` Hash 字段，并设置 expiry by 心跳；missed N 次心跳后被 sweeper 移除
2. 触发 `ApprovalService::resolve_disconnect_for_account`：account 全部 installation 都断线时，把该 account 关联的 host 的 pending approval 转 'disconnected'
3. 标记 agent_sessions：若 session.host_installation_id 对应 host 长期断线（> 5 分钟），且 session.status='running'，把 status → 'failed'，触发 DurableEvent + outbox

**Redis presence 维护**：

- gateway 进程心跳每 25s 调用 `HSET minos:presence:<account>:<installation> <conn_id> <last_pong_ms>`
- sweeper 扫所有 keys：`HGETALL` → 删除 `now - last_pong_ms > 60s` 的字段；HASH 为空时 `DEL`

**伪码**：

```rust
async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError> {
    let now = ctx.clock.now_ms();
    let mut total = 0;

    // 1. 清理 presence
    let stale = self.presence.sweep_stale(now - 60_000).await
        .map_err(|e| JobError::Transient(e.to_string()))?;
    total += stale.len() as u32;

    for entry in &stale {
        self.approvals.resolve_disconnect_for_account(&entry.account_id).await.ok();
    }

    // 2. 标记 host 长期断线后的 agent_sessions
    let cutoff = now - 5 * 60_000;
    let dead_hosts = self.presence.list_hosts_offline_since(cutoff).await
        .map_err(|e| JobError::Transient(e.to_string()))?;
    for host_id in dead_hosts {
        let count = self.agent_sessions.fail_running_for_host(&host_id).await.unwrap_or(0);
        total += count;
    }

    if total == 0 { Ok(JobOutcome::Idle) } else { Ok(JobOutcome::DidWork(total)) }
}
```

**Metrics**：

```
presence_stale_swept_total                 Counter
agent_sessions_failed_due_to_host_total    Counter
presence_hosts_offline                     Gauge
```

**测试**：

- `stale_presence_swept.rs`：write presence with old last_pong → tick → entry 被删
- `disconnect_resolves_approval.rs`：account 全部断线 + 有 pending approval → tick → approval 转 'disconnected' + host_command 入队
- `host_long_offline_fails_session.rs`：host >5min 没心跳 → running session → 'failed'

**工作量**：2 单位（1 天）

### P5.S6 — `RefreshTokenGcJob`（0.5 天）

**新增文件**：`src/jobs/refresh_token_gc.rs`

**职责**：每小时清理 `refresh_tokens.revoked_at_ms IS NOT NULL OR expires_at_ms < now`，限制单批 5000，分批 commit。

**SQL（PG）**：

```sql
DELETE FROM refresh_tokens
 WHERE token_hash IN (
   SELECT token_hash FROM refresh_tokens
    WHERE (revoked_at_ms IS NOT NULL AND revoked_at_ms <= $1)
       OR (expires_at_ms <= $2)
    ORDER BY COALESCE(revoked_at_ms, expires_at_ms)
    LIMIT $3
 )
RETURNING token_hash;
```

**`idle_interval()` 默认 1 小时**，但通过 `notify` 在被显式 `revoke_all_for_account` 之后立即唤醒一次。

**测试**：

- `refresh_gc_basic.rs`：插入 expired/revoked rows → tick → 全删
- `refresh_gc_batching.rs`：插入 12k 行 → 三次 tick 完成

**工作量**：1 单位（0.5 天）

### P5.S7 — `AuditIndexerJob`（0.5 天）

**新增文件**：`src/jobs/audit_indexer.rs`

**职责**：

- 把 `audit_events` 中超过 N 天的行导出到对象存储（S3/GCS/兼容 endpoint），并从 DB 中删除
- 每条导出记录在自己的"audit-archive index"里（也是 JSON Lines + 月度文件）
- 配置位：`MINOS_AUDIT_ARCHIVE_*`；若未配置 endpoint 则不归档（仅做 DB GC，按保留窗口）

**实现要点**：

- 导出文件命名 `audit_archive/<yyyymm>/<topic>.jsonl.gz`
- 使用 `aws-sdk-s3` 或兼容 SDK；本计划仅给出 trait：

```rust
#[async_trait]
pub trait AuditArchiveSink: Send + Sync {
    async fn append(&self, partition: &str, lines: &[Bytes]) -> Result<(), AppError>;
}
```

- 默认 `NoopSink`（仅记 metric `audit_archive_skipped_total`，不导出，但也不删除超龄行）

**测试**：

- 用 `tempfile::tempdir` 实现一个 `LocalFsSink`，断言 partition 文件按内容生成
- 故障注入：sink 失败 → DB 行不被删除，metric `audit_archive_failed_total` += 1

**工作量**：1 单位（0.5 天）

### P5 完成定义

- [ ] `JobRegistry` 注册全部 7 个 worker（outbox / approval timeout / host command timeout / retention / stale session / refresh gc / audit indexer），未来 P6 push fanout 也以同模式接入
- [ ] `MINOS_RUNTIME_MODE=worker-only` 启动后 `/metrics` 暴露 `job_*` 指标 + `/health/jobs` 可读
- [ ] 单元测试覆盖每个 job 的 happy / transient / fatal / timeout 四路
- [ ] kubernetes / docker compose 示例（`deploy/`）能直接以 worker-only 拉起 worker plane（在 P9 完成 deploy 文档；本 phase 仅保证可运行）

### P5 回退总结

- 若 PG 在高并发 `FOR UPDATE SKIP LOCKED` 下 vacuum 压力过大：把 outbox / host_command timeout 的 batch 缩小到 16，并把 idle_interval 上调到 1s
- 若 audit archive 因外部存储不可用导致大量 transient：临时把 `applies_to(mode)` 改为 false（停用），从 `MINOS_AUDIT_ARCHIVE_ENABLED=false` 关闭

---

## P6 — Notification（APNs / FCM / SMTP + Push Fanout）

**Phase 目标**：把"账号有未读消息 / 被提及 / 审批超时"等事件转化为外部 push 通道；客户端能注册/注销 push token；服务端尊重 presence、安静时间、用户偏好与 cool-down。

**Phase 完成定义**：

- iOS / Android / Email 三条通道实现 + 灰度开关
- `PushFanoutJob` 通过订阅 outbox 后产生的 durable 事件，决定是否 push
- 提供 `/v1/notifications/register|unregister|preferences` API
- DurableEvent 与 push 决策路径有完整 trace 关联（trace_id 从 ConversationMessageAppended 跨进程传递到 APNs 请求）

**Phase 输出依赖**：P1 outbox、P3 ConversationService / ApprovalService、P5 JobSupervisor。

### P6.S0 — Schema + 配置基线（0.5 天）

> `push_tokens` 表已经在 P0.S1 baseline 中创建。本节用于补足 user preferences。

**新增 migration（postgres，0003_notifications.sql）**：

```sql
CREATE TABLE notification_preferences (
    account_id              TEXT PRIMARY KEY REFERENCES accounts(account_id) ON DELETE CASCADE,
    direct_message_enabled  BOOLEAN NOT NULL DEFAULT TRUE,
    group_mention_enabled   BOOLEAN NOT NULL DEFAULT TRUE,
    approval_required_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    agent_session_ended_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    quiet_hours_start_minute SMALLINT,    -- 0..1440 of local day
    quiet_hours_end_minute   SMALLINT,
    quiet_hours_timezone     TEXT,        -- IANA tz, e.g. 'Asia/Shanghai'
    updated_at_ms            BIGINT NOT NULL
);

CREATE TABLE notification_cooldowns (
    account_id         TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    cooldown_key       TEXT NOT NULL,        -- e.g. 'conv:<id>', 'approval:<id>'
    last_sent_at_ms    BIGINT NOT NULL,
    PRIMARY KEY (account_id, cooldown_key)
);
CREATE INDEX idx_notif_cooldowns_last_sent ON notification_cooldowns(last_sent_at_ms);
```

**配置位**：

| ENV | 默认 | 说明 |
|---|---|---|
| `MINOS_PUSH_APNS_KEY_PATH` | — | `.p8` 私钥路径 |
| `MINOS_PUSH_APNS_KEY_ID` | — | Key ID |
| `MINOS_PUSH_APNS_TEAM_ID` | — | Team ID |
| `MINOS_PUSH_APNS_TOPIC` | `dev.minos.app` | bundle id |
| `MINOS_PUSH_APNS_SANDBOX` | `false` | true 走 sandbox endpoint |
| `MINOS_PUSH_FCM_PROJECT_ID` | — | GCP project |
| `MINOS_PUSH_FCM_SERVICE_ACCOUNT_JSON` | — | service-account JSON 路径 |
| `MINOS_PUSH_SMTP_URL` | — | `smtps://user:pass@host:port` |
| `MINOS_PUSH_SMTP_FROM` | — | `Minos <noreply@...>` |
| `MINOS_PUSH_COOLDOWN_DEFAULT_MS` | `15000` | 默认 cool-down |
| `MINOS_PUSH_BATCH_SIZE` | `64` | PushFanoutJob 单 tick 批量 |

**测试**：

- migration smoke：表存在、默认值对
- preferences round-trip via service trait（`NotificationService::get_or_default(account_id)` 回退到默认值）

**工作量**：1 单位（0.5 天）

### P6.S1 — Push token 注册 API + Service 骨架（1 天）

**新增文件**：

- `src/notifications/mod.rs`
- `src/notifications/use_case.rs`
- `src/notifications/preferences.rs`
- `src/store/postgres/push_tokens.rs`、`notification_preferences.rs`、`notification_cooldowns.rs`
- `src/http/v1/notifications.rs`

**Trait**：

```rust
#[async_trait]
pub trait NotificationService: Send + Sync {
    async fn register_token(&self, input: RegisterTokenInput) -> Result<(), NotificationError>;
    async fn unregister_token(&self, input: UnregisterTokenInput) -> Result<(), NotificationError>;
    async fn list_tokens(&self, account_id: &str) -> Result<Vec<PushTokenDto>, NotificationError>;
    async fn get_preferences(&self, account_id: &str) -> Result<NotificationPreferences, NotificationError>;
    async fn update_preferences(&self, input: UpdatePreferencesInput) -> Result<NotificationPreferences, NotificationError>;
    async fn dispatch_for_event(&self, event: &DurableEventEnvelope) -> Result<DispatchOutcome, NotificationError>;
}

pub struct RegisterTokenInput {
    pub account_id: String,
    pub installation_id: String,
    pub kind: PushKind,           // Apns | Fcm
    pub token: String,
    pub locale: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error("validation_format: {0}")]   ValidationFormat(&'static str),
    #[error("notification_token_invalid")] TokenInvalid,
    #[error("internal: {0}")] Internal(#[from] AppError),
}
```

**HTTP**：

- `POST /v1/notifications/tokens/register`
- `POST /v1/notifications/tokens/unregister`
- `POST /v1/notifications/preferences/get`
- `POST /v1/notifications/preferences/update`

**Token 验证**：

- APNs token 必须为 64 hex chars
- FCM token 长度 < 4096
- 同 token 已存在 → upsert（last_used_at_ms 刷新）

**测试**：

- token register / unregister roundtrip
- preferences get_or_default
- 非法 token format 返回 `notification_token_invalid`

**工作量**：2 单位（1 天）

### P6.S2 — APNs / FCM / SMTP 通道实现（2 天）

**新增文件**：

- `src/notifications/channels/mod.rs`
- `src/notifications/channels/apns.rs`
- `src/notifications/channels/fcm.rs`
- `src/notifications/channels/smtp.rs`
- `src/notifications/channels/composite.rs`

**Channel trait**：

```rust
#[async_trait]
pub trait PushChannel: Send + Sync {
    fn kind(&self) -> PushKind;
    async fn send(&self, attempt: PushAttempt) -> Result<PushSendOutcome, PushSendError>;
}

pub struct PushAttempt {
    pub token: PushTokenDto,
    pub payload: PushPayload,
    pub deduplication_key: String,        // 用作 APNs apns-collapse-id / FCM collapse_key
}

#[derive(Debug)]
pub enum PushSendOutcome {
    Sent,
    TokenExpired,         // 结果由 channel 解析；caller 据此 mark token revoked
    RateLimited { retry_after_ms: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum PushSendError {
    #[error("transient: {0}")] Transient(String),
    #[error("permanent: {0}")] Permanent(String),
}
```

**APNs**：

- 用 `a2 = "0.10"` 或 `apns2`（PR 内一次决策；本计划默认 `a2`）
- token-based auth：`.p8` + key id + team id；client 复用 connection pool
- `apns-priority: 10`，`apns-push-type: alert`
- 解析响应错误码：`BadDeviceToken / Unregistered` → `TokenExpired`；`TooManyRequests` → `RateLimited`
- 配置 sandbox / production endpoint 切换

**FCM**：

- 用 `fcm = "0.10"` HTTP v1
- service account JWT 自动续期
- `mutable_content`、`apns_config.payload.aps.mutable-content` 等配置封装

**SMTP**：

- 用 `lettre`，rustls 后端
- 仅用于 audit alert / approval timeout 等管理员通知
- HTML + plain 双模板

**Composite**：

- 给定一组 token + payload → 串行调用每个 channel；single-token 失败不影响其他 token
- 故障合并：channel 抛 Permanent → 返回 `TokenExpired`，由 caller 撤销 token；Transient → caller 重试

**测试**：

- `apns_token_expired_marks_revoked.rs`：mock APNs 返回 BadDeviceToken → token 表 revoked_at_ms 写入
- `apns_send_basic.rs`：mock server，断言 header / payload 与 spec 一致
- `fcm_send_basic.rs`：mock，断言 v1 接口 payload
- `smtp_send_basic.rs`：lettre stub transport，断言邮件正文
- `composite_partial_failure.rs`：APNs 成功 + FCM 失败 → caller 收到混合结果

**Metrics**：

```
push_send_total{channel, result}        Counter
push_send_duration_seconds{channel}     Histogram
push_token_revoked_total{channel}       Counter
```

**工作量**：4 单位（2 天）

### P6.S3 — `PushFanoutJob` + decision 引擎（2 天）

**新增文件**：`src/jobs/push_fanout.rs`、`src/notifications/decision.rs`

**Job 形态**：

- 订阅一组 outbox 触发 channel；本计划走 PG NOTIFY/LISTEN：
  - `OutboxRepository::ack` 时如果 event 涉及 push 类型 topic（`account:*`），`pg_notify('minos.push', event_id)`
  - PushFanoutJob 在 `tick` 中：
    - try `LISTEN minos.push`，take notifications；为空时直接扫 `outbox_events` recent acked 行作为兜底（防止 NOTIFY 丢失）
- 单 batch 默认 64

**Decision 引擎**：

```rust
pub fn decide(event: &DurableEvent, prefs: &NotificationPreferences, now_ms: i64, presence: &PresenceSnapshot) -> Decision {
    match event {
        DurableEvent::ConversationMessageAppended { conversation_id, message_id, sender, .. } => {
            // 1. 仅对目标 account_id 通知；这里 fanout 调用方已经按 topic=account:<id> 抽出 envelope
            // 2. 排除自己：sender 是当前 account 直接 skip
            // 3. presence: 任意 installation 在线 + 最近 60s pong → skip
            // 4. preferences:
            //    - 是 direct conv → direct_message_enabled
            //    - 是 group conv 且被 mentioned → group_mention_enabled
            //    - 否则 skip
            // 5. quiet hours
            // 6. cooldown_key = "conv:<conversation_id>"，按 default_ms 限频
            ...
        }
        DurableEvent::ApprovalRequested { request_id, session_id, .. } => {
            // approval 优先级高，cool-down 5s
            ...
        }
        DurableEvent::AgentSessionEnded { .. } => {
            // 仅 enabled 时，cool-down 60s
            ...
        }
        _ => Decision::Skip,
    }
}

pub enum Decision {
    Skip { reason: SkipReason },
    Send { channels: Vec<PushKind>, payload: PushPayload, cooldown_key: String, cooldown_ms: u64 },
}

pub enum SkipReason { SelfSender, Online, PreferenceOff, QuietHours, Cooldown, Unsupported }
```

**Quiet hours 算法**：

- 把 `now_ms` 转 user 的 `quiet_hours_timezone`，得到当日分钟数 `m`
- 处理 wrap-around（end < start 表示跨午夜）

**Cool-down**：

- 在事务内 INSERT … ON CONFLICT (account_id, cooldown_key) DO UPDATE SET last_sent_at_ms = ... WHERE excluded.last_sent_at_ms - cooldowns.last_sent_at_ms >= cooldown_ms RETURNING xmax = 0
- 若 RETURNING xmax = 0 表示新插入或更新成功 → 准许；否则 `Skip { Cooldown }`

**`tick` 算法**：

```rust
async fn tick(&self, ctx: &JobContext) -> Result<JobOutcome, JobError> {
    let envelopes = self.subscriber.poll(BATCH).await
        .map_err(|e| JobError::Transient(e.to_string()))?;
    if envelopes.is_empty() { return Ok(JobOutcome::Idle); }

    let mut sent = 0u32;
    for env in envelopes {
        let target_account = match env.topic_kind() {
            TopicKind::Account => env.partition_key().to_string(),
            _ => continue,
        };
        let prefs = self.notifications.get_preferences(&target_account).await?;
        let presence = self.presence.snapshot(&target_account).await?;
        let decision = decide(&env.payload, &prefs, ctx.clock.now_ms(), &presence);
        match decision {
            Decision::Skip { reason } => {
                metrics::PUSH_DECISION_SKIPPED.with_label_values(&[reason.as_str()]).inc();
                continue;
            }
            Decision::Send { channels, payload, cooldown_key, cooldown_ms } => {
                if !self.cooldown_check(&target_account, &cooldown_key, cooldown_ms, ctx.clock.now_ms()).await? {
                    metrics::PUSH_DECISION_SKIPPED.with_label_values(&["cooldown"]).inc();
                    continue;
                }
                let tokens = self.notifications.list_tokens(&target_account).await?;
                for token in tokens.into_iter().filter(|t| channels.contains(&t.kind)) {
                    let outcome = self.composite.send(PushAttempt {
                        token, payload: payload.clone(),
                        deduplication_key: env.event_id.clone(),
                    }).await;
                    match outcome {
                        Ok(PushSendOutcome::Sent) => sent += 1,
                        Ok(PushSendOutcome::TokenExpired) => self.notifications.revoke_token(&...).await.ok(),
                        Ok(PushSendOutcome::RateLimited { .. }) => continue,
                        Err(e) => tracing::warn!(?e, "push send failed"),
                    }
                }
            }
        }
    }
    Ok(JobOutcome::DidWork(sent))
}
```

**Metrics**：

```
push_decision_total{decision="send|skip", reason}     Counter
push_send_dispatched_total{channel}                   Counter
push_cooldown_block_total                             Counter
push_pipeline_lag_seconds                             Histogram
```

**测试矩阵**：

| 测试 | 场景 | 断言 |
|---|---|---|
| `push_send_offline_basic.rs` | 用户离线 + 直聊消息 | APNs send 调用 1 次 |
| `push_skip_when_online.rs` | 用户 presence 在线 | Skip Online；无 send |
| `push_skip_self_sender.rs` | 自己发的消息 | Skip SelfSender |
| `push_quiet_hours.rs` | 当前时间在 quiet 窗口 | Skip QuietHours |
| `push_cooldown.rs` | 同 conversation 1s 内两条 | 第二条 Skip Cooldown |
| `push_mention_only_in_group.rs` | group + 未被提及 | Skip PreferenceOff |
| `push_approval_high_prio.rs` | ApprovalRequested + cool-down 5s | 第一条 send，第二条 5s 内 skip |
| `push_token_expired.rs` | APNs 返回 BadDeviceToken | token revoked + skip |
| `push_pipeline_smoke.rs` | end-to-end 从 send_message → DurableEvent → outbox → push | mock APNs 收到 1 条 |

**工作量**：4 单位（2 天）

### P6.S4 — 真机灰度 + 可观测性（0.5 天）

**改动**：

- `apps/mobile/lib/infrastructure/push.dart`（Flutter）：调用 `/v1/notifications/tokens/register`；提供 dev toggle
- `apps/mobile/ios/Runner/AppDelegate.swift`：APNs token 注册
- 文档：`docs/ops/push-rollout.md`：灰度策略（per-account flag、percentile rollout）

**验收**：

- iOS 真机收到一条直聊推送
- Android 真机收到 FCM 推送
- 关闭 quiet hours / 关闭 group_mention 后，按预期不收

**工作量**：1 单位（0.5 天）

### P6 完成定义

- [ ] 注册 / 注销 / 偏好 API 上线
- [ ] APNs / FCM / SMTP 三 channel 可用
- [ ] PushFanoutJob 在 worker plane 中运行；presence、quiet hours、cool-down、自己排除全部生效
- [ ] 真机灰度跑过端到端

### P6 回退总结

- 若 APNs 突发 429：把 `MINOS_PUSH_BATCH_SIZE` 降到 16；引入 token bucket per channel
- 若 PG NOTIFY/LISTEN 在多 worker 实例下分配不均：退化为 polling-only 模式（`MINOS_PUSH_USE_NOTIFY=false`）

---

## P7 — Observability（OTel + Prom + 日志）

**Phase 目标**：把零散的 `tracing` 调用、`OnceLock` Counter 升级为 OpenTelemetry 标准管线；保证 trace 跨 HTTP / WS / worker / 外部 push 的端到端可视化；Metrics 与日志互相 join 不再依赖人工。

**Phase 完成定义**：

- OTel traces 通过 OTLP/gRPC 导出到 Tempo 兼容 collector，trace_id 覆盖 §0.6 列出的所有 span
- Prometheus `/metrics` 暴露架构 overview §3.8 列出的全部 metric 名称
- 日志为结构化 JSON，含 `trace_id` / `span_id` / `request_id` / `account_id?` / `installation_id?` / `session_id?`
- CI 增加 metric / span 名册 drift gate

**Phase 输出依赖**：P0–P6 中的 service / job / gateway 已具备明确边界。

### P7.S1 — `telemetry` 模块重构（1 天）

**改动文件**：

- `src/telemetry/mod.rs`：拆分为 `tracing.rs` / `metrics.rs` / `logs.rs`
- `Cargo.toml`：引入 OTel 依赖（已在 §0.3 列出）
- `src/main.rs`：`telemetry::init(&cfg)` 统一入口

**初始化流程**：

```rust
pub fn init(cfg: &TelemetryConfig) -> TelemetryGuard {
    let resource = Resource::new(vec![
        KeyValue::new("service.name", "minos-backend"),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        KeyValue::new("deployment.environment", cfg.environment.as_str()),
        KeyValue::new("minos.runtime_mode", cfg.runtime_mode.as_str()),
        KeyValue::new("minos.instance_id", cfg.instance_id.clone()),
    ]);

    // tracer
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(&cfg.otlp_endpoint))
        .with_trace_config(trace::config().with_sampler(Sampler::TraceIdRatioBased(cfg.sample_ratio)).with_resource(resource.clone()))
        .install_batch(opentelemetry::runtime::Tokio).expect("otlp tracer");

    // metrics: 单独导出器，prometheus 暴露
    let registry = prometheus::Registry::new();
    register_static_metrics(&registry);

    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cfg.log_level)))
        .with(tracing_subscriber::fmt::layer().json().with_current_span(true).with_span_list(false))
        .with(tracing_opentelemetry::layer().with_tracer(tracer));
    tracing::subscriber::set_global_default(subscriber).expect("tracing subscriber");

    TelemetryGuard { registry }
}

pub struct TelemetryGuard { pub registry: prometheus::Registry }
```

**`/metrics` endpoint** 改为从 `TelemetryGuard.registry.gather()` 收集，避免使用全局 `lazy_static`。

**测试**：

- `telemetry_init.rs`：mock OTLP collector（grpc tonic stub），断言启动 + tracing 输出能被收集
- `telemetry_metrics_endpoint.rs`：HTTP 拉 `/metrics`，断言基础指标存在

**工作量**：2 单位（1 天）

### P7.S2 — Metric 名册 + drift gate（1 天）

**新增文件**：

- `crates/minos-backend/metrics_registry.toml`：metrics 名册（一行一个 metric，含 type、help、labels）
- `xtask/src/lints/metrics.rs`：lint 把代码中所有 `register_*` 调用与名册比对

**名册示例（节选）**：

```toml
[[metric]]
name = "http_requests_total"
type = "counter"
help = "Total HTTP requests by route, method, status"
labels = ["route", "method", "status"]

[[metric]]
name = "http_request_duration_seconds"
type = "histogram"
help = "HTTP request duration in seconds"
labels = ["route", "method"]

[[metric]]
name = "ws_connections"
type = "gauge"
help = "Number of live WebSocket connections by role"
labels = ["role"]

[[metric]]
name = "outbox_dispatch_total"
type = "counter"
help = "Outbox dispatcher results"
labels = ["result"]
# ... 等等，覆盖 P0–P6 落地的全部 metric
```

**Lint 行为**：

- 扫描代码中 `Counter::with_label_values` / `register_int_counter_vec!` 等调用 → 提取 metric name + labels
- 与 `metrics_registry.toml` 比对：差集为 0；不一致（label 多/少 / 名字写错）→ CI red

**测试**：

- 故意在代码里加一个未注册的 metric → `cargo xtask lint-metrics` 退非 0
- 故意在名册里加一个未实现的 metric → 同上

**工作量**：2 单位（1 天）

### P7.S3 — Span 名册 + 跨进程 trace（1 天）

**Span 命名约定**：

| Span name | 触发点 | 属性 |
|---|---|---|
| `http.request` | 每个 HTTP 请求 | `http.method`, `http.route`, `http.status_code`, `account_id?`, `installation_id?` |
| `ws.session` | 每个 WS 连接生命周期 | `ws.role`, `ws.installation_id`, `ws.principal`, `ws.conn_id` |
| `ws.frame.in` / `ws.frame.out` | 每个 WS 帧 | `ws.frame_type`, `topic?` |
| `service.<name>.<method>` | 各 service 入口 | `service.method`, 业务 id |
| `agent_session.turn` | 每个 turn | `session_id`, `turn_id`, `turn_seq` |
| `host_command.dispatch` | 每个 host_command 出口 | `command_id`, `host_installation_id`, `method` |
| `approval.request` | approval 生命周期 | `request_id`, `session_id` |
| `outbox.dispatch` | 每条 outbox 投递 | `event_id`, `topic_kind`, `topic` |
| `worker.tick` | 每个 job tick | `job` |
| `push.send` | 每条 push 出口 | `channel`, `account_id` |

**跨进程 trace 传递**：

- HTTP：tower middleware 注入 W3C `traceparent` 头；客户端 SDK 在请求时携带
- WS：在 `Hello` 帧中带 `traceparent` 字段（仅 server → client 单向；client 后续帧带 `traceparent` 字段表示该帧的祖先）
- 跨 worker：`durable_event_log.payload_json` 中含 `_trace_context` 字段（W3C 编码），dispatcher 在恢复 span 时使用 `Context::from_remote(...)`
- 跨外部（APNs / FCM）：作为 attribute；HTTP 不带 traceparent（避免泄露给第三方）

**实现**：

- `src/telemetry/propagation.rs`：W3C inject / extract helpers
- `DurableEvent` 结构追加 `_trace`（默认 None；在 service 写入时填充）

**测试**：

- `e2e_trace.rs`：发起 HTTP `/v1/conversations/send-message` → trace 应贯穿 HTTP → ConversationService → DurableEvent → OutboxDispatcher → GatewayPush → WS frame
- 断言 trace 链 ≥ 4 跳（HTTP / service / outbox / gateway）

**Sampling**：

- `MINOS_TRACE_SAMPLE_RATIO`：默认 dev=1.0，prod=0.05
- 错误路径 always_on：errors 在 service 层抛出时强制 `record_error` + 标 sampling 优先

**工作量**：2 单位（1 天）

### P7.S4 — 日志结构化 + correlation（0.5 天）

**改动**：

- 全 crate 启用 JSON formatter
- 自定义 `tracing_subscriber::Layer`：把 `trace_id` / `span_id` 注入每条日志
- 引入 `tracing::Span::current().context()` 提取 trace_id

**字段约定**：

```json
{
  "ts": 1760000000000,
  "level": "INFO",
  "target": "minos_backend::agent_sessions",
  "msg": "agent session started",
  "trace_id": "0af7651916cd43dd8448eb211c80319c",
  "span_id": "b7ad6b7169203331",
  "request_id": "req_01J...",
  "account_id": "acct_01J...",
  "installation_id": "inst_01J...",
  "session_id": "sess_01J...",
  "fields": { "agent_id": "agent_codex", "host_installation_id": "inst_..." }
}
```

**测试**：

- 用 `tracing_test` 捕获输出 → 断言 trace_id 字段存在且为 32 hex chars

**工作量**：1 单位（0.5 天）

### P7.S5 — Dashboards + ops 文档（0.5 天）

**新增**：

- `docs/ops/dashboards/grafana-overview.json`
- `docs/ops/dashboards/grafana-realtime.json`
- `docs/ops/dashboards/grafana-workers.json`
- `docs/ops/dashboards/grafana-push.json`
- `docs/ops/observability.md`：每个 dashboard 的 panel 含义、告警阈值（如 `outbox_in_flight > 1000` 触发 warn）

**告警建议**：

| 名称 | 条件 | 阈值 | 严重度 |
|---|---|---|---|
| OutboxLagHigh | `histogram_quantile(0.99, outbox_dispatch_lag_seconds) > 30` | 5 min | warn |
| WorkerJobStuck | `job_last_success_age_seconds > 300` | 任一 job | crit |
| WsForceCloseSpike | `rate(ws_close_total{reason="superseded"}[5m]) > 50` | 持续 10m | warn |
| ApprovalTimeoutSpike | `rate(approval_timeout_resolved_total[5m]) > 5` | 持续 15m | warn |
| PushSendFailureRate | `rate(push_send_total{result="error"}[5m]) / rate(push_send_total[5m]) > 0.1` | 持续 10m | warn |
| HttpErrorRateHigh | `rate(http_requests_total{status=~"5.."}[5m]) > 5` | 持续 5m | crit |
| HostCommandTimeoutSpike | `rate(host_command_timeout_total[5m]) > 1` | 持续 15m | warn |

**验收**：

- Grafana 导入 4 份 dashboard，所有 panel 都有数据（dev 环境跑过端到端）
- ADR 中如有"observability stack 选型"决策，新建 `docs/adr/0021-...md`

**工作量**：1 单位（0.5 天）

### P7 完成定义

- [ ] OTel traces 在本地 docker-compose 跑通端到端可视化（HTTP → service → outbox → gateway）
- [ ] Prom `/metrics` 与名册 100% 对齐；CI lint 通过
- [ ] 全部日志为 JSON + trace_id 注入
- [ ] Grafana dashboards / 告警建议提交

### P7 回退总结

- 若 OTLP 在 prod 高 QPS 下出现导出延迟：把 `Sampler` 切到 `TraceIdRatioBased(0.01)` + `parent_based(always_off)`
- 若 metrics drift gate 误报：先把 lint 设为 warning，开 issue 跟踪规则修复
- 若 W3C trace 在 WS 帧中扩大 payload：仅在 `Hello`、`DurableEvent` 帧中带；其余 ephemeral 帧不带
