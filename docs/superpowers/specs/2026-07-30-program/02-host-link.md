# D02 · Host Link (Same-Account Binding)

| Field | Value |
|-------|--------|
| Domain ID | D02 |
| Status | Implementation-ready (refined 2026-07-31) |
| L0 | [long-term spec §4.4](../2026-07-30-cloud-identity-clients-long-term.md) |
| Tasks | `T-host-*`, `T-cleanup-*` in [tasks/TASKS.md](tasks/TASKS.md) |
| Depends on | **D01** account session (Minos bearer for the human) |
| Blocks | D05 projection completeness for remote viewers |

---

## 1. Goal

用 **same-account host link** 替代 QR-primary：

> 用户在 Desktop 上登录同一 Minos 账户 → **Link this Mac**（daemon 证明机器控制权）→ account↔host link → `/ws/host` 上线 → Mobile/Web 可以路由到该 host。

**术语澄清**：QR 移除后，"pairing"（配对）一词专指 **host link** 操作（account ↔ host 关联）。`host_links` 表的主键 `pair_id` 是该关联的标识符。旧的 QR-based `pairing/` 模块和 `pairing_codes` 表将被删除。

**旧 QR 配对代码彻底移除**（latest-only）：
- Backend：`/v1/host/pairing/request-code`、`/v1/pairing/confirm`、`/v1/host/pairing/redeem`、`pairing/` 模块
- `minos-pairing` crate（v1 P2P-era）
- `authenticate()` device-secret 轨道

---

## 2. Decisions (locked)

| # | Decision |
|---|----------|
| 1 | Human login ≠ host hijack：linking 需要 **daemon-local proof**（Ed25519 签名） |
| 2 | Pairs 保持 **account ↔ host installation**（ADR 0020），复用现有 Postgres `host_links` 表 |
| 3 | 每个 account 允许多个 host（multi-Mac） |
| 4 | Unlink 是一等操作（撤销路由 + 撤销 host installation tokens） |
| 5 | UI 必须清楚显示 **Local only / Linked / Error** |
| 6 | 保留现有 host bootstrap crypto（Ed25519 nonce + TOFU public key），只替换上层的"绑定到 account"步骤 |

---

## 3. Identities (do not collapse)

| Identity | Proof | Purpose |
|----------|-------|---------|
| Human account | Minos access JWT（来自 D01 exchange） | Authorization subject |
| Host installation | Ed25519 keypair + backend-issued nonce | 证明机器控制权 |
| Client device | `X-Device-Id` + `installation_id` on device_installations | Refresh revoke / audit |

---

## 4. Architecture: 复用现有，替换入口

### 4.1 保留的现有组件

这些组件**不变**，因为它们已经设计良好且与 account 无关：

| 组件 | 位置 | 作用 |
|------|------|------|
| `BootstrapNonceStore` | `auth/host_bootstrap.rs:28-65` | Ed25519 nonce 签发（需迁 Redis，见 §7） |
| `verify_and_register` | `auth/host_bootstrap.rs:86-142` | TOFU public key 验证 + device_installations 行管理 |
| `HostInstallationPrincipal` | `auth/host_installation.rs` | host installation token (`hit_*`) 提取 |
| `issue_host_ws_ticket` | `auth/use_case.rs:599` | host WS ticket 签发 |
| `/v1/host/bootstrap/nonce` | `http/v1/host.rs` | 获取 nonce（不变） |
| `/v1/host/realtime/ws-ticket` | `http/v1/host.rs` | host WS ticket（不变） |
| `/ws/host` gateway | `realtime/gateway.rs` | host WS 连接（不变） |
| `host_links` table | Postgres migration | account↔host 关联（已存在！） |
| `host_installation_tokens` table | Postgres migration | host opaque token（已存在！） |

### 4.2 移除的组件

| 组件 | 替代品 |
|------|--------|
| `/v1/host/pairing/request-code` | 不需要——nonce + Ed25519 proof 直接在 link endpoint 验证 |
| `/v1/pairing/confirm` | `POST /v1/hosts/link`（account bearer + host proof） |
| `/v1/host/pairing/redeem` | link 成功后直接签发 host installation token |
| `pairing/` module（pairing_codes 表等） | `host_links` 表 + `host_installation_tokens` 表 |
| `authenticate()` device-secret 轨道 | D01 exchange（account bearer）+ host bootstrap（Ed25519） |
| `minos-pairing` crate | `minos-protocol` 已有 `PairingQrPayload` 类型（如还需要 QR 辅助） |

### 4.3 新增的组件

| 组件 | 位置 | 作用 |
|------|------|------|
| `POST /v1/hosts/link` | `http/v1/hosts.rs`（新文件） | account bearer + host Ed25519 proof → upsert host_link + 签发 host installation token |
| `POST /v1/hosts/unlink` | `http/v1/hosts.rs` | account bearer → 撤销 host_link + 撤销 tokens |
| `GET /v1/hosts` | `http/v1/hosts.rs` | account bearer → 列出 account 的所有 host |
| `AuthUseCase::link_host` | `auth/use_case.rs` | 编排 link 逻辑 |
| `AuthUseCase::unlink_host` | `auth/use_case.rs` | 编排 unlink 逻辑 |

---

## 5. API contract

### `POST /v1/hosts/link`

**Auth**: account bearer（`Authorization: Bearer <minos-jwt>`）+ host bootstrap proof（body 中）

**Request**

```json
{
  "installation_id": "<host uuid>",
  "nonce": "nonce_<43 chars>",
  "public_key": "ed25519:<base64url>",
  "signature": "ed25519-sig:<base64url>",
  "host_display_name": "My MacBook Pro"
}
```

**Server steps**

1. **Account bearer 验证**：`bearer::require(&state, &headers)` → `AccountPrincipal { account_id }`
2. **Host bootstrap proof 验证**：`host_bootstrap::verify_and_register(...)`（复用现有逻辑）
   - 消耗 nonce（single-use）
   - 验证 Ed25519 签名 over `"{installation_id}:{nonce}:v1/hosts/link"`
   - TOFU public key（首次注册或验证现有 key）
   - 确保 `device_installations` 行（`kind=host`、`public_key` 设置、`account_id` 为 NULL——CHECK 约束要求）
3. **Upsert host_link**：
   ```sql
   INSERT INTO host_links (pair_id, account_id, host_installation_id, linked_via_installation_id, link_display_name, paired_at_ms)
   VALUES ($1, $account_id, $installation_id, $caller_device_id, $host_display_name, $now)
   ON CONFLICT (account_id, host_installation_id) DO UPDATE SET paired_at_ms = $now
   ```
   - `linked_via_installation_id` = caller 的 device_id（Desktop 的 installation_id，`kind=desktop`）
4. **签发 host installation token**：
   ```sql
   INSERT INTO host_installation_tokens (token_hash, host_installation_id, issued_at_ms)
   VALUES (sha256($plaintext_token), $installation_id, $now)
   ```
   返回明文 token `hit_<43 chars>`（一次性返回，不存储明文）。
5. **Return**：

```json
{
  "host_installation_id": "...",
  "host_installation_token": "hit_<43 chars>",
  "link": {
    "pair_id": "...",
    "account_id": "...",
    "host_display_name": "My MacBook Pro",
    "linked_at_ms": 1234567890
  }
}
```

**Response（错误）**

| Case | HTTP | code |
|------|------|------|
| 无 account bearer | 401 | `unauthorized` |
| nonce 无效/过期/已用 | 401 | `bootstrap_nonce_invalid` |
| Ed25519 签名验证失败 | 401 | `proof_invalid` |
| public_key 不匹配 | 401 | `public_key_mismatch` |
| host 已 link 到其他 account | 409 | `host_linked_elsewhere` |

### `POST /v1/hosts/unlink`

**Auth**: account bearer

**Request**

```json
{
  "host_installation_id": "<host uuid>"
}
```

**Server steps**

1. Account bearer 验证
2. `DELETE FROM host_links WHERE account_id = $account_id AND host_installation_id = $host_id`
3. `host_installation_tokens::revoke_all_for_host(host_id)` — 撤销所有 host tokens
4. Kill live WS session（`registry.remove(host_id).revoke(AuthRevoked)`）
5. `ingest::invalidate_peer_targets_for_account(account_id)` — 清理 fan-out 缓存
6. Return 204

### `GET /v1/hosts`

**Auth**: account bearer

**Response**

```json
{
  "hosts": [
    {
      "host_installation_id": "...",
      "host_display_name": "My MacBook Pro",
      "linked_at_ms": 1234567890,
      "online": true
    }
  ]
}
```

`online` 从 WS connection registry 派生（host 是否有活跃 `/ws/host` 连接）。

---

## 6. Target flow（Desktop ↔ Mobile 联通）

```text
Desktop 端：
  1. 用户在 Desktop 完成 D01 exchange（系统浏览器 OAuth → 深链接回调 → Minos session）
  2. Desktop 调 daemon：获取 host installation identity（Ed25519 keypair + installation_id）
  3. Desktop 调 backend：POST /v1/host/bootstrap/nonce → 获得 nonce
  4. Desktop 调 daemon：用 Ed25519 private key 签名 "{installation_id}:{nonce}:v1/hosts/link"
  5. Desktop 调 backend：POST /v1/hosts/link（account bearer + host proof + signature）
  6. Backend upsert host_link + 签发 host_installation_token
  7. Desktop 把 host_installation_token 传给 daemon
  8. Daemon 持久化 token，调 POST /v1/host/realtime/ws-ticket → 连接 /ws/host

Mobile 端：
  9. 用户在 Mobile 完成 D01 exchange → Minos session
  10. Mobile 调 GET /v1/hosts → 看到 Desktop 刚 link 的 host（online=true）
  11. Mobile 可以路由命令到该 host（通过 /ws/client → backend → /ws/host）
```

### Security invariant

- **daemon-local proof 必须**：纯浏览器 session 不能 link 任意远程 host。link endpoint 同时要求 account bearer（证明 human 身份）+ Ed25519 签名（证明机器控制权）。
- **unlink 立即生效**：撤销 token + kill WS + 清缓存。
- **多 host 支持**：一个 account 可以 link 多台 Mac（`UNIQUE (account_id, host_installation_id)` 允许多行）。

---

## 7. Implementation notes

### 7.1 Bootstrap nonce 迁移 Redis

现有 `BootstrapNonceStore` 是进程本地 `DashMap`。多实例部署下会失败。

方案：复用 `RealtimeTicketStore` 的 Redis 模式：
- `SET nonce:<value> <installation_id> EX 60`
- `GETDEL nonce:<value>`（consume 时原子删除）
- dev 环境用 Inline Redis（已有模式）

不阻塞 Phase 1，但 **Phase 4 之前必须完成**（因为 link endpoint 依赖 nonce）。

### 7.2 Daemon link RPC

新增 daemon JSON-RPC 方法（供 Desktop/Tauri 调用）：

```json
{
  "method": "host.prepare_link",
  "params": {}
}
→ {
  "installation_id": "...",
  "public_key": "ed25519:...",
  "nonce": "nonce_..."  // daemon 先调 backend 获取 nonce
}

{
  "method": "host.sign_link_proof",
  "params": { "installation_id": "...", "nonce": "..." }
}
→ {
  "signature": "ed25519-sig:..."
}

{
  "method": "host.apply_link_token",
  "params": { "host_installation_token": "hit_..." }
}
→ { "linked": true }  // daemon 持久化 token，连接 /ws/host
```

### 7.3 Store 层迁移

`store::devices` → `store::device_installations`（表名迁移）。这是 `T-schema-*` 任务的一部分。

---

## 8. Relationship to QR pairing（彻底移除）

| 旧路径 | 处理 |
|--------|------|
| `POST /v1/host/pairing/request-code` | **删除** |
| `POST /v1/pairing/confirm` | **删除** |
| `POST /v1/host/pairing/redeem` | **删除** |
| `POST /v1/pairing/revoke` | 由 `POST /v1/hosts/unlink` 替代 |
| `POST /v1/pairing/list-hosts` | 由 `GET /v1/hosts` 替代 |
| `DELETE /v1/pairings/:host_device_id` | 由 `POST /v1/hosts/unlink` 替代 |
| `pairing/` module | **整体删除** |
| `pairing_codes` table | **删除**（从 migration 中移除） |
| `minos-pairing` crate | **删除**（类型由 `minos-protocol` 替代） |
| QR 渲染（macOS app） | **移除** |

**不保留 QR 作为辅助路径**。如果未来需要"从手机添加另一台 Mac"，用 Mobile 端的"Link new host"按钮（Mobile 拿到 host 的 installation_id 后调 `POST /v1/hosts/link`——但需要该 host 的 daemon 参与）。

---

## 9. Exit criteria

- [ ] `POST /v1/hosts/link` + tests（link / unlink / multi-host list / host proof 验证）
- [ ] Daemon link RPC（prepare_link / sign_link_proof / apply_link_token）
- [ ] Desktop "Link this Mac" UX（登录后一键 link，无需 QR）
- [ ] Bootstrap nonce 迁移 Redis
- [ ] 旧 QR 配对代码全部移除（backend + daemon + minos-pairing crate + macOS QR）
- [ ] Unlink E2E：撤销后 remote 无法路由，Desktop 显示 Local only
- [ ] 威胁模型审查（无 browser-only bind）

---

## 10. Task slice

`T-host-01` … `T-host-08` + `T-cleanup-*` + `T-schema-*` in [tasks/TASKS.md](tasks/TASKS.md).
