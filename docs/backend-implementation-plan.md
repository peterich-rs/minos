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
