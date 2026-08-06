# Unify Backend Storage Parity (SQLite Dev ↔ Postgres Prod)

> **Status:** Conditionally approved after plan review (2026-08-06).  
> Revisions below address mergeability, gate depth, write semantics, fixture blast radius, and PG test defaults.  
> Do **not** open a side track that only patches `archived_at` or dual-reads old columns.

## Breaking Change Notice

This is an intentional **latest-only** storage-model reset for `minos-backend`. Development and production must share one logical schema; only SQL dialect types differ.

Breaking effects:

- SQLite and Postgres canonical migrations change shape (column names, FKs, CHECKs, table set).
- Local `./minos-backend.db` must be wiped after upgrade (no forward-compat dual-read).
- Postgres volumes that already applied the old `0001_initial.sql` must be **dropped and recreated**. There is **no** sqlx versioned ALTER chain; editing `0001_initial.sql` does **not** reshape an existing volume.
- Applies to: local docker volumes, **`deploy/prod` VPS Postgres**, and any **dev-binary** host that already ran the old migration.
- `StoreHandle: Deref → SqlitePool` is removed (compile-time; not deferred).
- Loose `insert_device` is removed from production surface; fixtures use strict helpers only.
- Wire/API field `workspace_slug` remains public; DB columns on **both** dialects use `workspace_slug` (Postgres drops divergent `workspace_root`).

Operator migration:

1. Stop backend.
2. Wipe local SQLite: `rm -f minos-backend.db minos-backend.db-wal minos-backend.db-shm`.
3. Postgres (dev compose / prod / dev-binary): `DROP DATABASE` / recreate volume — **do not** expect automatic ALTER from file edits.
4. Boot backend once so `sqlx::migrate!` applies the new dialect file (empty DB only).
5. Re-run host link / login (no schema compatibility path).

## Feasibility Assessment

Fully feasible. Evidence:

- Runtime already selects `StoreHandle::{Sqlite, Postgres}` in `main.rs` and mounts full `/v1` + `/ws/*` for both modes; prod compose forces `external-sql`.
- Nearly every store module already dual-dispatches on `StorePoolRef`; work is **schema + constraint + bind + test** convergence, not a greenfield rewrite.
- Formal policy: Postgres is production authority; SQLite is dev convenience (`docs/backend-formal-development.md`).
- Latest-only / no dual-read is project law (`AGENTS.md`).
- Caveat: any live PG data must be wiped or manually rebuilt — acceptable under latest-only.

## Merge strategy (hard constraint — supersedes earlier “each phase mergeable” wording)

| Slice | Contents | May merge to `main` alone? |
|-------|----------|----------------------------|
| **Slice A — Phase 0** | Schema parity gate harness | **Yes.** Gate may be **red** on current tree, or CI-run with `continue-on-error` / explicit allow-fail until Slice B. Documents drift; does not change runtime. |
| **Slice B — Phases 1+2+3** | DDL + store SQL/binds + remove Deref + audit real write + rotate chain + `test_support` strict fixtures + **all** `insert_device` / `insert_device_with_executor` migrations | **Yes as one PR (or stacked PRs that land together).** **Not** mergeable as “DDL only.” Default `just backend` / SQLite suite must be green on the merge commit. |
| **Slice C — Phase 4** | PG migration + contract smoke + CI service | **Hard gate** for declaring parity done. May follow Slice B immediately; CI must export `MINOS_PG_TESTS=1`. |
| **Slice D — Phases 5–6** | compose, docs, full contract regen, closeout | After Slice C (or same train as C if small). |

**Forbidden:** merging Phase 1 DDL alone while store still uses `created_at` / `issued_at` / missing `archived_at_ms` — that breaks default SQLite and is **not** a mergeable final-architecture slice.

**Allowed exception:** Phase 0 alone with gate intentionally red as a CI signal.

When Slice B lands, also drop the **obvious lies** in `runtime_blockers` (SQLite-only migrations / handlers) in the same PR or a one-line follow-up; full platform-contract regenerate may wait for Slice D.

## Current Surface Inventory

### Boot / config

- `crates/minos-backend/src/main.rs` — storage_mode branch; external-sql warning
- `crates/minos-backend/src/config.rs` — `StorageMode`, prod guards, stale comments
- `crates/minos-backend/src/store/mod.rs` — connect/migrate, `StoreHandle`, **`Deref → SqlitePool`**, migrators
- `crates/minos-backend/src/runtime.rs` — stale `runtime_blockers`
- `schemas/minos_backend_platform_contract.json` — obsolete blockers
- `deploy/docker-compose.yml` — initdb.d mount **and** app sqlx migrate
- `deploy/prod/docker-compose.yml` — app-only migrate (correct)

### Schema

- `crates/minos-backend/migrations/sqlite/0001_initial.sql`
- `crates/minos-backend/migrations/postgres/0001_initial.sql`

### Dual-path store (all under `src/store/`)

- accounts, refresh_tokens, device_installations, host_links, host_installation_tokens
- projects, agent_sessions, agent_turns, approval_requests, host_commands
- raw_events, thread_sync_state, sessions, durable_event_log, outbox_events
- social/*, media_blobs, push_*, notification_*, agent_dispatch_queue

### Parallel / incomplete paths

- `project/mod.rs` + `store/projects.rs` — HTTP path
- `app/repositories.rs` — `StoreBackedProjectsRepository` (`archived_at_ms`), audit SQLite no-op, `ProjectRow.workspace_root`, `rotated_to_hash: None`
- `app/tx.rs` — `DbTx`
- Protocol: `ProjectSummary.workspace_slug`
- **Loose insert call sites (~60+):** not only `store/*` tests — also `ingest/mod.rs`, `envelope/mod.rs`, `realtime/*`, `host_link/mod.rs`, `jobs/*`, `host_commands`, etc. Include `insert_device_with_executor` (tx SQLite path).

### Non-store SQL that must move with column renames

- `media/mod.rs` fixtures and any `created_at` / `issued_at` / `workspace_root` greps **repo-wide** under `crates/minos-backend` (not limited to `store/`)

### Tests / CI

- `tests/*` — SQLite only today
- `.github/workflows/ci.yml` — no live PG business matrix

## Design

### Goal

One **logical schema SSOT**, two **dialect projections**.

- Application dual SQL only for **syntax** (`?` vs `$n`, `json_extract` vs `->>`, locks, claim strategies).
- Dual SQL must **not** paper over different columns, FKs, or CHECKs.

### Key design decisions

1. **Postgres is production authority; SQLite mirrors it** (tighten SQLite; do not keep loose CHECK “for tests”).

2. **Column names unified**  
   - Timestamps: always `*_ms` on both dialects.  
   - Projects: `workspace_slug` + `archived_at_ms` on both (drop PG `workspace_root`).  
   - Wire stays `workspace_slug`.  
   - Rust fields: rename to `workspace_slug` / `*_ms` in the same Slice B; **acceptance:** `rg workspace_root crates/minos-backend` is empty (or comments only explaining history).  
   Rejected: long-term `AS created_at` aliases as the model.

3. **Table set unified**

   | Action | Object |
   |--------|--------|
   | **Delete** from SQLite | `pending_approvals`, `project_sessions` |
   | **Add** to SQLite | `audit_events`, `project_members` |
   | **Add** to both if missing | FKs: `agent_sessions.agent_id → agents`, `host_commands.agent_session_id → agent_sessions`, **`push_tokens.installation_id → device_installations`** (PG already has; SQLite must match) |
   | **Align** `raw_events.session_id` | **No FK** on both (ingest may outpace `sessions`) |
   | **Do not add** | `conversations.project_id` — **decision B** (see write semantics) |
   | **Keep** PG partitions | physical-only for `durable_event_log` |

4. **Installation CHECK (identical semantics both dialects)**  
   ```
   (kind IN ('mobile','browser','desktop') AND account_id IS NOT NULL AND public_key IS NULL)
   OR (kind = 'host' AND account_id IS NULL AND public_key IS NOT NULL)
   ```  
   Direct conversations: both require `direct_account_low < direct_account_high` when kind is direct.

5. **Boolean / enum / JSONB binding (full audit list, not 2–3 files)**  
   Postgres: `bool` for BOOLEAN; `$n::enum_type` or SQL literal for ENUMs; `CAST($n AS JSONB)` for JSONB.  
   Explicit bind audit targets:

   - `thread_sync_state.running`
   - `notification_preferences` bool columns
   - `agent_dispatch_queue.mention_sender` (and any other bool columns)
   - `agent_sessions.status`, `agent_turns.role` / `status`
   - `outbox_events.status`, `host_commands.status`, `approval_requests.state`
   - JSON: `usage_json`, `params_json`, `error_json`, `acl_json`, audit `metadata`, etc.

6. **Single projects access path**  
   HTTP + repository call `store::projects` only. No second SQL dialect in `repositories.rs`.

7. **Refresh rotation**  
   Both dialects: column `rotated_to_hash`; rotate updates old row in same tx; **find/map reads the column** (never hardcode `None` after rotate). Smoke asserts read-back.

8. **Audit is P0 for parity**  
   `audit_events` on both dialects; SQLite silent no-op **deleted**. Insert always persists.

9. **Remove `Deref for StoreHandle` in Slice B** (P1→ compile-time kill; not Phase 5 emotional cleanup).

10. **Dev compose migrate once** — remove initdb.d raw SQL mount.

11. **Test matrix**  
    - Default `cargo test -p minos-backend`: PG tests **skip** unless `MINOS_PG_TESTS=1` (or explicit CI env).  
    - CI **must** `export MINOS_PG_TESTS=1` for `pg_*` tests.  
    - `just check-backend` (or documented recipe) relationship:  
      - `just check-backend` = fast SQLite suite (today’s default).  
      - `just check-backend-pg` (new) = `MINOS_PG_TESTS=1` + pg tests.  
      - Declaring storage parity complete requires **both**. Closeout lists both commands.

12. **Docs / contract** — Slice B: strip false blockers; Slice D: full regen + architecture docs.

### Minimal write semantics (no fake columns)

These are **required** for any column/table added in this workstream. No “column exists but never written” for items we claim in acceptance.

#### `project_members` (both dialects)

| Op | Behavior |
|----|----------|
| **create project** | In same tx/unit as projects row: insert `(project_id, account_id, role='owner', joined_at_ms)`. Assert members non-empty after create. |
| **list / auth (this track)** | May still authorize via `projects.account_id` as owner SSOT for product scope. Dual SSOT rule: **owner account is `projects.account_id`; `project_members` owner row is mandatory mirror for future RBAC**, not a second write-only ghost. Delete project → CASCADE members. |
| **Not in this track** | Multi-member invite, role changes, editor/viewer ACL product. |

#### `archived_at_ms` (both dialects)

**Required in Slice B (not “column only”):**

- `store::projects::archive(account_id, project_id, at_ms) -> Result<…>` sets `archived_at_ms` once (`IS NULL` guard).
- `store::projects::list` / HTTP list **default filter** `archived_at_ms IS NULL` (archived hidden unless explicit `include_archived` if already in API; if no flag, always exclude).
- Wire archive into the existing projects surface that repositories already assumed (HTTP handler or repository method that production code can call). If no public HTTP archive exists today, add the minimal endpoint or internal use-case method used by smoke — **do not** claim “archive sets archived_at_ms” without a callable store API.
- Phase 4 smoke **must** call `archive` and assert column + list exclusion.

#### `conversations.project_id` — **Decision B (do not add)**

Project linkage for agent work stays on **`agent_sessions.project_id`** (and host `sessions.project_id` where applicable).  

**Rejected:** adding `conversations.project_id` “for symmetry” without a write path (schema lie).  

If a future product track links whole conversations to projects, that track owns the column + write. **Out of this workstream’s acceptance checklist.**

#### `rotated_to_hash`

Write on rotate + **read on find/map**; repository layer must not force `None`.

#### `durable_event_log` partitions (Postgres physical)

- Fixed LIST values must match exhaustive `TopicKind` (currently the five partitions: account, conversation, project, agent_session, host).
- **Rule:** new `topic_kind` requires a new partition in the same PR as code that emits it.
- Phase 4: assert each legal kind can insert; document that illegal/unpartitioned kind fails on PG.

### Logical schema invariants (acceptance)

Non-physical:

- Same table names (allowlist: `durable_event_log_*` partition children PG-only).
- Same column names + nullability (`NOT NULL`).
- Same PK / UNIQUE **by column set** (index **names** may differ — gate must not compare index names).
- Same FK graph: `(from_table, from_cols) → (to_table, to_cols)` + `ON DELETE` action.
- Same CHECK **semantics** (see gate depth).
- App never uses `is_sqlite()` for business capability (audit, archive, members).

Physical-only allowed:

- Type encoding allowlist (INTEGER↔BIGINT, TEXT+CHECK↔ENUM, TEXT↔JSONB, INTEGER 0/1↔BOOLEAN, CITEXT↔TEXT NOCASE).
- `durable_event_log` LIST partitions + `pg_advisory_xact_lock`.
- Outbox `FOR UPDATE SKIP LOCKED` vs SQLite serial claim.
- `BEGIN IMMEDIATE` vs SERIALIZABLE (host link).
- Extensions `citext` / `pgcrypto`.

### Explicit non-goals

- Incremental ALTER migrations for released fleets.
- Removing dual SQL / query DSL.
- Redis optional in prod; multi-instance fanout proof (docs-only; not this spec).
- Full project RBAC beyond owner membership row.
- Host daemon SQLite.
- `conversations.project_id` product feature.

## Phase 0: Schema parity gate (Slice A — mergeable alone)

**File: `xtask/src/lint_schema_parity.rs`** (new) + `xtask/src/main.rs` + `just schema-parity`

Parser: **restricted SQL subset** of the two `0001_initial.sql` files (token scan acceptable). Document that multi-line constraints and `CONSTRAINT` vs column-inline CHECK must be normalized (strip comments, collapse whitespace).

| Dimension | Minimum comparison |
|-----------|-------------------|
| **tables** | Name set; allowlist partition children `durable_event_log_*` |
| **columns** | Name + nullability (`NOT NULL` vs nullable) |
| **PK / UNIQUE** | Column sets only (**ignore index names**) |
| **FK** | `(from_table, from_cols) → (to_table, to_cols)` + ON DELETE action |
| **CHECK** | Normalized text **or** structured allowed-values; **must** cover `device_installations` kind/account/public_key rule and direct conversation pair order. If full CHECK parse is too hard v1: **golden normalized strings** for critical CHECKs that both files must equal. |
| **types** | Not equal; allowlist dialect pairs only (optional soft report) |

**Regression sample:** current tree **must fail** the gate (documents real drift). Keep that as a unit fixture until Slice B makes it green.

CI: run gate; until Slice B merges, either fail the job (signal) or `continue-on-error` with a tracked issue — prefer **fail** so drift is visible.

Verification: `just schema-parity` exits non-zero on current main.

## Slice B: Phases 1+2+3 (single mergeable unit)

### Phase 1 — Canonical DDL (not mergeable alone)

**Files:**

- `crates/minos-backend/migrations/sqlite/0001_initial.sql`
- `crates/minos-backend/migrations/postgres/0001_initial.sql`

SQLite:

- `accounts` / `refresh_tokens`: all time columns `*_ms`; add `rotated_to_hash`.
- Strict installation CHECK; direct pair `low < high`.
- `projects`: `workspace_slug` + `archived_at_ms`.
- `agent_sessions.agent_id` FK; `host_commands.agent_session_id` FK; `push_tokens.installation_id` FK (align PG).
- `raw_events`: drop session FK.
- Delete `pending_approvals`, `project_sessions`.
- Add `audit_events`, `project_members`.
- **Do not** add `conversations.project_id`.

Postgres:

- `workspace_root` → `workspace_slug`.
- Match table/column/FK/CHECK graph to SQLite (minus partition children).
- Keep ENUM/JSONB/BOOLEAN/partitions as physical choices.
- Confirm partition list matches `TopicKind`.

Phase 1 **checklist (extra face):**

- [ ] `push_tokens.installation_id` FK both sides
- [ ] Index **names** may differ; unique **column sets** match
- [ ] No dead `conversations.project_id`

### Phase 2 — Store SQL, binds, Deref, write paths

**Must ship in same merge as Phase 1.**

**Files (minimum):**

- `store/accounts.rs`, `refresh_tokens.rs`, `projects.rs`, `device_installations.rs`
- `thread_sync_state.rs`, `agent_sessions.rs`, `agent_turns.rs`, `outbox_events.rs`, `host_commands.rs`, `approval_requests.rs`
- `notification_preferences.rs`, `agent_dispatch_queue.rs`, `push_tokens.rs`, `media_blobs.rs`, `social/*` as needed
- `app/repositories.rs` — projects thin wrap; audit insert always; map `rotated_to_hash` from DB; rename `workspace_root` fields
- `store/mod.rs` — **remove `Deref`**
- `http/mod.rs`, `project/mod.rs` — archive + list filter; StoreHandle-friendly ctors
- **Repo-wide rename grep:** `created_at` (accounts), `issued_at`/`expires_at`/`revoked_at` (refresh), `workspace_root` — including `media/mod.rs` fixtures

**Write path requirements (see Design):**

- `projects::create` → insert owner `project_members`
- `projects::archive` + list excludes archived
- rotate sets + find returns `rotated_to_hash`
- audit persists on SQLite

**Bind audit:** complete list in Design §5; do not stop at `thread_sync_state`.

**Platform contract (minimal):** delete false “migrations are SQLite-only / most handlers SQLite-only” blockers in `runtime.rs` (and regen JSON if required by CI).

### Phase 3 — Fixtures (same PR as 1+2 or stacked, must be green together)

Blast radius: **~60+** call sites; under-estimated if only grepping `store/`.

**File: `crates/minos-backend/src/store/test_support.rs`** (or existing test_support)

Add:

```rust
// fixed test public key; always satisfies host CHECK
pub async fn insert_test_host(store, id, name, now_ms) -> Result<(), …>

// requires existing account_id; client kinds only
pub async fn insert_test_client(store, id, role, account_id, name, now_ms) -> Result<(), …>
```

**Remove / rewire:**

- Production `insert_device` — delete or `#[cfg(test)]` thin wrapper that **only** calls strict helpers (and errors if host without key / client without account).
- `insert_device_with_executor` — same strict rules inside tx (host_link paths).

**Migrate call sites (non-exhaustive inventory — implementer greps all):**

- `store/*` unit tests
- `host_link/mod.rs` tests
- `ingest/mod.rs` tests
- `envelope/mod.rs` tests
- `realtime/*` tests
- `host_commands/*`, `jobs/*` (e.g. stale_session_sweeper)
- `refresh_tokens`, `host_installation_tokens`, `outbox_events`, etc.

**Slice B verification (all required for merge):**

```bash
rm -f minos-backend.db minos-backend.db-wal minos-backend.db-shm
just schema-parity          # green after 1+2
cargo test -p minos-backend # default: SQLite + pg tests skipped
rg 'workspace_root' crates/minos-backend --glob '*.rs'   # empty or comments only
rg 'insert_device\(' crates/minos-backend --glob '*.rs'   # none, or only re-export/test_support docs
```

## Phase 4: Postgres matrix (Slice C — hard gate)

**Default skip:** `pg_*.rs` tests no-op unless `MINOS_PG_TESTS=1` (or `CI=true` if team prefers — **document one rule**; recommended: **only** `MINOS_PG_TESTS=1` so local accidental CI env does not surprise).

**Files:**

- `Cargo.toml` — testcontainers or CI `DATABASE_URL`
- `tests/common_pg.rs`, `tests/pg_migration.rs`, `tests/pg_contract_smoke.rs`
- `.github/workflows/ci.yml` — postgres:16 service; `export MINOS_PG_TESTS=1`
- `justfile` — `check-backend-pg` recipe

**Smoke cases:**

1. Account + strict client + strict host installs.
2. Host link; refresh rotate; **read back** `rotated_to_hash`.
3. Create project → `project_members` owner row; **archive** → `archived_at_ms` set; list excludes archived.
4. Group conversation + message + durable/outbox enqueue + `claim_available`.
5. Agent session with valid `agent_id` FK; invalid `agent_id` fails.
6. `thread_sync_state` upsert `running` true/false.
7. **Partition guard:** insert durable event for each legal `topic_kind` (five kinds).
8. **Strict CHECK guard (SQLite + PG):** client insert without `account_id` fails; host insert without `public_key` fails.

Redis multi-instance: **out of scope** (docs only).

## Phase 5–6: Ops / docs / closeout (Slice D)

- `deploy/docker-compose.yml` — remove initdb.d migration mount.
- Architecture + formal-dev + local-full-chain + VPS wipe wording (no automatic ALTER).
- Full platform contract regen.
- Closeout:

```bash
just schema-parity
cargo test -p minos-backend
just check-backend-pg   # MINOS_PG_TESTS=1 …
# optional: docker compose -f deploy/docker-compose.yml up --build + health
```

**Acceptance checklist:**

- [ ] Parity gate green (tables, nullability, PK/UNIQUE cols, FK graph, critical CHECKs)
- [ ] No `pending_approvals` / `project_sessions`
- [ ] Both dialects: `audit_events`, `project_members`, `archived_at_ms`, `rotated_to_hash`
- [ ] **No** `conversations.project_id` dead column
- [ ] `project_members` owner written on create; archive API + list filter real
- [ ] No `StoreHandle: Deref`
- [ ] Audit persists on SQLite
- [ ] `rg workspace_root` clean under backend crate sources
- [ ] Strict installation fixtures; no loose `insert_device` production path
- [ ] PG smoke green in CI with `MINOS_PG_TESTS=1`
- [ ] Default `cargo test -p minos-backend` does not require Docker
- [ ] Dev compose single migrator path
- [ ] Platform blockers not lying about SQLite-only runtime

## Architectural Notes

- **Semver / clients:** wire `workspace_slug` unchanged; DB renames are wipe-only.
- **Concurrency:** keep dialect lock/claim strategies; prove PG claim in smoke.
- **Partitions:** parent table name only in queries; new kind = new partition same PR.
- **Rejected patches:** dual-read; feature flags; “DDL-only merge”; “archive column without API”; loose CHECK for tests; second plan that only fixes `archived_at`.
- **Ownership:** parity = `minos-backend` + xtask gate.
- **Policy alignment:** dual-read banned; latest-only; final architecture; delete dead tables with adds in Slice B; mergeable slices = A / B / C / D as defined above (not “Phase 1 alone”).

## File Change Summary

- `crates/minos-backend/migrations/sqlite/0001_initial.sql` — logical parity DDL
- `crates/minos-backend/migrations/postgres/0001_initial.sql` — align names/FK/CHECK; `workspace_slug`
- `crates/minos-backend/src/store/mod.rs` — remove Deref; docs
- `crates/minos-backend/src/store/test_support.rs` — `insert_test_host` / `insert_test_client`
- `crates/minos-backend/src/store/{accounts,refresh_tokens,projects,device_installations,...}.rs` — SQL + binds
- `crates/minos-backend/src/app/repositories.rs` — thin projects; audit; rotated_to_hash read; rename root→slug
- `crates/minos-backend/src/project/mod.rs` + `http/v1/projects.rs` — archive + list filter
- `crates/minos-backend/src/{ingest,envelope,realtime,host_link,jobs,...}` tests — strict fixtures
- `crates/minos-backend/src/media/mod.rs` — fixture column names if any
- `crates/minos-backend/tests/pg_*.rs`, `common_pg.rs` — PG matrix
- `crates/minos-backend/Cargo.toml` — testcontainers if used
- `deploy/docker-compose.yml` — no double migrate
- `xtask/src/lint_schema_parity.rs`, `xtask/src/main.rs` — deep parity gate
- `justfile` — `schema-parity`, `check-backend-pg`
- `.github/workflows/ci.yml` — PG service + `MINOS_PG_TESTS=1`
- `schemas/minos_backend_platform_contract.json` — regen
- `docs/architecture-backend.md`, `docs/backend-formal-development.md`, `docs/ops/*` — parity + wipe
- `docs/superpowers/specs/backend-storage-parity-design.md` — this plan

## Findings map (revised severity + slice)

| Finding | Severity | Slice |
|---------|----------|-------|
| Two divergent schemas | P0 | B |
| Projects dual path / archive write | P0 | B |
| Installation CHECK + insert_device blast radius | P0 | B |
| No PG tests | P0 | C |
| Dev compose double migrate | P0 | D (or C if touching deploy early) |
| `running` / bool / enum / JSONB binds (full list) | P0 | B |
| Audit SQLite no-op | **P0** (parity claim) | B |
| `rotated_to_hash` write+read | P1 | B |
| `project_members` owner write | P1 | B |
| FK: agent_id, host_commands, **push_tokens.installation_id** | P1 | B |
| `StoreHandle` Deref | **P1** (prod panic) | B |
| Dead tables | P2 | B |
| `workspace_root` rename completeness | P2 | B (`rg` gate) |
| Stale platform contract | P2 | B minimal + D full |
| Redis multi-instance | P1 docs-only | D |
| `conversations.project_id` dead column | **Avoid** | N/A (decision B) |
| Gate only comparing table/column names | P0 plan bug | **A fixed** (deep gate) |
| Phase 1 alone mergeable | P0 plan bug | **Superseded** by Slice B |

## Execution order (revised)

0. **Slice A:** parity gate (main may be red).  
1. **Slice B:** Phases **1+2+3** together — DDL + store + Deref + audit + rotate + archive/members writes + full strict fixture migration. SQLite suite green.  
2. **Slice C:** Phase 4 PG matrix + CI hard gate.  
3. **Slice D:** Phase 5–6 compose/docs/closeout.  

No short-term dual-read shims. No second “hotfix archived_at only” plan.
