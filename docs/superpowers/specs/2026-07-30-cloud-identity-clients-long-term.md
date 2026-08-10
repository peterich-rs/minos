# Unify Cloud Identity, Host Binding, and Multi-Client Surfaces

| Field | Value |
|-------|--------|
| Status | Refined (direction accepted 2026-07-30; L1 fleshed 2026-07-31) |
| Date | 2026-07-30 (refined 2026-07-31) |
| Scope | Auth IdP, host binding UX, Web/Desktop UI SSOT, Mobile role, cloud hub |
| Related | ADR 0020 (account-keyed pairs), [vps-deploy.md](../../ops/vps-deploy.md), [architecture-desktop.md](../../architecture-desktop.md), [architecture-web.md](../../architecture-web.md), [architecture-mobile.md](../../architecture-mobile.md) |
| Non-goals (this doc) | Per-file implementation checklist; Grok/Codex protocol changes; VPS infra beyond current hub |
| Execution map (L1/L2) | [2026-07-30-program/](2026-07-30-program/README.md) — domain specs + task DAG |
| First milestone | Desktop ↔ Mobile 联通: Desktop 登录 + Link Mac + Mobile 远程查看 (Phase 0→1→4→3) |

---

## 1. Breaking Change Notice

This program is **product- and protocol-breaking** relative to today's email/password + QR-primary pairing UX. Minos remains latest-only (no dual-write / compatibility shims unless explicitly requested).

Downstream migration (all first-party clients):

1. Login becomes **account-first** (Supabase Auth → Minos token exchange), not optional demo auth.
2. Host attachment becomes **same-account link on the host machine**, not phone-scan-QR as the primary path.
3. Web abandons its independent shadcn demo shell; UI tokens and chrome track Desktop.
4. Clients that only held local-daemon state without cloud link will not appear on Mobile/Web until the host is Linked.
5. **旧配对代码彻底移除**（latest-only）：backend 的 QR 三步流程（`/v1/host/pairing/request-code` → `/v1/pairing/confirm` → `/v1/host/pairing/redeem`）、`minos-pairing` crate（v1 P2P-era `QrPayload`）、`authenticate()` device-secret 轨道、SQLite `devices`/`account_host_pairings` 表——在 Host Link API 上线后全部删除，不保留兼容层。Postgres `device_installations`/`host_links` 表已经是目标形态。

Exact wire shapes for new endpoints land in implementation PRs; this document freezes **roles, trust boundaries, and phase order**.

---

## 2. Feasibility Assessment

Evidence the direction is achievable without rewriting the hub:

- **Cloud hub is live**: `https://minos.ainexc.com` (Caddy + GHCR `minos-backend` + Postgres/Redis). Health paths and `/v1/*` + `/ws/*` allowlist already match the public surface.
- **Account-centric pairs already exist** (ADR 0020): mobile is bearer-only; pairs are account↔host, not phone-device↔host as routing key.
- **Token exchange preserves the business session model**: Minos already issues short-lived access JWT + refresh + one-shot WS tickets. Supabase becomes IdP only; `/v1/*` and WS ticket issuance stay Minos-owned.
- **Desktop UI is mature** as a Host console over daemon RPC; Web is the same React/Tailwind/Radix family and can share chrome via a Port boundary (daemon vs REST+WS).
- **Mobile is Flutter**: shares **identity and cloud API semantics**, not React components.

Feasible with caveats: account-merge rules, dual-session lifecycle (Supabase vs Minos), and host-link must be proven on-device before QR is demoted. Fully feasible as a multi-phase program.

### 2.1 Codebase findings（2026-07-31 审计）

在深化 L1 之前对代码做了精确审计，发现 3 项影响设计的约束：

**发现 1：SQLite/Postgres schema 已分叉**
- **Postgres 生产 schema** 已经是目标形态：`device_installations`（`installation_kind` enum: `mobile`/`browser`/`host` + CHECK 约束）+ `host_links`（带 `acl_json`、`link_display_name`）。没有 `secret_hash` 列。
- **SQLite dev schema** 仍然是旧形态：`devices`（带 `secret_hash` + `role` TEXT）+ `account_host_pairings`（无 `acl_json`）。
- **Rust store 层**（`store::devices`、`store::account_host_pairings`）还在用 SQLite 形态读写**两种**数据库——没有消费 Postgres 的新表名和 enum。
- **影响**：Host Link 的 Postgres schema 基础已就位，但需要 (a) 把 Rust store 层迁移到新表名，(b) 把 SQLite dev schema 对齐到相同形态。

**发现 2：`authenticate()` device-secret 轨道是历史遗留**
- `POST /v1/auth/register` 和 `POST /v1/auth/login` 都先调 `authenticate()`（`http/auth.rs:59-115`），要求 `X-Device-Id` header + 可能的 `X-Device-Secret`。
- 这是 P2P 时代的设备认证轨道，在 server-centric 模型中是 friction。
- `authenticate()` 的输出（`device_id`、`role`）被传入 `AuthUseCase::register/login`，后者调 `bind_device_to_account` 关联 `devices.account_id`。
- **影响**：Supabase exchange endpoint **必须绕过** `authenticate()`。最干净的方案是 exchange endpoint 自己合成 `device_installations` 行（role 由 client 类型推导），然后直接调 `AuthUseCase` 的 session-issuance 逻辑。

**发现 3：bootstrap nonce 是进程本地的 `DashMap`**
- `BootstrapNonceStore`（`auth/host_bootstrap.rs:28-65`）是内存 `DashMap`，不经过 Redis。
- 单实例没问题，但如果 backend 扩容到多实例，host 打到 instance A 取 nonce、打到 instance B 验证会失败。
- `RealtimeTicketStore` 已经支持 Redis，模式可复用。
- **影响**：在 Host Link 上线前（或同时），把 bootstrap nonce 迁到 Redis。不阻塞 Phase 1，但阻塞 Phase 4 之后的多实例场景。

---

## 3. Current Surface Inventory

### 3.1 Identity and sessions

| Surface | Today | Role |
|---------|--------|------|
| `POST /v1/auth/register\|login\|refresh\|logout` | Email + Argon2 + Minos JWT | Account identity |
| Mobile / Web stored session | access + refresh + device_id | Client auth |
| Daemon host credentials | host installation / device secret, pairing codes | Host identity |
| QR pairing flow | host code → phone confirm → redeem | Account↔host bind |

### 3.2 Realtime and API

| Surface | Today |
|---------|--------|
| REST | `/v1/*` under public origin |
| Client WS | `/ws/client` after WS ticket |
| Host WS | `/ws/host` after host auth |
| Config | Native often `MINOS_BACKEND_URL=wss://…`; Web `VITE_…=https://…` (scheme mapping) |

### 3.3 Clients

| Client | Data plane today | UI today |
|--------|------------------|----------|
| **Desktop** | Local daemon (projects/conversations/sessions/agents); optional cloud link incomplete as product | Strong: multi-pane Host console |
| **Web** | Cloud REST + `/ws/client` | Weak: separate shadcn demo shell |
| **Mobile** | Cloud REST + `/ws/client`; remote control via host commands | Native Flutter; many features, quality uneven; not Desktop chrome |
| **macOS menu bar** | Daemon + pairing QR | Host bootstrap / QR |
| **TUI** | Daemon-local | Host-local only |

### 3.4 Gaps vs target

- No external IdP (Google/Apple) without custom OAuth.
- QR is the **mental model** for "connect phone to Mac", though account pairs already exist in schema.
- Desktop excellence is **offline-first**; Mobile/Web cannot see unlinked local-only work.
- Web UI diverges from Desktop; no shared Port abstraction.
- Background jobs still assume some legacy schema names (non-blocking for health; separate cleanup).

---

## 4. Design

### 4.1 Product roles (long-term)

```text
┌─────────────────────────────────────────────────────────────────────────┐
│  Identity: Supabase Auth (email / Google / Apple / …)                     │
│       │ access_token                                                      │
│       ▼                                                                   │
│  Minos: POST /v1/auth/supabase (token exchange)                           │
│       │ Minos access + refresh                                            │
│       ▼                                                                   │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────────────────────────────┐│
│  │ Mobile       │  │ Web          │  │ Desktop Host Console            ││
│  │ Flutter      │  │ React Cloud  │  │ React + Tauri + minos-daemon    ││
│  │ /ws/client   │  │ /ws/client   │  │ local RPC + optional /ws/host   ││
│  │ remote view  │  │ remote view  │  │ run agents / worktrees          ││
│  │ + control    │  │ + admin      │  │ same-account "Link this Mac"    ││
│  └──────┬───────┘  └──────┬───────┘  └──────────────┬──────────────────┘│
│         │                 │                           │                   │
│         └────────────┬────┴───────────────────────────┘                   │
│                      ▼                                                    │
│              minos-backend (VPS hub)                                      │
│              accounts · account↔host · sessions projection · fanout       │
│                      ▲                                                    │
│                      │ /ws/host (Linked)                                  │
│              minos-daemon (user machine)                                  │
└─────────────────────────────────────────────────────────────────────────┘
```

| Actor | Owns | Does not own |
|-------|------|----------------|
| **Supabase Auth** | Who the human is; OAuth UX | Business data, WS, agents |
| **minos-backend** | Accounts (bound to IdP sub), hosts, REST, WS, projections | CLI agent processes |
| **minos-daemon** | Local runtime, workspaces, agent processes, local SQLite | Being the multi-user cloud |
| **Desktop UI** | Host command surface (daemon Port) | Replacing Mobile |
| **Web UI** | Cloud command/view surface (Cloud Port); **same visual system as Desktop** | Local daemon |
| **Mobile UI** | Remote view/control; **native** IA | Desktop multi-pane shell |

### 4.2 Key design decisions

1. **Token exchange (Mode A), not "verify Supabase JWT on every API"**  
   - **Choice**: Client obtains Supabase `access_token` → `POST /v1/auth/supabase` → Minos validates JWKS (`iss`/`aud`/`exp`/`sub`) → upsert account → return **existing Minos access + refresh**.  
   - **Rejected**: Every handler verifies Supabase JWT (rewrites refresh, WS ticket, device binding).  
   - **Why**: Keeps business session SSOT in Minos; IdP is swappable.

2. **Supabase Auth only — not Supabase as business DB**  
   - **Choice**: IdP only. Postgres for product data remains Minos (VPS).  
   - **Rejected**: Dual-write conversations/sessions into Supabase.  
   - **Why**: One authority for realtime and host routing.

3. **Account login on Mobile, Web, and Desktop**  
   - **Choice**: All human-facing clients sign into the same Minos account (via exchange).  
   - **Rejected**: Desktop forever anonymous local-only as the product default.  
   - **Why**: Cloud visibility for Mobile/Web requires an account key.

4. **Host binding = same-account link on the host, not QR-primary**  
   - **Choice**: On Desktop/daemon, user signs in (or confirms) with the same account, then **"Link this computer as Host"** using existing host installation crypto / proof. Backend creates/updates `account_host` relationship.  
   - **Rejected**: "Login alone makes any online daemon my host" (hijack risk).  
   - **Rejected**: Phone QR as the **only** happy path (friction; wrong for "my account everywhere").  
   - **Optional**: Keep QR/code as secondary "add another Mac from phone".

5. **Two identities remain distinct**  
   - **Human account** (Supabase sub → Minos `accounts`).  
   - **Host installation** (machine key material, `/ws/host`).  
   - **Client device** (mobile/browser device_id for refresh revocation and audit).  
   - Login does not collapse these three.

6. **Desktop UI is SSOT for React surfaces; Web shares chrome via Port**  
   - **Choice**: Extract or consume Desktop `shared/ui`, theme tokens, and high-level shells; Web implements `CloudPort` (REST + `/ws/client`); Desktop keeps `HostPort` (daemon).  
   - **Rejected**: Second shadcn app in `apps/web` with divergent layout.  
   - **Rejected**: Running full Desktop invoke/daemon stack in the browser.

7. **Mobile does not adopt Desktop multi-pane shadcn**  
   - **Choice**: Flutter keeps mobile IA; aligns on **auth, API, event semantics, and account↔host model** only.  
   - **Rejected**: Embedding Desktop React or desktop-density shadcn on phones.

8. **Cloud visibility requires Linked host + projection**  
   - **Choice**: Local-only daemon data stays local until host is Linked and events/sessions are ingested/projected. UI must state "Local only — not visible on phone".  
   - **Rejected**: Pretending offline Desktop history appears on Mobile without sync.

9. **Config origin (deferred polish)**  
   - **Choice**: Long-term prefer a single **HTTPS origin** config with client-side `https↔wss` derivation (Web already does this). Native may keep `wss://` until a dedicated cleanup.  
   - **Not blocking** identity or UI programs.

10. **Latest-only migrations**  
    - Schema and auth paths move to the target shape; no long dual-read of QR-only pairing as the primary product path once host-link ships.

### 4.3 Auth: token exchange contract (normative sketch)

```http
POST /v1/auth/supabase
Content-Type: application/json
X-Device-Id: <client device uuid>
{ "access_token": "<supabase jwt>", "device_name": "optional" }
```

Server:

1. Fetch/cache Supabase JWKS; verify JWT.
2. Extract `sub` (required), `email` / `email_verified` (if present).
3. Upsert `accounts` by `supabase_sub` (unique). Merge policy: if no row for `sub` but verified email matches existing password account → bind `supabase_sub` (configurable; default prefer merge when verified).
4. Ensure client device row (role: mobile-client | browser-admin | desktop-console as appropriate).
5. Issue Minos access JWT + refresh token (**same shape as password login**).
6. Return `{ account, access_token, refresh_token, expires_in }` compatible with existing clients.

Non-goals of this endpoint: host linking, WS upgrade, agent start.

**Dual session rule for clients:**

| Token | Used for |
|-------|----------|
| Supabase session | Sign-in UX, re-login / re-exchange when Minos refresh is dead |
| Minos access + refresh | All `/v1/*`, WS ticket, business APIs |
| Logout | Revoke Minos refresh; Supabase `signOut` (best effort) |

Password `register`/`login` may remain during transition; new surfaces prefer OIDC.

### 4.4 Host link contract (normative sketch)

```text
Preconditions:
  - User has Minos session on Desktop (via exchange or transitional password login)
  - Daemon has host installation identity (existing bootstrap crypto)

Action (daemon-local, not from arbitrary browser alone):
  - "Link this Mac to current account"
  - Prove host installation + present Minos account bearer (or host-mediated exchange)
  - Backend upserts account↔host pair; host may obtain/refresh host credentials for /ws/host

Invariants:
  - Linking requires proof of control of the machine (daemon)
  - Unlink revokes routing for that host for the account
  - Multiple hosts per account allowed (ADR 0020 multi-Mac)
```

QR pairing becomes **optional secondary** for "phone-initiated add host" if still desired.

### 4.5 UI and Port architecture

```text
                    ┌──────────────────────────┐
                    │ Shared React design system │
                    │ tokens, shared/ui, shells  │
                    │ transcript presenters      │
                    └────────────┬─────────────┘
                                 │
              ┌──────────────────┼──────────────────┐
              ▼                                     ▼
     ┌─────────────────┐                   ┌─────────────────┐
     │ Desktop app     │                   │ Web app         │
     │ HostPort        │                   │ CloudPort       │
     │ → daemon invoke │                   │ → HTTPS + WSS   │
     └─────────────────┘                   └─────────────────┘

Flutter Mobile: separate UI; implements CloudPort semantics in Dart
```

**Web P0 product surface (cloud):** Auth → Hosts list → one conversation/session timeline (read + send) → settings.  
**Desktop:** Keep Host console depth; add account session + link status + cloud presence indicators.  
**Mobile:** Golden path only until stable (login, hosts, session stream, send, basic approvals).

### 4.6 Data visibility matrix

| Data | Local daemon only | Cloud after Linked host |
|------|-------------------|-------------------------|
| Workspace paths / CLI processes | Yes | No (never send raw FS as product default) |
| Conversation / agent session projection | Yes (local) | Yes (if ingested) |
| Approvals requiring host | Yes | Prompt on Mobile/Web → command to host |
| Agent / bot identity (数字肉身) | Host may cache | **Hub `agents` SSOT** — 全局 bot 用户；见 [global-bot-identity-design](global-bot-identity-design.md)。本地 daemon/Mobile profile 不再是身份权威 |

### 4.7 Explicitly out of scope (for this program)

- Replacing minos-backend with Supabase Edge for core API.
- E2EE.
- Making VPS run agents.
- Full Desktop feature parity on Mobile UI.
- Immediate kill of password auth (transition allowed).
- Immediate `MINOS_BACKEND_URL` scheme unification (tracked as polish).

---

## 5. Phased Implementation

Phases are ordered dependencies. Each phase should leave `main` shippable. No duration estimates.

### Phase 0: Stabilize current hub golden path

- Prove on production origin: register/login (current) → pair host → host online → session → Mobile or Web observes stream.
- Fix only blockers on that path (not UI redesign).
- Document known job/schema WARNs separately.

**Exit**: One recorded E2E on `minos.ainexc.com`.

### Phase 1: Schema and auth exchange (backend)

- Add `accounts.supabase_sub` (unique, nullable) + migration (latest-only).
- Implement JWKS client + `POST /v1/auth/supabase`.
- Unit tests: valid token, bad aud, merge policy, rate limit.
- Keep password login working.

**Exit**: curl/exchange integration test green; no client UI required.

### Phase 2: Web design system + CloudPort (UI SSOT start)

- Stop investing in divergent `apps/web` demo chrome.
- Introduce shared UI package **or** temporary consume Desktop shared modules with clear boundary.
- Web: Auth screen → exchange (or password until Phase 1 clients) → hosts + one timeline via CloudPort.
- Wire `VITE_MINOS_BACKEND_URL=https://minos.ainexc.com`.

**Exit**: Web looks recognizably Desktop-themed; can login and list cloud hosts/sessions against prod/staging.

### Phase 3: Mobile auth via exchange

- Supabase Flutter Auth (or native Google) → exchange → existing Minos session store.
- Align logout and refresh with dual-session rules.
- Trim/hide non-golden-path features that fight the new model.

**Exit**: Mobile login with Google/email via exchange against hub.

### Phase 4: Desktop account session + Host link

- Desktop/daemon: sign-in (exchange) for human account.
- Implement **Link this Mac** (daemon-mediated) as primary bind path.
- UI: Linked / Local only / error; Mobile/Web visibility copy.
- Demote QR in UX (settings/advanced or secondary CTA).

**Exit**: Same account on Desktop + Mobile sees host online **without** QR as primary step.

### Phase 5: Projection completeness

- Audit which Desktop actions never reach cloud; fix ingest/fanout gaps for golden path.
- Attention/approvals remote path if not already solid.
- Reconnect and multi-host routing checks (ADR 0020).

**Exit**: Checklist: start/send/stream/stop (and critical approvals) visible remotely.

### Phase 6: Hardening and cleanup

- Optional: disable public password registration.
- QR optional or remove from primary docs.
- Config origin polish (`https` SSOT).
- Ops: image publish via version tags; secrets rotation for Supabase + `MINOS_JWT_SECRET`.
- Remove dead web demo components; update architecture-*.md to match.

**Exit**: Docs and product match this design; obsolete pairing-primary copy gone.

### Phase 7: Verification gates

- Backend: exchange + link unit/integration tests.
- Web/Desktop: `pnpm check` / existing gates; visual smoke of shared shell.
- Mobile: analyze + critical auth/session tests.
- Manual: three-client matrix (Desktop linked, Web view, Mobile view) on prod hub.

---

## 6. Architectural Notes

- **Semver / latest-only**: Internal breaking changes are expected; no dual protocol stacks.
- **Trust boundary**: Supabase tokens never authorize host commands directly; Minos session + host proof do.
- **Security**: Host link must be daemon-local; browser-only "claim host by account password" is insufficient without machine proof.
- **Availability**: If Supabase is down, **new** logins fail; existing Minos refresh continues until expiry. Optional password fallback during transition.
- **Open-source self-host**: Document "Supabase project (or compatible GoTrue) + Minos hub"; Auth-only dependency.
- **UI package strategy**: Prefer monorepo package (`packages/ui` or `apps/desktop` export map) over copy-paste; exact packaging is an implementation choice in Phase 2.
- **What does not change**: Agent runtimes on host; Caddy path allowlist shape; monolith backend default; GHCR runtime-only VPS policy.
- **ADR follow-ups**: New ADR for "Supabase exchange IdP" and "Host link primary" when Phase 1/4 land. This **supersedes** ADR 0014 (backend-assembled QR) as the primary UX path and **deletes** the QR pairing code per latest-only policy. ADR 0020 (account-keyed pairs) remains in force; its `account_mac_pairings` table is replaced by the already-existing Postgres `host_links` table.

### Open questions（resolved 2026-07-31）

以下开放问题已在深化过程中锁定，详见各 L1 域文档：

| # | 问题 | 决议 |
|---|------|------|
| 1 | Exact Supabase `aud` / JWT claim set | 在 Supabase 项目创建后从项目设置读取；exchange endpoint 接受 `iss`=`https://<project>.supabase.co/auth/v1`、`aud` 由 env `SUPABASE_JWT_AUD` 配置。不阻塞设计。 |
| 2 | Account merge: auto-merge vs explicit UI | **自动合并**（verified email 匹配时绑定 `supabase_sub`）。unverified email 不合并，创建新账户。详见 D01 §3.2。 |
| 3 | Desktop console role name for device rows | 新增 `installation_kind` enum 值 `desktop`（Postgres 已有 `mobile`/`browser`/`host`）。Desktop 登录后创建 `kind=desktop` 行。 |
| 4 | macOS menu-bar app shares Desktop login? | macOS menu-bar 从 daemon 读取 link 状态（不独立登录）。Desktop 应用拥有 account session，通过 daemon 影响 host link。 |
| 5 | Shared UI package boundary | **workspace alias**（Option B）：Web 通过 `tsconfig paths` / vite alias 直接 import `apps/desktop/src/shared/*`。第二个消费者稳定后再抽取 `packages/ui`。详见 D03 §4。 |

**Remaining open（non-blocking）：**

6. `acl_json` in `host_links`：当前默认 `'{}'`。是否在 v1 使用 ACL 限制哪些 client device 可以路由到哪个 host？还是全部 account 绑定 host 都可路由？倾向 v1 全开，ACL 留作 polish。

---

## 7. File Change Summary (expected program footprint)

Illustrative; exact diffs per phase.

**Backend**

- `crates/minos-backend/migrations/postgres/*` — `accounts.supabase_sub` (unique nullable); `installation_kind` enum 加 `desktop` 值
- `crates/minos-backend/migrations/sqlite/*` — 对齐 Postgres 形态：`devices` → `device_installations`、`account_host_pairings` → `host_links`、移除 `secret_hash`
- `crates/minos-backend/src/auth/*` — JWKS client、exchange handler；移除 `authenticate()` device-secret 轨道、`host_bootstrap.rs` nonce 迁 Redis
- `crates/minos-backend/src/http/*` — 新增 `/v1/auth/supabase`、`/v1/hosts/link`、`/v1/hosts/unlink`；**移除** `/v1/host/pairing/request-code`、`/v1/pairing/confirm`、`/v1/host/pairing/redeem`
- `crates/minos-backend/src/store/*` — `devices` → `device_installations`、`account_host_pairings` → `host_links` 表名迁移
- `crates/minos-backend/src/pairing/*` — **整体移除**（QR 三步流程被 Host Link 替代）
- Tests under `crates/minos-backend` / mobile envelope as needed

**Desktop / Web**

- `apps/desktop/src/shared/{ui,theme,lib}` — Web 通过 workspace alias 消费
- `apps/desktop` — account session（系统浏览器 OAuth + 深链接回调）、Link host UX、connection card
- `apps/web` — 替换 demo shell；CloudPort（REST + WS）；Supabase JS client + exchange
- `apps/desktop/src-tauri` — deep link handler（`minos://auth-callback?...`）

**Mobile**

- `apps/mobile/lib/**` — Supabase auth、exchange、session store、golden-path focus

**Daemon / macOS**

- `crates/minos-daemon` — account-aware host link RPC；移除 QR relay pairing 代码
- `crates/minos-pairing` — **整体移除**（v1 P2P-era crate，被 protocol crate 的类型替代）
- `crates/minos-mobile` / `crates/minos-ffi-uniffi` — 移除对 `minos-pairing` 的依赖
- `apps/macos` — 移除 QR primary；从 daemon 读取 link 状态

**Docs**

- This spec (source of truth for direction)
- `docs/architecture-overview.md`, `architecture-web.md`, `architecture-desktop.md`, `architecture-mobile.md`, `architecture-business-flow.md`, `architecture-daemon.md` — update when phases ship
- New ADRs: "Supabase exchange IdP" + "Host link primary" (supersedes ADR 0014)
- `docs/ops/*` — Supabase env, secrets

**Ops / config**

- `.env.example`, deploy env — Supabase URL/JWKS/aud server config (no client secrets in git)
- Client build: Supabase anon key (public) via existing just/CI pipeline
- Docker image: 新增 `SUPABASE_*` env vars 到 `deploy/prod/docker-compose.yml`

---

## 8. Success criteria (program level)

1. User signs in with Google (or email) on Web, Mobile, and Desktop to **one** Minos account.
2. User links a Mac from Desktop without QR as the primary path; host shows online on Mobile/Web.
3. Agent work started on Desktop (Linked) is observable on Mobile/Web for the golden path.
4. Web chrome matches Desktop design system; Web does not depend on a second ad-hoc component library.
5. Mobile remains native; no Desktop multi-pane forced on small screens.
6. Business APIs and WS tickets remain Minos-issued after exchange.

---

## 9. Suggested immediate next actions

**第一个里程碑：Desktop ↔ Mobile 联通**

1. Accept this document as the program north star (open questions resolved above).
2. Track execution in **[2026-07-30-program/](2026-07-30-program/README.md)** (L1 domains + [tasks/TASKS.md](2026-07-30-program/tasks/TASKS.md) DAG).
3. **Phase 0**：在生产 hub 上跑一遍当前 password+QR E2E（`T-p0-*`），记录真实阻塞项。
4. **Phase 1**（与 Phase 0 并行）：backend Supabase exchange（`T-auth-*`）。curl 可测，不需要客户端 UI。同时：Supabase 项目创建 + OAuth provider 配置。
5. **Phase 4**：Desktop 账户登录（系统浏览器 + 深链接）+ Host Link API + daemon link RPC。这是核心产品价值。
6. **Phase 3**：Mobile Supabase 登录 + exchange + 查看已 Linked 的 host。
7. 并行：`T-ui-01`（UI packaging 决策已锁：workspace alias）、`T-proj-01`（gap audit 可随时开始）。

**关键路径**：见 [README §First milestone](2026-07-30-program/README.md#first-milestone-desktoptop--mobile-联通)。
核心依赖链：`T-auth-04`（exchange 测试绿）→ `T-host-01..04`（Host Link）→ `T-mob-01..04`（Mobile）。

---

## Appendix A — Comparison: today vs target UX

| Step | Today (typical) | Target |
|------|-----------------|--------|
| Create identity | Email/password on Minos | Supabase (Google/email/…) |
| API session | Minos JWT from password login | Minos JWT from **exchange** |
| Attach Mac | QR on Mac, scan with phone | Login on Desktop + **Link this Mac** |
| See work on phone | After pair + host online | After account login + host Linked + projection |
| Web look | Demo shadcn | Desktop system + CloudPort |
| Desktop | Local daemon hero | Local daemon hero **+** account/cloud link |

## Appendix B — Threat notes (short)

- Stolen Supabase access_token alone: exchange yields Minos session for that user — protect tokens like passwords; short TTL.
- Stolen Minos refresh: same as today — device-bound revoke story remains.
- Malicious website with user's Minos cookie cannot link host without daemon proof.
- Public GHCR image does not expose JWT secrets; Supabase service role never ships to clients.
