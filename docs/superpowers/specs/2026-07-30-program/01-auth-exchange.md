# D01 · Auth Exchange (Supabase → Minos)

| Field | Value |
|-------|--------|
| Domain ID | D01 |
| Status | Implementation-ready (refined 2026-07-31) |
| L0 | [long-term spec §4.3](../2026-07-30-cloud-identity-clients-long-term.md) |
| Tasks | `T-auth-*` in [tasks/TASKS.md](tasks/TASKS.md) |
| Depends on (domain) | P0 golden path (recommended before production cutover) |
| Blocks (domain) | D02 Host link, D04 Mobile auth, Desktop account session |

---

## 1. Goal

引入外部身份（Supabase Auth: email / Google / Apple / …），同时保持 **Minos 作为业务会话权威**。

```text
Client → Supabase sign-in → access_token (Supabase JWT)
      → POST /v1/auth/supabase (exchange)
      → Minos access + refresh  (existing AuthSession shape)
      → /v1/* + WS ticket unchanged
```

**核心约束**（来自代码审计）：
- exchange endpoint **必须绕过** `authenticate()` device-secret 轨道（`http/auth.rs:59-115`）。
- exchange endpoint 自己合成 `device_installations` 行（`kind` 由 client 类型推导），然后调用 `issue_auth_session` 逻辑。
- `installation_kind` enum 需要新增 `desktop` 值。

---

## 2. Decisions (locked)

| # | Decision |
|---|----------|
| 1 | **Token exchange (Mode A)** only — 不在每个 API handler 上验证 Supabase JWT |
| 2 | **Supabase Auth only** — Minos 的 Postgres 不存 Supabase 业务数据 |
| 3 | Minos 继续签发 short-lived access JWT + refresh + WS tickets（HS256，不变） |
| 4 | Password `register`/`login` 在过渡期保留；新 UX 优先 OIDC |
| 5 | Account 的 IdP 主键：`supabase_sub`（unique）；email 是辅助 |
| 6 | Dual sessions：Supabase 用于 IdP UX；Minos tokens 用于所有 product API |
| 7 | **自动合并**：verified email 匹配时自动绑定 `supabase_sub` 到现有账户 |
| 8 | exchange endpoint **不调用** `authenticate()`；直接接受 Supabase JWT + `X-Device-Id` header |

---

## 3. Data model

### 3.1 Schema changes

**Postgres migration**（latest-only，修改 `0001_initial.sql`）：

```sql
-- 1. accounts 表加 supabase_sub
ALTER TABLE accounts ADD COLUMN supabase_sub TEXT UNIQUE;

-- 2. installation_kind enum 加 desktop
ALTER TYPE installation_kind ADD VALUE 'desktop';

-- 3. installation_kind_account_consistency CHECK 更新：
--    desktop 和 mobile/browser 一样：需要 account_id，不能有 public_key
ALTER TABLE device_installations DROP CONSTRAINT installation_kind_account_consistency;
ALTER TABLE device_installations ADD CONSTRAINT installation_kind_account_consistency CHECK (
    (kind IN ('mobile', 'browser', 'desktop') AND account_id IS NOT NULL AND public_key IS NULL) OR
    (kind = 'host' AND account_id IS NULL AND public_key IS NOT NULL)
);
```

**SQLite dev migration**（对齐 Postgres 形态，详见 D06 / TASKS.md `T-schema-*`）：

SQLite 需要从 `devices(secret_hash, role TEXT)` 迁移到 `device_installations(kind installation_kind)` 形态。这是一个独立的 schema 对齐任务（见 `T-schema-01`），不阻塞 D01 的 Postgres 实现，但 dev 测试需要它。

### 3.2 Merge policy（locked: 自动合并）

```text
exchange(supabase_jwt):
  sub = jwt.sub
  email = jwt.email (if present)
  email_verified = jwt.email_verified (if present)

  1. SELECT * FROM accounts WHERE supabase_sub = sub
     → 如果存在：login that account（更新 last_login_at_ms）

  2. 如果 sub 不存在，且 email_verified == true：
     SELECT * FROM accounts WHERE email = email
     → 如果存在：UPDATE accounts SET supabase_sub = sub WHERE account_id = ...
        → 这是"自动合并"：password 账户 + Supabase 身份关联

  3. 如果 sub 不存在，且 email 不匹配或未验证：
     INSERT INTO accounts (account_id, email, supabase_sub, ...)
        → 创建新账户（email 可以为空）

  4. 确保 device_installations 行（kind 由 client 推导）
  5. issue_auth_session(account_id, email, device_id)
```

**边界情况**：

| 情况 | 行为 |
|------|------|
| `sub` 已绑定 | 登录该账户 |
| 新 `sub`，无 email | 创建新账户（email = NULL 可接受，CITEXT UNIQUE 允许多个 NULL） |
| 新 `sub`，verified email 匹配现有 password 账户 | **绑定** `supabase_sub` |
| 新 `sub`，verified email 匹配已绑定其他 `sub` 的账户 | 409 Conflict（罕见；手动解决） |
| 新 `sub`，unverified email 匹配 | **不**自动合并；创建新账户 |

---

## 4. API contract

### `POST /v1/auth/supabase`

**Request**

```http
POST /v1/auth/supabase HTTP/1.1
Content-Type: application/json
X-Device-Id: <client device uuid>
X-Device-Role: mobile-client | browser-admin | desktop-console
X-Device-Name: optional human label

{
  "access_token": "<supabase jwt>",
  "device_name": "optional human label (overrides header)"
}
```

**Headers 说明**：
- `X-Device-Id`：必需，UUID v4。与现有 password login 相同的 device binding 语义。
- `X-Device-Role`：可选（kebab-case）。不提供时由 client 类型推导：Web=`browser-admin`、Mobile=`mobile-client`、Desktop=`desktop-console`。
- **不要求** `X-Device-Secret`——这是与 `authenticate()` 的关键区别。

**Server steps**

1. **Rate limit**：per IP（与 register 相同：3/hr）。在 JWKS 验证之前检查。
2. **JWKS verify**：加载/缓存 Supabase JWKS（`https://<project>.supabase.co/auth/v1/.well-known/jwks.json`）。验证 JWT 签名、`iss`、`aud`、`exp`。
   - `iss` = `https://<project>.supabase.co/auth/v1`（从 env `SUPABASE_URL` 推导）
   - `aud` = env `SUPABASE_JWT_AUD`（默认 = Supabase project ref）
   - JWKS cache：TTL 5min + `kid` 轮换感知
3. **Extract claims**：`sub`（必需）、`email`（如果有）、`email_verified`（如果有）。
4. **Upsert account**：按 §3.2 merge policy。
5. **Ensure device_installations row**：
   - Header `X-Device-Role` → `installation_kind` enum 映射：`mobile-client → mobile`、`browser-admin → browser`、`desktop-console → desktop`
   - 如果 `X-Device-Id` 不存在 → INSERT（`kind` 由映射推导，`account_id` = upserted account）
   - 如果已存在且 `account_id` 不同 → UPDATE `account_id`（re-bind，同 `bind_device_to_account`）
   - 如果已存在且 `account_id` 相同 → touch `last_seen_at_ms`
6. **Issue Minos session**：`jwt::sign` + `refresh_tokens::insert`（与 `issue_auth_session` 完全相同）。
7. **Return** `AuthResp`（与 `/v1/auth/login` 完全相同的 envelope）。

**Response（成功）**

```json
{
  "account": { "account_id": "...", "email": "..." },
  "access_token": "<minos jwt>",
  "refresh_token": "<minos opaque>",
  "expires_in": 900
}
```

**Response（错误）**

| Case | HTTP | code | Notes |
|------|------|------|-------|
| Missing `X-Device-Id` 或非 UUID | 401 | `unauthorized` | |
| Missing/invalid Supabase JWT | 401 | `invalid_supabase_token` | JWKS verify 失败 |
| JWKS fetch 超时 / Supabase 不可达 | 503 | `idp_unavailable` | 重试可恢复 |
| JWT `exp` 过期 | 401 | `supabase_token_expired` | client 应 refresh Supabase session |
| `iss` / `aud` 不匹配 | 401 | `supabase_token_invalid` | 配置错误 |
| Merge conflict（email 匹配但 sub 已绑他人） | 409 | `merge_conflict` | 罕见；手动解决 |
| Rate limit | 429 | `rate_limited` | + `Retry-After` header |

### Unchanged after exchange

以下端点**不变**，使用 Minos access token（Bearer）：

- `POST /v1/auth/refresh` — Minos refresh token 轮换
- `POST /v1/auth/logout` — 撤销 Minos refresh token
- `POST /v1/realtime/ws-ticket` — Minos WS ticket
- 所有 `/v1/*` Bearer 认证

---

## 5. Client responsibilities

| Client | Auth flow | Session storage |
|--------|-----------|-----------------|
| **Web** | `@supabase/supabase-js` → `signInWithOAuth({provider})` 或 `signInWithPassword` → exchange → Minos session | localStorage / sessionStorage（Zustand persist） |
| **Mobile** | `supabase_flutter` → `signInWithOAuth` 或 `signInWithPassword` → exchange → Minos session | iOS Keychain / Android Keystore |
| **Desktop** | 打开系统浏览器 → Supabase OAuth → deep link 回调 `minos://auth-callback#access_token=...` → exchange → Minos session | Tauri secure store（`tauri-plugin-stronghold` 或 keyring crate） |
| **Daemon** | 不持有 human OAuth UI；从 Desktop 接收 account token 用于 host link | 不持久化 human session |

**Logout 流程（所有 client）**：
1. `POST /v1/auth/logout`（撤销 Minos refresh token）
2. Supabase `signOut()`（best-effort；网络失败不阻塞）

**Refresh 流程**：
1. Minos refresh token 有效 → `POST /v1/auth/refresh` → 新 Minos session
2. Minos refresh token 过期/失效 → 检查 Supabase session → 如果有效 → 重新 exchange
3. Supabase session 也失效 → 用户重新登录

---

## 6. Config / secrets

| Name | Where | Notes |
|------|-------|-------|
| `SUPABASE_URL` | Server env | `https://<project>.supabase.co` |
| `SUPABASE_JWT_AUD` | Server env | 通常 = project ref（在 Supabase Settings → API → JWT Settings） |
| `SUPABASE_ANON_KEY` | Clients only | Public key，安全暴露在 client bundle |
| JWKS URL | Server 推导 | `<SUPABASE_URL>/auth/v1/.well-known/jwks.json` |
| `MINOS_JWT_SECRET` | Server env | **不变**，与 Supabase 无关 |
| Service role key | **Never** in clients | exchange 只用 JWKS verify，不需要 service role |

**Desktop deep link config**：
- `apps/desktop/src-tauri/tauri.conf.json` → `tauri-plugin-deep-link` 注册 `minos://` scheme
- Supabase OAuth redirect：`minos://auth-callback`（在 Supabase Dashboard → Authentication → URL Configuration 配置）

---

## 7. Implementation notes

### 7.1 绕过 `authenticate()`

新 endpoint `post_supabase_exchange` 在 `http/v1/auth.rs` 中注册，但**不调用** `authenticate()`。它直接从 `X-Device-Id` header 读取 device UUID（验证格式），然后：

```rust
pub async fn post_supabase_exchange(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<SupabaseExchangeReq>,
) -> Response {
    let device_id = match parse_device_id(&headers) {
        Ok(id) => id,
        Err(_) => return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response(),
    };
    let client_ip = client_ip(&headers);

    match state.auth.supabase_exchange(
        device_id,
        &req.access_token,
        derive_device_kind(&headers),
        headers.get("x-device-name").and_then(|v| v.to_str().ok()),
        &client_ip,
    ).await {
        Ok(session) => auth_session_response(session),
        Err(error) => supabase_auth_error_response(error),
    }
}
```

### 7.2 `AuthUseCase::supabase_exchange` 新方法

```rust
pub async fn supabase_exchange(
    &self,
    device_id: DeviceId,
    supabase_jwt: &str,
    device_kind: InstallationKind,
    device_name: Option<&str>,
    client_ip: &str,
) -> Result<AuthSession, AuthUseCaseError>
```

Flow:
1. `limits.check_register_per_ip(client_ip)?` — 复用 register 限速
2. `supabase::verify_jwt(self.jwks_cache, supabase_jwt, &self.config.supabase_iss, &self.config.supabase_aud)?`
3. Extract `sub`, `email`, `email_verified`
4. Upsert account per merge policy（新 `accounts::upsert_by_supabase_sub` store 方法）
5. `ensure_device_installation(device_id, device_kind, account_id, device_name)`
6. `issue_auth_session(account_id, email, device_id)`

### 7.3 JWKS client

新 crate 或 `minos-backend/src/auth/supabase.rs`：
- `JwksCache`：`Arc<RwLock<HashMap<String, JsonWebKeySet>>>`，TTL 5min
- `verify_jwt(jwks, token, iss, aud) -> Result<SupabaseClaims, _>`
- 使用 `jsonwebtoken` crate（Rust 生态标准选择）
- Supabase 使用 RS256（非对称），与 Minos 的 HS256（对称）不同

---

## 8. Exit criteria (domain)

- [ ] Postgres migration：`accounts.supabase_sub` + `installation_kind` 加 `desktop`
- [ ] JWKS client + unit tests（valid/invalid/expired/bad-iss/bad-aud）
- [ ] `POST /v1/auth/supabase` 端点 + integration tests（new user / existing sub / verified-email merge / unverified no-merge）
- [ ] 不调用 `authenticate()`；仅要求 `X-Device-Id`
- [ ] 至少一个 client（Web 优先）完成 Google/email OIDC → Minos session → 认证 `/v1` 调用
- [ ] Password login 在过渡期仍工作
- [ ] Rate limit 生效（per IP + per sub）

---

## 9. Task slice

See [tasks/TASKS.md](tasks/TASKS.md) ids: `T-auth-01` … `T-auth-09`。
