# D06 · Ops, Config & Cleanup

| Field | Value |
|-------|--------|
| Domain ID | D06 |
| Status | Refined (2026-07-31) |
| L0 | Ops notes in long-term spec + [vps-deploy.md](../../../ops/vps-deploy.md) |
| Tasks | `T-ops-*`, `T-schema-*`, `T-cleanup-*` in [tasks/TASKS.md](tasks/TASKS.md) |
| Depends on | Product path (D01–D05) largely working |
| Blocks | Nothing critical for first E2E; improves reliability |

---

## 1. Goal

在 identity/UI/projection 路径工作后**加固**生产和开发者体验：

- Secrets 和 Supabase 项目配置
- Image publish 策略
- Config origin polish
- **Schema 对齐**（SQLite dev → Postgres prod 形态）
- **旧配对代码移除**（清理工作）
- 文档扫描；移除 QR-primary 描述
- 可选：禁用公开 password 注册

---

## 2. Decisions (locked)

| # | Decision |
|---|----------|
| 1 | VPS 保持 runtime-only（不 clone monorepo） |
| 2 | Prod pin immutable image tags（`sha-…` 或 `backend-v*`） |
| 3 | `minos-backend` 公开 package OK（开源）；secrets 永远不在 image 中 |
| 4 | Config scheme 统一是 **polish**，不阻塞 D01–D05 |
| 5 | SQLite dev schema 对齐到 Postgres 形态（`devices` → `device_installations` 等） |

---

## 3. Workstreams

### 3.1 Secrets（`T-ops-01`）

| Secret | 位置 | 备注 |
|--------|------|------|
| `SUPABASE_URL` | Server env（docker-compose） | `https://<project>.supabase.co` |
| `SUPABASE_JWT_AUD` | Server env | Project ref |
| `MINOS_JWT_SECRET` | Server env | 不变 |
| `SUPABASE_ANON_KEY` | Client build env（`.env.local` / CI） | Public key |
| Supabase service role key | **Supabase Dashboard only** | 永远不在 Minos 代码中 |

### 3.2 Schema 对齐（`T-schema-*`）

SQLite dev migration 需要对齐 Postgres 形态：

| SQLite 旧 | Postgres 新（目标） | 动作 |
|-----------|---------------------|------|
| `devices` | `device_installations` | 重命名 + 列变更 |
| `devices.secret_hash` | （移除） | device-secret 轨道移除 |
| `devices.role TEXT` | `device_installations.kind installation_kind` | enum 化（`mobile`/`browser`/`desktop`/`host`） |
| `account_host_pairings` | `host_links` | 重命名 + 加 `acl_json`/`link_display_name` |
| `pairing_codes` | （移除） | 删除 |
| `installation_kind_account_consistency` | 新增 CHECK | 同 Postgres |

Rust store 层（`store::devices`、`store::account_host_pairings`）迁移到新表名。

### 3.3 旧代码移除（`T-cleanup-*`）

| 组件 | 动作 |
|------|------|
| `crates/minos-pairing/` | **删除 crate** |
| `crates/minos-backend/src/pairing/` | **删除模块** |
| `crates/minos-backend/src/http/v1/pairing.rs` | **删除** |
| `crates/minos-backend/src/http/auth.rs::authenticate()` | **删除**（device-secret 轨道） |
| `POST /v1/host/pairing/request-code` | 从 ROUTE_INVENTORY 移除 |
| `POST /v1/pairing/confirm` | 从 ROUTE_INVENTORY 移除 |
| `POST /v1/host/pairing/redeem` | 从 ROUTE_INVENTORY 移除 |
| `POST /v1/pairing/revoke` | 从 ROUTE_INVENTORY 移除（由 `/v1/hosts/unlink` 替代） |
| `POST /v1/pairing/list-hosts` | 从 ROUTE_INVENTORY 移除（由 `GET /v1/hosts` 替代） |
| `DELETE /v1/pairings/:host_device_id` | 从 ROUTE_INVENTORY 移除 |
| `crates/minos-daemon/src/relay_pairing.rs` | **删除** |
| `apps/mobile/lib/features/pairing/` | **删除** |
| `apps/macos` QR 渲染 | **移除** |
| `pairing_codes` table（SQLite + Postgres migration） | **移除** |

### 3.4 Images（`T-ops-02`）

- Tag-triggered 或 workflow_dispatch publish
- `vps-deploy.md` 对齐 pin tag 策略

### 3.5 Config（`T-ops-04`）

- 单一 origin env story（`https://minos.ainexc.com` SSOT）
- Client scheme mapping（`https` → `wss` 自动推导）

### 3.6 Docs（`T-ops-05`）

- `architecture-*.md`、`business-flow` 更新到匹配 program
- 移除 QR-primary 描述

### 3.7 Auth freeze（`T-ops-06`）

- OIDC 稳定后可选禁用 password 注册

---

## 4. Exit criteria

- [ ] Runbook：轮换 Minos JWT + Supabase config
- [ ] Deploy 文档匹配实际 host link + exchange
- [ ] Image update 路径文档化（不依赖 "always build on merge"）
- [ ] SQLite dev schema 对齐 Postgres
- [ ] 旧配对代码全部移除（grep 无残留引用）
- [ ] QR 文档标记为已移除（如还有引用）

---

## 5. Task slice

`T-ops-01` … `T-ops-07` + `T-schema-01..02` + `T-cleanup-01..06` in [tasks/TASKS.md](tasks/TASKS.md).
