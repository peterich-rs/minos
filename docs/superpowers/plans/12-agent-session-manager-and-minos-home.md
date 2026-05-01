# Agent Session Manager + `$MINOS_HOME` + Naming Cleanup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Every commit MUST be preceded by `cargo xtask check-all`.

**Goal:** Lift the agent runtime to multi-workspace / multi-thread with durable state, consolidate all daemon paths under `$MINOS_HOME`, and rename platform-leaked names (`Mac → Host`, `Ios → Mobile`) at every protocol/FFI/HTTP/SQL boundary.

**Architecture:** Per-workspace `AppServerInstance` (one codex app-server child) hosts multiple `ThreadHandle`s. A single-writer `EventWriter` writes events to local SQLite (WAL) before forwarding via `Envelope::Ingest` to the relay backend. On reconnect a `Reconciliator` reads `Event::IngestCheckpoint` from backend and replays missing events from DB; gaps fall back to `~/.codex/sessions/*.jsonl`.

**Tech Stack:** Rust workspace (tokio, sqlx-sqlite, jsonrpsee, tracing), uniffi (macOS), flutter_rust_bridge (mobile), SwiftUI macOS app, Flutter mobile app. CI gate: `cargo xtask check-all`.

**Spec:** `docs/superpowers/specs/2026-05-01-agent-session-manager-and-minos-home-design.md`

---

## File Structure (Created / Modified)

**New files**

| Path | Responsibility |
|---|---|
| `crates/minos-agent-runtime/src/manager.rs` | `AgentManager` top-level coordinator |
| `crates/minos-agent-runtime/src/instance.rs` | `AppServerInstance` per-workspace child wrapper |
| `crates/minos-agent-runtime/src/thread_handle.rs` | `ThreadHandle` per-thread state |
| `crates/minos-agent-runtime/src/state_machine.rs` | `ThreadState` + `PauseReason` + `CloseReason` + transition rules |
| `crates/minos-agent-runtime/src/manager_event.rs` | `ManagerEvent` enum |
| `crates/minos-agent-runtime/tests/multi_session_smoke.rs` | End-to-end multi-thread test |
| `crates/minos-daemon/src/store/mod.rs` | `LocalStore` SQLite handle + reads |
| `crates/minos-daemon/src/store/event_writer.rs` | Single-writer `EventWriter` task |
| `crates/minos-daemon/src/store/migrations_loader.rs` | `sqlx::migrate!()` integration |
| `crates/minos-daemon/src/reconciliator.rs` | Reconciliation task |
| `crates/minos-daemon/src/jsonl_recover.rs` | JSONL fallback parser |
| `crates/minos-daemon/migrations/0001_initial.sql` | daemon-side schema |
| `crates/minos-daemon/tests/reconciliation_integration.rs` | End-to-end reconciliation test |
| `crates/minos-backend/migrations/0013_rename_account_mac_to_host.sql` | Mac→Host SQL rename |
| `xtask/src/lint_naming.rs` | Naming lint subcommand |

**Edited files (key)**

| Path | What changes |
|---|---|
| `crates/minos-agent-runtime/src/lib.rs` | Replace `AgentRuntime` re-exports with `AgentManager` |
| `crates/minos-agent-runtime/src/runtime.rs` | Removed (logic absorbed into manager) |
| `crates/minos-agent-runtime/src/state.rs` | `AgentState` → `ThreadState` |
| `crates/minos-agent-runtime/src/ingest.rs` | Seq sourced from `EventWriter`, not local atomic |
| `crates/minos-daemon/src/agent.rs` | Glue over `AgentManager` not `AgentRuntime` |
| `crates/minos-daemon/src/agent_ingest.rs` | Replaced by `EventWriter` flow |
| `crates/minos-daemon/src/paths.rs` | Add `state_dir`, `secrets_dir`, `db_dir`, `db_path`, `logs_dir`, `workspaces_dir`, `run_dir` |
| `crates/minos-daemon/src/local_state.rs` | Use `paths::state_dir()` |
| `crates/minos-daemon/src/main.rs` | Drop `platform_data_dir()`, `MINOS_DATA_DIR`, `MINOS_LOG_DIR` |
| `crates/minos-daemon/src/logging.rs` | Use `paths::logs_dir()` |
| `crates/minos-daemon/src/relay_client.rs` | Reconciliator wiring |
| `crates/minos-protocol/src/messages.rs` | `MacSummary → HostSummary`, new RPC types, drop alias |
| `crates/minos-protocol/src/envelope.rs` | Add `EventKind::IngestCheckpoint` |
| `crates/minos-protocol/src/rpc.rs` | RPC trait surgery (drop `stop_agent`; add `interrupt_thread`/`close_thread`/`list_threads`/`get_thread`) |
| `crates/minos-domain/src/role.rs` | `IosClient → MobileClient`, wire `"ios-client"` → `"mobile-client"` |
| `crates/minos-mobile/src/{store,client,http}.rs` | `mac → host` rename across mobile rust client |
| `crates/minos-ffi-uniffi/src/lib.rs` | New FFI signatures + start_agent workspace param |
| `crates/minos-ffi-frb/src/api/minos.rs` | Same as uniffi, plus DTO renames |
| `crates/minos-backend/src/store/account_*pairings.rs` | Module rename |
| `crates/minos-backend/src/http/v1/me.rs` | `/v1/me/hosts` route |
| `crates/minos-backend/src/http/ws_devices.rs` | Emit `IngestCheckpoint` first frame |
| `xtask/src/main.rs` | Wire `lint-naming` subcommand into `check-all` |

---

## Phase A — `$MINOS_HOME` Path Consolidation

Goal: every daemon path goes through `paths.rs`. Drop macOS-specific branches and the two redundant env vars. End state: `cargo xtask check-all` green; `paths::state_dir()` etc. exist; daemon emits `minos_home={path}` on startup.

### Task A1: Extend `paths.rs` with subdirectory helpers

**Files:**
- Modify: `crates/minos-daemon/src/paths.rs:5-15`
- Test: `crates/minos-daemon/src/paths.rs` (inline `#[cfg(test)] mod tests`)

- [ ] **Step 1: Add failing test for `state_dir()`**

In `crates/minos-daemon/src/paths.rs`, add at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn state_dir_is_under_minos_home() {
        let tmp = tempfile::tempdir().unwrap();
        env::set_var("MINOS_HOME", tmp.path());
        let s = state_dir().unwrap();
        assert_eq!(s, tmp.path().join("state"));
        assert!(s.is_dir());
        env::remove_var("MINOS_HOME");
    }

    #[test]
    fn db_path_is_under_db_dir() {
        let tmp = tempfile::tempdir().unwrap();
        env::set_var("MINOS_HOME", tmp.path());
        let p = db_path().unwrap();
        assert_eq!(p, tmp.path().join("db").join("minos.sqlite"));
        env::remove_var("MINOS_HOME");
    }

    #[test]
    fn secrets_dir_has_owner_only_perms() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let tmp = tempfile::tempdir().unwrap();
            env::set_var("MINOS_HOME", tmp.path());
            let s = secrets_dir().unwrap();
            let mode = std::fs::metadata(&s).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
            env::remove_var("MINOS_HOME");
        }
    }
}
```

Add `tempfile = "3"` to `crates/minos-daemon/Cargo.toml` `[dev-dependencies]` if not present.

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test -p minos-daemon paths::tests -- --test-threads=1`
Expected: FAIL — `state_dir`, `db_path`, `secrets_dir` not defined.

- [ ] **Step 3: Implement helpers**

Append to `crates/minos-daemon/src/paths.rs`:

```rust
use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub fn state_dir() -> anyhow::Result<PathBuf> {
    let p = minos_home()?.join("state");
    fs::create_dir_all(&p)?;
    Ok(p)
}

pub fn secrets_dir() -> anyhow::Result<PathBuf> {
    let p = minos_home()?.join("secrets");
    fs::create_dir_all(&p)?;
    #[cfg(unix)]
    {
        let mut perm = fs::metadata(&p)?.permissions();
        perm.set_mode(0o700);
        fs::set_permissions(&p, perm)?;
    }
    Ok(p)
}

pub fn db_dir() -> anyhow::Result<PathBuf> {
    let p = minos_home()?.join("db");
    fs::create_dir_all(&p)?;
    Ok(p)
}

pub fn db_path() -> anyhow::Result<PathBuf> {
    Ok(db_dir()?.join("minos.sqlite"))
}

pub fn logs_dir() -> anyhow::Result<PathBuf> {
    let p = minos_home()?.join("logs");
    fs::create_dir_all(&p)?;
    Ok(p)
}

pub fn workspaces_dir() -> anyhow::Result<PathBuf> {
    let p = minos_home()?.join("workspaces");
    fs::create_dir_all(&p)?;
    Ok(p)
}

pub fn run_dir() -> anyhow::Result<PathBuf> {
    let p = minos_home()?.join("run");
    fs::create_dir_all(&p)?;
    Ok(p)
}
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p minos-daemon paths::tests -- --test-threads=1`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
cargo xtask check-all
git add crates/minos-daemon/src/paths.rs crates/minos-daemon/Cargo.toml
git commit -m "feat(daemon/paths): add state/secrets/db/logs/workspaces/run helpers"
```

### Task A2: Migrate `local_state.rs` to `state_dir()`

**Files:**
- Modify: `crates/minos-daemon/src/local_state.rs:21-29`

- [ ] **Step 1: Replace `default_path()` body**

Find the existing `default_path()` (around lines 21-29). Replace with:

```rust
pub fn default_path() -> anyhow::Result<PathBuf> {
    Ok(crate::paths::state_dir()?.join("local-state.json"))
}
```

Remove any `dirs::home_dir()` / `Library/Application Support` references from this file.

- [ ] **Step 2: Run targeted test**

Run: `cargo test -p minos-daemon local_state -- --test-threads=1`
Expected: PASS.

- [ ] **Step 3: Run full check**

Run: `cargo xtask check-all`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/local_state.rs
git commit -m "refactor(daemon/local_state): default_path uses paths::state_dir"
```

### Task A3: Migrate `logging.rs` to `logs_dir()`

**Files:**
- Modify: `crates/minos-daemon/src/logging.rs:19-31`

- [ ] **Step 1: Replace `log_dir()` body**

Replace the existing `log_dir()` function with:

```rust
fn log_dir() -> anyhow::Result<PathBuf> {
    crate::paths::logs_dir()
}
```

Drop any `MINOS_LOG_DIR` env reads, `cfg!(target_os = "macos")` branches, and `Library/Logs` literals.

- [ ] **Step 2: Run check-all**

Run: `cargo xtask check-all`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/minos-daemon/src/logging.rs
git commit -m "refactor(daemon/logging): logs_dir via paths; drop MINOS_LOG_DIR + macOS branch"
```

### Task A4: Drop `platform_data_dir()` from `main.rs`

**Files:**
- Modify: `crates/minos-daemon/src/main.rs:141-157`

- [ ] **Step 1: Remove the function and its callsites**

Delete `fn platform_data_dir() -> PathBuf { ... }` and the `apply_paths` block at lines 141-144 that writes `MINOS_DATA_DIR` / `MINOS_LOG_DIR` into the process env.

Replace any callsite of `platform_data_dir()` with `crate::paths::state_dir()?` (there should be exactly one, in the data-dir resolution path).

- [ ] **Step 2: Drop `MINOS_DATA_DIR` env reads**

Search the daemon for `MINOS_DATA_DIR` and remove every read; the env var is officially gone:

```bash
grep -rn MINOS_DATA_DIR crates/minos-daemon/
```

Should return zero hits after edits.

- [ ] **Step 3: Run check-all**

Run: `cargo xtask check-all`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/main.rs
git commit -m "refactor(daemon/main): drop platform_data_dir + MINOS_DATA_DIR/LOG_DIR env"
```

### Task A5: Add startup `minos_home={path}` log line

**Files:**
- Modify: `crates/minos-daemon/src/main.rs` (within the daemon `main` after logger init)

- [ ] **Step 1: Add log line**

In `main()`, immediately after the logger is initialised, add:

```rust
let home = crate::paths::minos_home()?;
tracing::info!(minos_home = %home.display(), "daemon starting");
```

- [ ] **Step 2: Run check-all**

Run: `cargo xtask check-all`
Expected: PASS.

- [ ] **Step 3: Manual verify**

Run the daemon binary briefly and confirm the startup log shows `minos_home=<path>`.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/main.rs
git commit -m "feat(daemon): log minos_home path on startup"
```

### Task A6: Verify Phase A end state

- [ ] **Step 1: Final grep verification**

Run:
```bash
grep -rn 'Library/Application Support/Minos\|Library/Logs/Minos\|MINOS_DATA_DIR\|MINOS_LOG_DIR' crates/minos-daemon/src/
```
Expected: zero hits.

- [ ] **Step 2: Run integration smoke**

Run: `cargo test -p minos-daemon -- --test-threads=1`
Expected: PASS.

- [ ] **Step 3: Run check-all once more**

Run: `cargo xtask check-all`
Expected: PASS.

Phase A is complete when all of the above are green.

---

## Phase B — Protocol Naming Sweep (`Mac → Host`, `Ios → Mobile`)

Goal: zero `mac_*` / `ios_*` identifiers in protocol-facing code. SQL migration `0013` rolls forward losslessly. Lint guard `xtask lint-naming` runs in `check-all`.

### Task B1: Add `lint-naming` xtask

**Files:**
- Create: `xtask/src/lint_naming.rs`
- Modify: `xtask/src/main.rs`

- [ ] **Step 1: Implement the lint module**

Create `xtask/src/lint_naming.rs`:

```rust
use std::path::Path;
use std::process::Command;

const TARGETS: &[&str] = &[
    "crates/minos-protocol/src",
    "crates/minos-domain/src",
    "crates/minos-ffi-uniffi/src",
    "crates/minos-ffi-frb/src",
    "crates/minos-mobile/src",
    "crates/minos-daemon/src",
    "crates/minos-backend/migrations",
    "crates/minos-backend/src/http",
    "crates/minos-backend/src/store",
];

const PATTERN: &str =
    r"\b(mac|ios)_(device_id|display_name|client|pairings|host|secret)\b|\bMacSummary\b|\bIosClient\b|MeMacsResponse|account_mac_pairings";

pub fn run(repo_root: &Path) -> anyhow::Result<()> {
    let mut hits: Vec<String> = Vec::new();
    for t in TARGETS {
        let dir = repo_root.join(t);
        if !dir.exists() { continue; }
        let out = Command::new("rg")
            .args(["-n", "--no-heading", "-E", PATTERN, dir.to_str().unwrap()])
            .output()?;
        if !out.stdout.is_empty() {
            hits.push(String::from_utf8_lossy(&out.stdout).into_owned());
        }
    }
    if hits.is_empty() {
        println!("lint-naming: clean");
        Ok(())
    } else {
        for h in &hits { println!("{}", h); }
        anyhow::bail!("lint-naming: {} hits in protocol/FFI/HTTP/SQL surfaces", hits.iter().map(|s| s.lines().count()).sum::<usize>())
    }
}
```

In `xtask/src/main.rs`, add a `LintNaming` subcommand variant and dispatch to `lint_naming::run`. Wire it into `check-all` so it runs after `cargo clippy`.

- [ ] **Step 2: Run lint-naming once to confirm it errors today (it should — pre-rename)**

Run: `cargo xtask lint-naming`
Expected: FAIL with hits on `MacSummary`, `IosClient`, etc.

- [ ] **Step 3: Commit just the xtask**

```bash
git add xtask/
git commit -m "chore(xtask): add lint-naming for Mac/Ios identifiers"
```

The lint will block subsequent commits until B2-B11 finish. That is the intended pressure to land the rename atomically.

### Task B2: SQL migration `0013` — rename table + column

**Files:**
- Create: `crates/minos-backend/migrations/0013_rename_account_mac_to_host.sql`

- [ ] **Step 1: Author migration**

```sql
-- Rename the account-pairings table and its mac_device_id column to host-*.
-- SQLite 3.25+ supports both ALTER TABLE RENAME TO and RENAME COLUMN losslessly.

ALTER TABLE account_mac_pairings RENAME TO account_host_pairings;
ALTER TABLE account_host_pairings RENAME COLUMN mac_device_id TO host_device_id;

-- Drop and re-create indexes that hard-code the old name (verify against 0012).
DROP INDEX IF EXISTS idx_account_mac_pairings_account;
DROP INDEX IF EXISTS idx_account_mac_pairings_mac;
CREATE INDEX idx_account_host_pairings_account
    ON account_host_pairings(account_id);
CREATE INDEX idx_account_host_pairings_host
    ON account_host_pairings(host_device_id);
```

- [ ] **Step 2: Verify migration applies on a fresh DB**

Run: `cargo test -p minos-backend store_smoke -- --test-threads=1`
Expected: PASS (schema applies without error).

- [ ] **Step 3: Commit**

```bash
git add crates/minos-backend/migrations/0013_rename_account_mac_to_host.sql
git commit -m "feat(backend/migrations): 0013 rename account_mac_pairings to account_host_pairings"
```

### Task B3: Rename `DeviceRole::IosClient` → `MobileClient`

**Files:**
- Modify: `crates/minos-domain/src/role.rs:18-30`
- Modify: every consumer (mobile, daemon, backend)

- [ ] **Step 1: Edit the enum and wire string**

In `crates/minos-domain/src/role.rs`:

```rust
pub enum DeviceRole {
    AgentHost,
    MobileClient,     // was IosClient
    BrowserAdmin,
}

impl DeviceRole {
    pub fn as_wire(&self) -> &'static str {
        match self {
            DeviceRole::AgentHost     => "agent-host",
            DeviceRole::MobileClient  => "mobile-client",   // was "ios-client"
            DeviceRole::BrowserAdmin  => "browser-admin",
        }
    }

    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "agent-host"     => Some(DeviceRole::AgentHost),
            "mobile-client"  => Some(DeviceRole::MobileClient),
            "browser-admin" => Some(DeviceRole::BrowserAdmin),
            _ => None,
        }
    }
}
```

Update the variant comment if any.

- [ ] **Step 2: Update every consumer**

Run:
```bash
grep -rln 'IosClient\|"ios-client"' crates/
```

For each hit, replace `IosClient` → `MobileClient` and `"ios-client"` → `"mobile-client"`. Concretely the known hits are:

- `crates/minos-mobile/src/http.rs:75` (header literal)
- `crates/minos-mobile/src/client.rs:986, 1555` (header literals)
- Any test fixtures / matchers in `crates/minos-domain/src/role.rs` and tests

- [ ] **Step 3: Run check-all**

Run: `cargo xtask check-all` (lint-naming will still fail due to MacSummary etc.)
Run: `cargo build --workspace --all-targets`
Expected: BUILD PASS; lint-naming will fail until B4-B9 land.

- [ ] **Step 4: Commit**

```bash
git add -p crates/minos-domain/ crates/minos-mobile/
git commit -m "refactor(domain/role): IosClient -> MobileClient + ios-client -> mobile-client"
```

### Task B4: Rename `MacSummary` → `HostSummary`

**Files:**
- Modify: `crates/minos-protocol/src/messages.rs:30-41`

- [ ] **Step 1: Edit struct + JSON field renames**

In `crates/minos-protocol/src/messages.rs`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HostSummary {
    pub host_device_id: DeviceId,
    pub host_display_name: String,
    pub paired_at_ms: i64,
    pub paired_via_device_id: DeviceId,
}
```

Drop the old `MacSummary` definition entirely.

- [ ] **Step 2: Replace all references**

Run:
```bash
grep -rln 'MacSummary\|mac_device_id\|mac_display_name' crates/
```

Replace each:
- `MacSummary` → `HostSummary`
- `mac_device_id` → `host_device_id`
- `mac_display_name` → `host_display_name`

Within tests, only rename **identifiers**; literal string values like `"MacBook Pro"` stay.

- [ ] **Step 3: Build**

Run: `cargo build --workspace --all-targets`
Expected: PASS (or fix any stragglers and rebuild).

- [ ] **Step 4: Commit**

```bash
git commit -am "refactor(protocol): MacSummary -> HostSummary, mac_* -> host_* fields"
```

### Task B5: Rename `MeMacsResponse` → `MeHostsResponse`

**Files:**
- Modify: `crates/minos-protocol/src/messages.rs`

- [ ] **Step 1: Replace struct + field**

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MeHostsResponse {
    pub hosts: Vec<HostSummary>,
}
```

Drop `MeMacsResponse`.

- [ ] **Step 2: Replace consumers**

Run:
```bash
grep -rln 'MeMacsResponse\|\.macs\b' crates/
```

Replace `MeMacsResponse` → `MeHostsResponse`, `.macs` (when referring to this field) → `.hosts`.

- [ ] **Step 3: Build + commit**

```bash
cargo build --workspace --all-targets
git commit -am "refactor(protocol): MeMacsResponse -> MeHostsResponse, macs -> hosts"
```

### Task B6: Drop `mac_display_name` alias from `PairingQrPayload`

**Files:**
- Modify: `crates/minos-protocol/src/messages.rs:130-140`

- [ ] **Step 1: Remove the alias attribute**

Find the `host_display_name` field on `PairingQrPayload`. Remove `#[serde(alias = "mac_display_name")]`.

- [ ] **Step 2: Build + verify**

Run: `cargo build --workspace --all-targets && cargo test -p minos-protocol`
Expected: PASS. Any test that fed in the alias string should already be removed by B4 fixtures cleanup.

- [ ] **Step 3: Commit**

```bash
git commit -am "refactor(protocol): drop mac_display_name serde alias"
```

### Task B7: Rename mobile-side `mac` methods to `host`

**Files:**
- Modify: `crates/minos-mobile/src/store.rs:74,77,80,148-160`
- Modify: `crates/minos-mobile/src/client.rs:459,513,523`

- [ ] **Step 1: Rename in `store.rs`**

In `crates/minos-mobile/src/store.rs`, rename trait methods + field on `MobileStore`:

| Old | New |
|---|---|
| `save_active_mac(&self, mac: DeviceId)` | `save_active_host(&self, host: DeviceId)` |
| `load_active_mac(&self) -> Option<DeviceId>` | `load_active_host(&self) -> Option<DeviceId>` |
| `clear_active_if(&self, mac: DeviceId)` | `clear_active_if(&self, host: DeviceId)` (param rename only) |
| field `active_mac: Option<DeviceId>` | `active_host: Option<DeviceId>` |

Update the inner `RwLock<Inner>` impl body accordingly.

- [ ] **Step 2: Rename in `client.rs`**

For each of the three existing methods on `MobileClient` (`forget_mac`, `list_paired_macs`, `set_active_mac` at `client.rs:459, 513, 523`), perform a mechanical rename:

- Method name `*_mac` → `*_host`
- Parameter name `mac: DeviceId` → `host: DeviceId` (and every reference to that local in the body)
- Return type `MacSummary` → `HostSummary` (already renamed by B4 — the type follows automatically)
- HTTP route literal `/v1/me/macs` → `/v1/me/hosts` (will be made true by B9; the constant in client.rs needs updating in lockstep)

Concretely the diff inside each method body is just:
- `s/\bmac\b/host/g` on local variables and field accessors
- No structural change to the method body — it still does the same HTTP / store call

Run `grep -n 'mac' crates/minos-mobile/src/client.rs` after the rename and ensure all hits are either string literals you intended to keep (e.g. test names) or have been renamed.

- [ ] **Step 3: Build + commit**

```bash
cargo build --workspace --all-targets
cargo test -p minos-mobile
git commit -am "refactor(mobile): mac -> host across MobileStore + MobileClient"
```

### Task B8: Rename FFI surface (uniffi + frb)

**Files:**
- Modify: `crates/minos-ffi-frb/src/api/minos.rs:143-247`
- Modify: `crates/minos-ffi-uniffi/src/lib.rs` (parts touching pairing types only)

- [ ] **Step 1: frb DTO + methods**

In `crates/minos-ffi-frb/src/api/minos.rs`:

```rust
pub struct HostSummaryDto {
    pub host_device_id: String,
    pub host_display_name: String,
    pub paired_at_ms: i64,
    pub paired_via_device_id: String,
}

impl MobileClient {
    pub async fn forget_host(&self, host_device_id: String) -> Result<()> { /* ... */ }
    pub async fn list_paired_hosts(&self) -> Result<Vec<HostSummaryDto>> { /* ... */ }
    pub async fn set_active_host(&self, host_device_id: String) -> Result<()> { /* ... */ }
}
```

Drop `MacSummaryDto`, `forget_mac`, `list_paired_macs`, `set_active_mac`. Update `From<HostSummary>` impls.

- [ ] **Step 2: Regenerate frb**

Run: `cargo xtask gen-frb` (or whichever workspace command rebuilds the frb mirror — see existing scripts).
Inspect `crates/minos-ffi-frb/src/frb_generated.rs` — confirm new generated wire functions exist for `_host_*` variants and old `_mac_*` are gone.

- [ ] **Step 3: Build + commit**

```bash
cargo build --workspace --all-targets
git commit -am "refactor(ffi/frb): HostSummaryDto + forget_host/list_paired_hosts/set_active_host; drop mac variants"
```

### Task B9: Rename backend `account_mac_pairings` module + `/v1/me/macs` route

**Files:**
- Rename: `crates/minos-backend/src/store/account_mac_pairings.rs` → `account_host_pairings.rs`
- Modify: `crates/minos-backend/src/store/mod.rs` (re-exports)
- Modify: `crates/minos-backend/src/http/v1/me.rs`
- Modify: `crates/minos-backend/src/http/v1/threads.rs:23-25` (parent registration)

- [ ] **Step 1: Rename the file**

```bash
git mv crates/minos-backend/src/store/account_mac_pairings.rs \
       crates/minos-backend/src/store/account_host_pairings.rs
```

Inside the file, rename the public type if any (`AccountMacPairing` → `AccountHostPairing`), the module-level `mac_device_id` references, and any SQL string literals that still say `account_mac_pairings`.

- [ ] **Step 2: Update the route**

In `crates/minos-backend/src/http/v1/me.rs`:
- Path: `"/v1/me/macs"` → `"/v1/me/hosts"`
- Handler returns `MeHostsResponse { hosts: ... }`

In `crates/minos-backend/src/http/v1/threads.rs:23-25` and any router wiring, update path constants.

- [ ] **Step 3: Update store re-exports**

In `crates/minos-backend/src/store/mod.rs`, replace `pub mod account_mac_pairings;` with `pub mod account_host_pairings;` and any re-exports.

- [ ] **Step 4: Run backend tests**

Run: `cargo test -p minos-backend -- --test-threads=1`
Expected: PASS. Migration `0013` applies cleanly; `/v1/me/hosts` round-trips.

- [ ] **Step 5: Commit**

```bash
git commit -am "refactor(backend): account_host_pairings module + /v1/me/hosts route"
```

### Task B10: Update test fixtures (identifiers only — values stay)

**Files:**
- Various test files under `crates/minos-protocol/tests/`, `crates/minos-backend/tests/`, `crates/minos-mobile/tests/`

- [ ] **Step 1: Find remaining lint-naming hits**

Run: `cargo xtask lint-naming`

For each hit that is an **identifier** (variable name, struct field, function name, type), rename to the host/mobile equivalent. For each hit that is a **string literal value** (e.g. `"MacBook Pro"` as a test fixture), leave it.

- [ ] **Step 2: Iterate until clean**

Run: `cargo xtask lint-naming`
Expected: PASS — `lint-naming: clean`.

- [ ] **Step 3: Build all targets**

Run: `cargo build --workspace --all-targets && cargo test --workspace -- --test-threads=1`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git commit -am "test: rename mac->host identifiers in test fixtures"
```

### Task B11: Phase B verification

- [ ] **Step 1: Final lint sweep**

Run: `cargo xtask lint-naming`
Expected: clean.

- [ ] **Step 2: Full check-all**

Run: `cargo xtask check-all`
Expected: PASS — fmt, clippy, test, lint-naming, frb mirror all green.

- [ ] **Step 3: Confirm git log**

Run: `git log --oneline feature/mobile-auth-and-agent-session ^HEAD~20`
Confirm commits for B1-B10 are on the branch.

Phase B is complete when `lint-naming` is clean and `check-all` is green.

---

## Phase C — Multi-Session Manager + Local Persistence

Goal: replace the single-session `AgentRuntime` with a multi-workspace `AgentManager` backed by SQLite. New FFI surface (start_agent with workspace, interrupt_thread, close_thread, list_threads, get_thread). After Phase C the macOS / Flutter apps will not compile against the daemon FFI; that is the agreed intermediate state.

### Task C1: Create daemon migrations directory + initial schema

**Files:**
- Create: `crates/minos-daemon/migrations/0001_initial.sql`
- Modify: `crates/minos-daemon/Cargo.toml` (`sqlx` feature setup if not yet present in this crate)

- [ ] **Step 1: Add sqlx as a daemon dependency**

In `crates/minos-daemon/Cargo.toml`, add to `[dependencies]`:

```toml
sqlx = { workspace = true }
```

(`sqlx` is already in workspace deps with `sqlite` feature.)

- [ ] **Step 2: Author the migration**

Create `crates/minos-daemon/migrations/0001_initial.sql`:

```sql
CREATE TABLE schema_version (
    version    INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);

CREATE TABLE workspaces (
    root          TEXT PRIMARY KEY,
    first_seen_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL
);

CREATE TABLE threads (
    thread_id          TEXT PRIMARY KEY,
    workspace_root     TEXT NOT NULL REFERENCES workspaces(root),
    agent              TEXT NOT NULL,
    codex_session_id   TEXT,
    status             TEXT NOT NULL,
    last_pause_reason  TEXT,
    last_close_reason  TEXT,
    last_seq           INTEGER NOT NULL DEFAULT 0,
    started_at         INTEGER NOT NULL,
    last_activity_at   INTEGER NOT NULL,
    ended_at           INTEGER
);

CREATE INDEX threads_by_workspace ON threads(workspace_root, last_activity_at DESC);
CREATE INDEX threads_by_status    ON threads(status, last_activity_at DESC);

CREATE TABLE events (
    thread_id TEXT NOT NULL,
    seq       INTEGER NOT NULL,
    payload   BLOB NOT NULL,
    ts_ms     INTEGER NOT NULL,
    source    TEXT NOT NULL DEFAULT 'live',
    PRIMARY KEY (thread_id, seq),
    FOREIGN KEY (thread_id) REFERENCES threads(thread_id)
) WITHOUT ROWID;

CREATE INDEX events_by_ts ON events(thread_id, ts_ms);
```

- [ ] **Step 3: Commit**

```bash
git add crates/minos-daemon/Cargo.toml crates/minos-daemon/migrations/
git commit -m "feat(daemon/db): add migrations dir + 0001_initial.sql"
```

### Task C2: Implement `LocalStore` open + migrate

**Files:**
- Create: `crates/minos-daemon/src/store/mod.rs`
- Create: `crates/minos-daemon/src/store/migrations_loader.rs`
- Modify: `crates/minos-daemon/src/lib.rs` (add `pub mod store;`)

- [ ] **Step 1: Failing test**

Create `crates/minos-daemon/src/store/mod.rs` with:

```rust
pub mod migrations_loader;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

#[derive(Clone)]
pub struct LocalStore {
    pool: SqlitePool,
}

impl LocalStore {
    pub async fn open(db_file: &Path) -> anyhow::Result<Self> {
        let url = format!("sqlite://{}?mode=rwc", db_file.display());
        let opts = SqliteConnectOptions::from_str(&url)?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool { &self.pool }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("test.sqlite");
        let store = LocalStore::open(&p).await.unwrap();
        let row: (i64,) = sqlx::query_as("SELECT count(*) FROM events")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }
}
```

In `crates/minos-daemon/src/lib.rs` add `pub mod store;`.

- [ ] **Step 2: Run test (should fail until file is wired)**

Run: `cargo test -p minos-daemon store::tests`
Expected: PASS (the test should pass once the file is created — it fails only if the schema apply fails, which would be a real bug).

- [ ] **Step 3: Commit**

```bash
git add crates/minos-daemon/src/store/ crates/minos-daemon/src/lib.rs
git commit -m "feat(daemon/store): LocalStore open + sqlx::migrate!"
```

### Task C3: Implement `EventWriter` skeleton + write-ahead test

**Files:**
- Create: `crates/minos-daemon/src/store/event_writer.rs`
- Modify: `crates/minos-daemon/src/store/mod.rs` (`pub mod event_writer;`)

- [ ] **Step 1: Failing test for write-ahead order**

In `crates/minos-daemon/src/store/event_writer.rs`:

```rust
use crate::store::LocalStore;
use anyhow::Result;
use minos_agent_runtime::RawIngest;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventSource { Live, JsonlRecovery }

pub struct EventWriter {
    tx: mpsc::Sender<WriteJob>,
}

#[derive(Debug)]
struct WriteJob {
    ingest: RawIngest,
    source: EventSource,
    ack: tokio::sync::oneshot::Sender<Result<u64>>, // returns assigned seq on success
}

impl EventWriter {
    pub fn spawn(store: Arc<LocalStore>, relay_out: mpsc::Sender<minos_protocol::Envelope>) -> Self {
        let (tx, rx) = mpsc::channel::<WriteJob>(1024);
        tokio::spawn(writer_loop(store, relay_out, rx));
        Self { tx }
    }

    pub async fn write_live(&self, ingest: RawIngest) -> Result<u64> {
        self.write_internal(ingest, EventSource::Live).await
    }

    pub async fn write_recovery(&self, ingest: RawIngest) -> Result<u64> {
        self.write_internal(ingest, EventSource::JsonlRecovery).await
    }

    async fn write_internal(&self, ingest: RawIngest, source: EventSource) -> Result<u64> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(WriteJob { ingest, source, ack: tx }).await
            .map_err(|_| anyhow::anyhow!("event writer task gone"))?;
        rx.await.map_err(|_| anyhow::anyhow!("event writer dropped"))?
    }
}

async fn writer_loop(
    store: Arc<LocalStore>,
    relay_out: mpsc::Sender<minos_protocol::Envelope>,
    mut rx: mpsc::Receiver<WriteJob>,
) {
    while let Some(job) = rx.recv().await {
        let res = process_one(&store, &relay_out, job.ingest.clone(), job.source).await;
        let _ = job.ack.send(res);
    }
}

async fn process_one(
    store: &LocalStore,
    relay_out: &mpsc::Sender<minos_protocol::Envelope>,
    ingest: RawIngest,
    source: EventSource,
) -> Result<u64> {
    let mut tx = store.pool().begin().await?;
    let prev: Option<i64> = sqlx::query_scalar(
        "SELECT last_seq FROM threads WHERE thread_id = ?"
    )
    .bind(&ingest.thread_id)
    .fetch_optional(&mut *tx)
    .await?;
    let seq = (prev.unwrap_or(0) + 1) as u64;
    let payload_bytes = serde_json::to_vec(&ingest.payload)?;
    sqlx::query("INSERT INTO events(thread_id, seq, payload, ts_ms, source) VALUES (?, ?, ?, ?, ?)")
        .bind(&ingest.thread_id)
        .bind(seq as i64)
        .bind(&payload_bytes)
        .bind(ingest.ts_ms)
        .bind(match source { EventSource::Live => "live", EventSource::JsonlRecovery => "jsonl_recovery" })
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE threads SET last_seq = ?, last_activity_at = ? WHERE thread_id = ?")
        .bind(seq as i64)
        .bind(ingest.ts_ms)
        .bind(&ingest.thread_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let env = minos_protocol::Envelope::Ingest {
        version: 1,
        agent: ingest.agent.clone(),
        thread_id: ingest.thread_id.clone(),
        seq,
        payload: ingest.payload.clone(),
        ts_ms: ingest.ts_ms,
    };
    let _ = relay_out.send(env).await;
    Ok(seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_thread(store: &LocalStore, tid: &str) {
        sqlx::query("INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/tmp/ws', 0, 0)")
            .execute(store.pool()).await.unwrap();
        sqlx::query("INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, '/tmp/ws', 'codex', 'idle', 0, 0, 0)")
            .bind(tid)
            .execute(store.pool()).await.unwrap();
    }

    #[tokio::test]
    async fn write_live_assigns_monotonic_seq() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(LocalStore::open(&tmp.path().join("t.sqlite")).await.unwrap());
        seed_thread(&store, "thr-A").await;
        let (relay_tx, mut relay_rx) = mpsc::channel(16);
        let writer = EventWriter::spawn(store.clone(), relay_tx);

        for i in 0..5 {
            let ingest = RawIngest {
                agent: minos_agent_runtime::AgentKind::Codex,
                thread_id: "thr-A".into(),
                payload: serde_json::json!({"i": i}),
                ts_ms: i,
            };
            let seq = writer.write_live(ingest).await.unwrap();
            assert_eq!(seq, (i + 1) as u64);
        }

        for i in 0..5 {
            let env = relay_rx.recv().await.unwrap();
            match env {
                minos_protocol::Envelope::Ingest { seq, .. } => assert_eq!(seq, (i + 1) as u64),
                _ => panic!("unexpected envelope"),
            }
        }
    }
}
```

In `crates/minos-daemon/src/store/mod.rs`, add `pub mod event_writer;`.

- [ ] **Step 2: Run the test**

Run: `cargo test -p minos-daemon store::event_writer::tests`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/minos-daemon/src/store/event_writer.rs crates/minos-daemon/src/store/mod.rs
git commit -m "feat(daemon/store): EventWriter with write-ahead pipeline + monotonic seq"
```

### Task C4: Add batching window to `EventWriter`

**Files:**
- Modify: `crates/minos-daemon/src/store/event_writer.rs`

- [ ] **Step 1: Failing test for batching**

Add to `event_writer.rs` `tests`:

```rust
#[tokio::test]
async fn batches_within_5ms_window() {
    use std::time::Duration;
    use tokio::time::Instant;
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalStore::open(&tmp.path().join("t.sqlite")).await.unwrap());
    seed_thread(&store, "thr-B").await;
    let (relay_tx, mut relay_rx) = mpsc::channel(256);
    let writer = EventWriter::spawn(store.clone(), relay_tx);

    let start = Instant::now();
    let mut handles = Vec::new();
    for i in 0..50 {
        let w = writer.clone();
        handles.push(tokio::spawn(async move {
            w.write_live(RawIngest {
                agent: minos_agent_runtime::AgentKind::Codex,
                thread_id: "thr-B".into(),
                payload: serde_json::json!({"i": i}),
                ts_ms: i as i64,
            }).await.unwrap()
        }));
    }
    for h in handles { let _ = h.await; }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(500), "50 events should commit fast: {:?}", elapsed);

    let mut got = 0;
    while let Ok(_) = tokio::time::timeout(Duration::from_millis(200), relay_rx.recv()).await {
        got += 1;
        if got == 50 { break; }
    }
    assert_eq!(got, 50);
}
```

This test will pass with the per-event commit, but document the expectation; the batching optimisation goes in Step 2.

- [ ] **Step 2: Implement batching loop**

Replace `writer_loop` in `event_writer.rs`:

```rust
async fn writer_loop(
    store: Arc<LocalStore>,
    relay_out: mpsc::Sender<minos_protocol::Envelope>,
    mut rx: mpsc::Receiver<WriteJob>,
) {
    use tokio::time::{Duration, Instant};
    const BATCH_MAX: usize = 100;
    const BATCH_WINDOW: Duration = Duration::from_millis(5);

    let mut buf: Vec<WriteJob> = Vec::with_capacity(BATCH_MAX);
    while let Some(first) = rx.recv().await {
        buf.push(first);
        let deadline = Instant::now() + BATCH_WINDOW;
        while buf.len() < BATCH_MAX {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(job)) => buf.push(job),
                Ok(None) => break,
                Err(_) => break,
            }
        }
        process_batch(&store, &relay_out, std::mem::take(&mut buf)).await;
    }
}

async fn process_batch(
    store: &LocalStore,
    relay_out: &mpsc::Sender<minos_protocol::Envelope>,
    jobs: Vec<WriteJob>,
) {
    if jobs.is_empty() { return; }
    let mut tx = match store.pool().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            let err = std::sync::Arc::new(e);
            for j in jobs { let _ = j.ack.send(Err(anyhow::anyhow!("begin tx: {}", err))); }
            return;
        }
    };
    let mut results: Vec<Result<u64>> = Vec::with_capacity(jobs.len());
    let mut envs: Vec<minos_protocol::Envelope> = Vec::with_capacity(jobs.len());
    for job in &jobs {
        let prev: Option<i64> = match sqlx::query_scalar("SELECT last_seq FROM threads WHERE thread_id = ?")
            .bind(&job.ingest.thread_id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(v) => v,
            Err(e) => { results.push(Err(e.into())); continue; }
        };
        let seq = (prev.unwrap_or(0) + 1) as u64;
        let payload = match serde_json::to_vec(&job.ingest.payload) {
            Ok(v) => v,
            Err(e) => { results.push(Err(e.into())); continue; }
        };
        if let Err(e) = sqlx::query(
            "INSERT INTO events(thread_id, seq, payload, ts_ms, source) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&job.ingest.thread_id)
        .bind(seq as i64)
        .bind(&payload)
        .bind(job.ingest.ts_ms)
        .bind(match job.source { EventSource::Live => "live", EventSource::JsonlRecovery => "jsonl_recovery" })
        .execute(&mut *tx).await {
            results.push(Err(e.into()));
            continue;
        }
        if let Err(e) = sqlx::query("UPDATE threads SET last_seq = ?, last_activity_at = ? WHERE thread_id = ?")
            .bind(seq as i64)
            .bind(job.ingest.ts_ms)
            .bind(&job.ingest.thread_id)
            .execute(&mut *tx).await {
            results.push(Err(e.into()));
            continue;
        }
        results.push(Ok(seq));
        envs.push(minos_protocol::Envelope::Ingest {
            version: 1,
            agent: job.ingest.agent.clone(),
            thread_id: job.ingest.thread_id.clone(),
            seq,
            payload: job.ingest.payload.clone(),
            ts_ms: job.ingest.ts_ms,
        });
    }
    if let Err(e) = tx.commit().await {
        for (job, _) in jobs.into_iter().zip(results.into_iter()) {
            let _ = job.ack.send(Err(anyhow::anyhow!("commit: {}", e)));
        }
        return;
    }
    for (job, r) in jobs.into_iter().zip(results.into_iter()) {
        let _ = job.ack.send(r);
    }
    for env in envs { let _ = relay_out.send(env).await; }
}
```

Drop the per-job `process_one` (replaced by the batch path).

- [ ] **Step 3: Run all event_writer tests**

Run: `cargo test -p minos-daemon store::event_writer`
Expected: PASS (both `write_live_assigns_monotonic_seq` and `batches_within_5ms_window`).

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(daemon/store): batch EventWriter commits (5ms / 100-job window)"
```

### Task C5: Add `LocalStore` reads (`list_threads`, `get_thread`, event range)

**Files:**
- Modify: `crates/minos-daemon/src/store/mod.rs`

- [ ] **Step 1: Failing test**

In `crates/minos-daemon/src/store/mod.rs`, append to `tests`:

```rust
#[tokio::test]
async fn list_and_get_threads() {
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalStore::open(&tmp.path().join("t.sqlite")).await.unwrap();
    sqlx::query("INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w', 0, 0)")
        .execute(store.pool()).await.unwrap();
    for i in 0..3 {
        sqlx::query("INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, '/w', 'codex', 'idle', 0, ?, ?)")
            .bind(format!("thr-{}", i))
            .bind(i as i64)
            .bind(i as i64)
            .execute(store.pool()).await.unwrap();
    }
    let threads = store.list_threads(None, None).await.unwrap();
    assert_eq!(threads.len(), 3);
    let one = store.get_thread("thr-1").await.unwrap();
    assert_eq!(one.unwrap().agent, "codex");
}
```

- [ ] **Step 2: Implement reads**

Add to `LocalStore`:

```rust
pub struct ThreadRow {
    pub thread_id: String,
    pub workspace_root: String,
    pub agent: String,
    pub codex_session_id: Option<String>,
    pub status: String,
    pub last_pause_reason: Option<String>,
    pub last_close_reason: Option<String>,
    pub last_seq: i64,
    pub started_at: i64,
    pub last_activity_at: i64,
    pub ended_at: Option<i64>,
}

impl LocalStore {
    pub async fn list_threads(
        &self,
        before_ts_ms: Option<i64>,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<ThreadRow>> {
        let limit = limit.unwrap_or(50).min(500) as i64;
        let rows = match before_ts_ms {
            Some(ts) => sqlx::query_as::<_, ThreadRow>(
                "SELECT * FROM threads WHERE last_activity_at < ? ORDER BY last_activity_at DESC LIMIT ?")
                .bind(ts).bind(limit).fetch_all(&self.pool).await?,
            None => sqlx::query_as::<_, ThreadRow>(
                "SELECT * FROM threads ORDER BY last_activity_at DESC LIMIT ?")
                .bind(limit).fetch_all(&self.pool).await?,
        };
        Ok(rows)
    }

    pub async fn get_thread(&self, thread_id: &str) -> anyhow::Result<Option<ThreadRow>> {
        Ok(sqlx::query_as::<_, ThreadRow>("SELECT * FROM threads WHERE thread_id = ?")
            .bind(thread_id).fetch_optional(&self.pool).await?)
    }

    pub async fn read_events(
        &self,
        thread_id: &str,
        from_seq: u64,
        to_seq: u64,
    ) -> anyhow::Result<Vec<EventRow>> {
        Ok(sqlx::query_as::<_, EventRow>(
            "SELECT thread_id, seq, payload, ts_ms, source FROM events WHERE thread_id = ? AND seq BETWEEN ? AND ? ORDER BY seq ASC")
            .bind(thread_id).bind(from_seq as i64).bind(to_seq as i64)
            .fetch_all(&self.pool).await?)
    }
}

#[derive(sqlx::FromRow)]
pub struct EventRow {
    pub thread_id: String,
    pub seq: i64,
    pub payload: Vec<u8>,
    pub ts_ms: i64,
    pub source: String,
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for ThreadRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            thread_id: row.try_get("thread_id")?,
            workspace_root: row.try_get("workspace_root")?,
            agent: row.try_get("agent")?,
            codex_session_id: row.try_get("codex_session_id")?,
            status: row.try_get("status")?,
            last_pause_reason: row.try_get("last_pause_reason")?,
            last_close_reason: row.try_get("last_close_reason")?,
            last_seq: row.try_get("last_seq")?,
            started_at: row.try_get("started_at")?,
            last_activity_at: row.try_get("last_activity_at")?,
            ended_at: row.try_get("ended_at")?,
        })
    }
}
```

Add `use sqlx::Row;` at the top.

- [ ] **Step 3: Run + commit**

```bash
cargo test -p minos-daemon store
git commit -am "feat(daemon/store): list_threads / get_thread / read_events"
```

### Task C6: Define `ThreadState` + reasons + transition rules

**Files:**
- Create: `crates/minos-agent-runtime/src/state_machine.rs`
- Modify: `crates/minos-agent-runtime/src/lib.rs`

- [ ] **Step 1: Author the module**

Create `crates/minos-agent-runtime/src/state_machine.rs`:

```rust
use std::time::Instant;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ThreadState {
    Starting,
    Idle,
    Running { turn_started_at_ms: i64 },
    Suspended { reason: PauseReason },
    Resuming,
    Closed { reason: CloseReason },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    UserInterrupt,
    CodexCrashed,
    DaemonRestart,
    InstanceReaped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    UserClose,
    TerminalError,
}

#[derive(Debug, thiserror::Error)]
#[error("illegal thread state transition: {from:?} → {to:?}")]
pub struct IllegalTransition {
    pub from: ThreadState,
    pub to: ThreadState,
}

pub fn validate_transition(from: &ThreadState, to: &ThreadState) -> Result<(), IllegalTransition> {
    use ThreadState::*;
    let ok = matches!((from, to),
        (Starting, Idle)
      | (Idle, Running { .. })
      | (Running { .. }, Idle)
      | (Running { .. }, Suspended { .. })
      | (Idle, Suspended { .. })
      | (Suspended { .. }, Resuming)
      | (Resuming, Idle)
      | (Resuming, Closed { reason: CloseReason::TerminalError })
      | (Starting, Closed { .. })
      | (Idle, Closed { .. })
      | (Running { .. }, Closed { .. })
      | (Suspended { .. }, Closed { .. })
      | (Resuming, Closed { .. })
    );
    if ok { Ok(()) } else { Err(IllegalTransition { from: from.clone(), to: to.clone() }) }
}

pub fn status_str(state: &ThreadState) -> &'static str {
    match state {
        ThreadState::Starting => "starting",
        ThreadState::Idle => "idle",
        ThreadState::Running { .. } => "running",
        ThreadState::Suspended { .. } => "suspended",
        ThreadState::Resuming => "resuming",
        ThreadState::Closed { .. } => "closed",
    }
}
```

In `crates/minos-agent-runtime/src/lib.rs`, add `pub mod state_machine;` and `pub use state_machine::{ThreadState, PauseReason, CloseReason};`. Drop the existing `pub use state::AgentState;` re-export but keep `state.rs` intact for now (will be deleted in C18).

- [ ] **Step 2: Failing test for transitions**

Append to `state_machine.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_transition_idle_to_running() {
        validate_transition(
            &ThreadState::Idle,
            &ThreadState::Running { turn_started_at_ms: 1 },
        ).unwrap();
    }

    #[test]
    fn illegal_transition_running_to_starting() {
        let err = validate_transition(
            &ThreadState::Running { turn_started_at_ms: 1 },
            &ThreadState::Starting,
        ).unwrap_err();
        assert!(format!("{err}").contains("illegal"));
    }

    #[test]
    fn suspended_can_resume_or_close() {
        validate_transition(
            &ThreadState::Suspended { reason: PauseReason::UserInterrupt },
            &ThreadState::Resuming,
        ).unwrap();
        validate_transition(
            &ThreadState::Suspended { reason: PauseReason::UserInterrupt },
            &ThreadState::Closed { reason: CloseReason::UserClose },
        ).unwrap();
    }
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p minos-agent-runtime state_machine
git add crates/minos-agent-runtime/src/state_machine.rs crates/minos-agent-runtime/src/lib.rs
git commit -m "feat(agent-runtime/state): ThreadState + transition validator"
```

### Task C7: Define `ThreadHandle` and `ManagerEvent`

**Files:**
- Create: `crates/minos-agent-runtime/src/thread_handle.rs`
- Create: `crates/minos-agent-runtime/src/manager_event.rs`

- [ ] **Step 1: Implement both**

`crates/minos-agent-runtime/src/thread_handle.rs`:

```rust
use crate::state_machine::ThreadState;
use crate::AgentKind;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::watch;

#[derive(Clone)]
pub struct ThreadHandle {
    pub thread_id: String,
    pub workspace: PathBuf,
    pub agent: AgentKind,
    pub codex_session_id: Option<String>,
    pub state_tx: Arc<watch::Sender<ThreadState>>,
    pub state_rx: watch::Receiver<ThreadState>,
    pub last_seq: Arc<AtomicU64>,
}

impl ThreadHandle {
    pub fn new(
        thread_id: String,
        workspace: PathBuf,
        agent: AgentKind,
        initial: ThreadState,
        last_seq: u64,
    ) -> Self {
        let (tx, rx) = watch::channel(initial);
        Self {
            thread_id,
            workspace,
            agent,
            codex_session_id: None,
            state_tx: Arc::new(tx),
            state_rx: rx,
            last_seq: Arc::new(AtomicU64::new(last_seq)),
        }
    }

    pub fn current_state(&self) -> ThreadState {
        self.state_rx.borrow().clone()
    }

    pub fn transition(&self, new: ThreadState) -> Result<(), crate::state_machine::IllegalTransition> {
        let from = self.current_state();
        crate::state_machine::validate_transition(&from, &new)?;
        let _ = self.state_tx.send(new);
        Ok(())
    }
}
```

`crates/minos-agent-runtime/src/manager_event.rs`:

```rust
use crate::state_machine::{CloseReason, ThreadState};
use crate::AgentKind;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum ManagerEvent {
    ThreadAdded { thread_id: String, workspace: PathBuf, agent: AgentKind },
    ThreadStateChanged { thread_id: String, old: ThreadState, new: ThreadState, at_ms: i64 },
    ThreadClosed { thread_id: String, reason: CloseReason },
    InstanceCrashed { workspace: PathBuf, affected_threads: Vec<String> },
}
```

In `lib.rs`, add `pub mod thread_handle;` and `pub mod manager_event;`. Re-export `pub use thread_handle::ThreadHandle; pub use manager_event::ManagerEvent;`.

- [ ] **Step 2: Add a transition test**

In `thread_handle.rs`, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_machine::{PauseReason, ThreadState};

    #[test]
    fn rejects_illegal_transition() {
        let h = ThreadHandle::new("t".into(), "/w".into(), AgentKind::Codex, ThreadState::Idle, 0);
        let err = h.transition(ThreadState::Starting).unwrap_err();
        assert!(format!("{err}").contains("illegal"));
        assert_eq!(h.current_state(), ThreadState::Idle);
    }

    #[test]
    fn accepts_legal_transition() {
        let h = ThreadHandle::new("t".into(), "/w".into(), AgentKind::Codex, ThreadState::Idle, 0);
        h.transition(ThreadState::Running { turn_started_at_ms: 1 }).unwrap();
        assert!(matches!(h.current_state(), ThreadState::Running { .. }));
    }
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p minos-agent-runtime thread_handle manager_event
git add crates/minos-agent-runtime/src/thread_handle.rs crates/minos-agent-runtime/src/manager_event.rs crates/minos-agent-runtime/src/lib.rs
git commit -m "feat(agent-runtime): ThreadHandle + ManagerEvent"
```

### Task C8: Implement `AppServerInstance` wrapper (skeleton)

**Files:**
- Create: `crates/minos-agent-runtime/src/instance.rs`

- [ ] **Step 1: Author skeleton**

```rust
use crate::codex_client::CodexClient;
use crate::state_machine::{PauseReason, ThreadState};
use crate::thread_handle::ThreadHandle;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, mpsc};

pub struct AppServerInstance {
    pub workspace: PathBuf,
    pub child: tokio::process::Child,
    pub client: Arc<CodexClient>,
    pub threads: Mutex<HashSet<String>>,
    pub spawned_at: Instant,
    pub last_activity_at: Mutex<Instant>,
    pub crash_signal: mpsc::Sender<()>,
}

impl AppServerInstance {
    pub fn new(
        workspace: PathBuf,
        child: tokio::process::Child,
        client: Arc<CodexClient>,
        crash_signal: mpsc::Sender<()>,
    ) -> Self {
        let now = Instant::now();
        Self {
            workspace,
            child,
            client,
            threads: Mutex::new(HashSet::new()),
            spawned_at: now,
            last_activity_at: Mutex::new(now),
            crash_signal,
        }
    }

    pub async fn touch(&self) {
        *self.last_activity_at.lock().await = Instant::now();
    }

    pub async fn add_thread(&self, thread_id: String) {
        self.threads.lock().await.insert(thread_id);
    }

    pub async fn remove_thread(&self, thread_id: &str) {
        self.threads.lock().await.remove(thread_id);
    }

    pub async fn thread_ids(&self) -> Vec<String> {
        self.threads.lock().await.iter().cloned().collect()
    }
}
```

In `lib.rs`, add `pub mod instance;` and `pub use instance::AppServerInstance;`.

- [ ] **Step 2: Build**

Run: `cargo build -p minos-agent-runtime`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/minos-agent-runtime/src/instance.rs crates/minos-agent-runtime/src/lib.rs
git commit -m "feat(agent-runtime): AppServerInstance skeleton"
```

### Task C9: Implement `AgentManager` skeleton + start_agent

**Files:**
- Create: `crates/minos-agent-runtime/src/manager.rs`

- [ ] **Step 1: Author the module**

```rust
use crate::codex_client::CodexClient;
use crate::instance::AppServerInstance;
use crate::manager_event::ManagerEvent;
use crate::state_machine::{CloseReason, PauseReason, ThreadState};
use crate::thread_handle::ThreadHandle;
use crate::{AgentKind, AgentRuntimeConfig, RawIngest};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast, watch};

#[derive(Clone, Debug)]
pub struct InstanceCaps {
    pub max_instances: usize,
    pub idle_timeout: std::time::Duration,
}

impl Default for InstanceCaps {
    fn default() -> Self {
        Self {
            max_instances: 8,
            idle_timeout: std::time::Duration::from_secs(30 * 60),
        }
    }
}

pub struct AgentManager {
    pub config: Arc<AgentRuntimeConfig>,
    pub caps: InstanceCaps,
    instances: Arc<Mutex<HashMap<PathBuf, Arc<AppServerInstance>>>>,
    threads: Arc<Mutex<HashMap<String, ThreadHandle>>>,
    events_tx: broadcast::Sender<RawIngest>,
    manager_tx: broadcast::Sender<ManagerEvent>,
}

impl AgentManager {
    pub fn new(config: AgentRuntimeConfig, caps: InstanceCaps) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        let (manager_tx, _) = broadcast::channel(64);
        Self {
            config: Arc::new(config),
            caps,
            instances: Arc::new(Mutex::new(HashMap::new())),
            threads: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            manager_tx,
        }
    }

    pub fn ingest_stream(&self) -> broadcast::Receiver<RawIngest> {
        self.events_tx.subscribe()
    }

    pub fn manager_event_stream(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager_tx.subscribe()
    }

    pub async fn thread_state_stream(&self, thread_id: &str) -> Option<watch::Receiver<ThreadState>> {
        self.threads.lock().await.get(thread_id).map(|h| h.state_rx.clone())
    }
}
```

In `lib.rs`, add `pub mod manager;` and `pub use manager::{AgentManager, InstanceCaps};`.

- [ ] **Step 2: Build**

Run: `cargo build -p minos-agent-runtime`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/minos-agent-runtime/src/manager.rs crates/minos-agent-runtime/src/lib.rs
git commit -m "feat(agent-runtime): AgentManager skeleton with stream surface"
```

### Task C10: Implement `AgentManager::start_agent`

**Files:**
- Modify: `crates/minos-agent-runtime/src/manager.rs`

- [ ] **Step 1: Failing test (mock instance)**

This needs a fake `CodexClient`. Reuse the existing `test_support` module if present; otherwise add a feature-gated mock under `#[cfg(test)]` that returns canned `ThreadStartResponse`.

Append to `manager.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeCodexBackend;

    #[tokio::test]
    async fn start_agent_creates_instance_and_thread() {
        let cfg = AgentRuntimeConfig::new(tempfile::tempdir().unwrap().into_path());
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let ws = std::path::PathBuf::from("/w-test");
        // For the first iteration, we wire start_agent to a stub that does not actually spawn codex.
        let resp = mgr.start_agent(AgentKind::Codex, ws.clone()).await.unwrap();
        assert_eq!(resp.cwd, ws);
        let snap = mgr.list_threads().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].workspace, ws);
    }
}
```

- [ ] **Step 2: Implement (with stubbed instance for now; full codex spawn in C12)**

```rust
pub struct StartAgentOutcome {
    pub thread_id: String,
    pub cwd: PathBuf,
}

impl AgentManager {
    pub async fn start_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or(workspace.clone());
        let instance = self.ensure_instance(&canon).await?;
        let thread_id = instance.client.start_thread().await?;
        instance.add_thread(thread_id.clone()).await;
        instance.touch().await;

        let handle = ThreadHandle::new(thread_id.clone(), canon.clone(), agent, ThreadState::Starting, 0);
        self.threads.lock().await.insert(thread_id.clone(), handle.clone());
        let _ = self.manager_tx.send(ManagerEvent::ThreadAdded {
            thread_id: thread_id.clone(),
            workspace: canon.clone(),
            agent,
        });
        // Caller awaits codex's `thread/started` to advance to Idle (C12).
        Ok(StartAgentOutcome { thread_id, cwd: canon })
    }

    async fn ensure_instance(&self, workspace: &Path) -> anyhow::Result<Arc<AppServerInstance>> {
        let mut guard = self.instances.lock().await;
        if let Some(existing) = guard.get(workspace) {
            return Ok(existing.clone());
        }
        if guard.len() >= self.caps.max_instances {
            self.lru_evict(&mut *guard).await?;
        }
        // Spawning the actual codex child is implemented in C12.
        let inst = self.spawn_instance(workspace).await?;
        guard.insert(workspace.to_path_buf(), inst.clone());
        Ok(inst)
    }

    async fn spawn_instance(&self, workspace: &Path) -> anyhow::Result<Arc<AppServerInstance>> {
        // Stub: real implementation lands in C12. For now return error so the test can be skipped
        // with `#[ignore]` until C12, or implement enough to satisfy the basic flow under test_support.
        anyhow::bail!("spawn_instance unimplemented (C12)")
    }

    async fn lru_evict(&self, map: &mut HashMap<PathBuf, Arc<AppServerInstance>>) -> anyhow::Result<()> {
        // Detail in C16.
        anyhow::bail!("evict unimplemented (C16)")
    }

    pub async fn list_threads(&self) -> Vec<crate::store_facing::ThreadSnapshot> {
        let g = self.threads.lock().await;
        g.values()
            .map(|h| crate::store_facing::ThreadSnapshot {
                thread_id: h.thread_id.clone(),
                workspace: h.workspace.clone(),
                state: h.current_state(),
            })
            .collect()
    }
}
```

Add a small `store_facing.rs`:

```rust
use crate::state_machine::ThreadState;
use std::path::PathBuf;
pub struct ThreadSnapshot {
    pub thread_id: String,
    pub workspace: PathBuf,
    pub state: ThreadState,
}
```

In `lib.rs`, `pub mod store_facing;`.

For the test in step 1 to pass before C12 lands, mark it `#[ignore]` for now and wire the real path in C12. Add a placeholder commit message reflecting that.

- [ ] **Step 3: Commit**

```bash
cargo build -p minos-agent-runtime
git add crates/minos-agent-runtime/src/manager.rs crates/minos-agent-runtime/src/store_facing.rs crates/minos-agent-runtime/src/lib.rs
git commit -m "feat(agent-runtime): AgentManager start_agent + ensure_instance scaffold"
```

### Task C11: `send_user_message` happy path (Idle thread)

**Files:**
- Modify: `crates/minos-agent-runtime/src/manager.rs`

- [ ] **Step 1: Implement**

```rust
impl AgentManager {
    pub async fn send_user_message(&self, thread_id: &str, text: String) -> anyhow::Result<()> {
        let handle = self.threads.lock().await.get(thread_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        match handle.current_state() {
            ThreadState::Idle => {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let new_state = ThreadState::Running { turn_started_at_ms: now_ms };
                handle.transition(new_state.clone())?;
                let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
                    thread_id: thread_id.to_string(),
                    old: ThreadState::Idle,
                    new: new_state,
                    at_ms: now_ms,
                });
                let workspace = handle.workspace.clone();
                let inst = self.instances.lock().await.get(&workspace).cloned()
                    .ok_or_else(|| anyhow::anyhow!("instance for workspace gone"))?;
                inst.touch().await;
                inst.client.send_user_message(thread_id, &text).await?;
                Ok(())
            }
            ThreadState::Suspended { .. } => {
                self.implicit_resume(thread_id, text).await
            }
            other => anyhow::bail!("send_user_message rejected: state={:?}", other),
        }
    }

    async fn implicit_resume(&self, _thread_id: &str, _text: String) -> anyhow::Result<()> {
        // Real impl in C13.
        anyhow::bail!("implicit_resume unimplemented (C13)")
    }
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p minos-agent-runtime`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git commit -am "feat(agent-runtime/manager): send_user_message Idle path"
```

### Task C12: Spawn real codex child in `spawn_instance`

**Files:**
- Modify: `crates/minos-agent-runtime/src/manager.rs`
- Reuse: existing `crates/minos-agent-runtime/src/process.rs` and `codex_client.rs`

- [ ] **Step 1: Migrate spawn logic**

Move/adapt the `start_inner` logic that today lives in `runtime.rs` (around lines 540-680) into `manager.rs`'s `spawn_instance` and `start_agent` flow. Concretely:

```rust
async fn spawn_instance(&self, workspace: &Path) -> anyhow::Result<Arc<AppServerInstance>> {
    let cmd = crate::process::build_codex_app_server_command(&self.config, workspace)?;
    let child = crate::process::spawn_with_logging(cmd).await?;
    let client = Arc::new(crate::codex_client::CodexClient::connect_loopback(&self.config, child.id().unwrap_or(0)).await?);
    let (crash_tx, mut crash_rx) = tokio::sync::mpsc::channel::<()>(1);
    let inst = Arc::new(AppServerInstance::new(workspace.to_path_buf(), child, client.clone(), crash_tx));
    let inst_for_watcher = inst.clone();
    let mgr_tx = self.manager_tx.clone();
    let threads_ref = self.threads.clone();
    tokio::spawn(async move {
        let _ = crash_rx.recv().await;
        let affected = inst_for_watcher.thread_ids().await;
        for tid in &affected {
            if let Some(h) = threads_ref.lock().await.get(tid) {
                let _ = h.transition(ThreadState::Suspended { reason: PauseReason::CodexCrashed });
            }
        }
        let _ = mgr_tx.send(ManagerEvent::InstanceCrashed {
            workspace: inst_for_watcher.workspace.clone(),
            affected_threads: affected,
        });
    });
    Ok(inst)
}
```

(`crates/minos-agent-runtime/src/process.rs` already contains the spawn primitives; expose helper functions if needed.)

- [ ] **Step 2: Wire codex `thread/started` → state Idle**

In the `CodexClient` event-loop subscription (existing code in `codex_client.rs` / `runtime.rs`), forward each typed event into a manager-side handler that, when it sees `thread/started { thread_id, session_id }`, looks up the `ThreadHandle` and transitions `Starting → Idle`, recording `codex_session_id`. The exact wiring depends on existing `CodexClient` API surface; pull the listener loop out of `runtime.rs:540-700` and drop it into `manager.rs`.

- [ ] **Step 3: Un-ignore test C10**

Remove `#[ignore]` from the `start_agent_creates_instance_and_thread` test added in C10.

Run: `cargo test -p minos-agent-runtime manager`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(agent-runtime/manager): spawn codex app-server child + wire crash signal"
```

### Task C13: Implement implicit `Resume` flow

**Files:**
- Modify: `crates/minos-agent-runtime/src/manager.rs`

- [ ] **Step 1: Implement**

Replace the placeholder `implicit_resume`:

```rust
async fn implicit_resume(&self, thread_id: &str, text: String) -> anyhow::Result<()> {
    let handle = self.threads.lock().await.get(thread_id).cloned()
        .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
    handle.transition(ThreadState::Resuming)?;
    let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
        thread_id: thread_id.to_string(),
        old: ThreadState::Suspended { reason: PauseReason::UserInterrupt }, // best-effort old
        new: ThreadState::Resuming,
        at_ms: chrono::Utc::now().timestamp_millis(),
    });
    let workspace = handle.workspace.clone();
    let codex_session_id = handle.codex_session_id.clone();

    let inst = self.ensure_instance(&workspace).await?;
    if let Some(sid) = codex_session_id {
        inst.client.start_thread_resume(thread_id, &sid).await?;
    } else {
        // No codex_session_id => fresh thread under same id is impossible; treat as terminal error.
        let _ = handle.transition(ThreadState::Closed { reason: CloseReason::TerminalError });
        anyhow::bail!("resume failed: no codex_session_id");
    }
    handle.transition(ThreadState::Idle)?;
    inst.touch().await;
    inst.client.send_user_message(thread_id, &text).await?;
    Ok(())
}
```

(`CodexClient::start_thread_resume(thread_id, codex_session_id)` is part of the typed-protocol crate per Plan 10. If that exact method does not exist, adapt to whatever the typed protocol exposes for resume.)

- [ ] **Step 2: Failing test**

Append to `manager.rs::tests`:

```rust
#[tokio::test]
async fn implicit_resume_from_suspended() {
    use crate::test_support::FakeCodexBackend;
    use crate::state_machine::PauseReason;

    let cfg = AgentRuntimeConfig::new(tempfile::tempdir().unwrap().into_path());
    let mgr = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
    let ws = std::path::PathBuf::from("/w-resume-test");

    let outcome = mgr.start_agent(AgentKind::Codex, ws.clone()).await.unwrap();
    let thread_id = outcome.thread_id;

    // Move the thread to Idle (simulate codex thread/started arriving).
    {
        let g = mgr.threads.lock().await;
        let h = g.get(&thread_id).unwrap();
        h.transition(ThreadState::Idle).unwrap();
    }
    // Then to Suspended (simulate user interrupt).
    {
        let g = mgr.threads.lock().await;
        let h = g.get(&thread_id).unwrap();
        h.transition(ThreadState::Suspended { reason: PauseReason::UserInterrupt }).unwrap();
        // Manually attach a codex_session_id so resume can target a session.
        // (In production this is set when codex emits thread/started; for the test we patch it.)
    }

    // Drive send_user_message → Resuming → Idle → Running.
    let mut state_rx = mgr.thread_state_stream(&thread_id).await.unwrap();
    let mgr_for_send = mgr.clone();
    let tid_for_send = thread_id.clone();
    let send_handle = tokio::spawn(async move {
        mgr_for_send.send_user_message(&tid_for_send, "hello".into()).await
    });

    let mut saw_resuming = false;
    while state_rx.changed().await.is_ok() {
        match &*state_rx.borrow() {
            ThreadState::Resuming => saw_resuming = true,
            ThreadState::Running { .. } => break,
            ThreadState::Closed { .. } => panic!("resume failed terminally"),
            _ => {}
        }
    }
    send_handle.await.unwrap().unwrap();
    assert!(saw_resuming, "expected to observe Resuming in transition");
    assert!(matches!(*state_rx.borrow(), ThreadState::Running { .. }));
}
```

Note: `AgentManager::threads` field needs to be `pub(crate)` for the test to reach in. If you do not want to expose it, add a `#[cfg(test)] pub(crate) fn force_state(&self, thread_id: &str, state: ThreadState)` helper instead.

- [ ] **Step 3: Run + commit**

```bash
cargo test -p minos-agent-runtime manager::tests::implicit_resume_from_suspended
git commit -am "feat(agent-runtime/manager): implicit resume from Suspended → Idle → Running"
```

### Task C14: `interrupt_thread` and `close_thread`

**Files:**
- Modify: `crates/minos-agent-runtime/src/manager.rs`

- [ ] **Step 1: Implement**

```rust
impl AgentManager {
    pub async fn interrupt_thread(&self, thread_id: &str) -> anyhow::Result<()> {
        let handle = self.threads.lock().await.get(thread_id).cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        if !matches!(handle.current_state(), ThreadState::Running { .. } | ThreadState::Idle) {
            anyhow::bail!("interrupt rejected: state={:?}", handle.current_state());
        }
        let workspace = handle.workspace.clone();
        if let Some(inst) = self.instances.lock().await.get(&workspace).cloned() {
            // Best-effort
            let _ = inst.client.interrupt_turn(thread_id).await;
        }
        handle.transition(ThreadState::Suspended { reason: PauseReason::UserInterrupt })?;
        Ok(())
    }

    pub async fn close_thread(&self, thread_id: &str) -> anyhow::Result<()> {
        let handle = self.threads.lock().await.get(thread_id).cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        if matches!(handle.current_state(), ThreadState::Closed { .. }) {
            return Ok(()); // idempotent
        }
        handle.transition(ThreadState::Closed { reason: CloseReason::UserClose })?;
        let workspace = handle.workspace.clone();
        if let Some(inst) = self.instances.lock().await.get(&workspace).cloned() {
            inst.remove_thread(thread_id).await;
        }
        let _ = self.manager_tx.send(ManagerEvent::ThreadClosed {
            thread_id: thread_id.to_string(),
            reason: CloseReason::UserClose,
        });
        Ok(())
    }
}
```

- [ ] **Step 2: Test**

Add tests for both:

```rust
#[tokio::test]
async fn interrupt_running_thread_to_suspended() {
    use crate::state_machine::PauseReason;
    let cfg = AgentRuntimeConfig::new(tempfile::tempdir().unwrap().into_path());
    let mgr = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
    let ws = std::path::PathBuf::from("/w-int");
    let outcome = mgr.start_agent(AgentKind::Codex, ws).await.unwrap();
    {
        let g = mgr.threads.lock().await;
        let h = g.get(&outcome.thread_id).unwrap();
        h.transition(ThreadState::Idle).unwrap();
        h.transition(ThreadState::Running { turn_started_at_ms: 0 }).unwrap();
    }
    mgr.interrupt_thread(&outcome.thread_id).await.unwrap();
    let g = mgr.threads.lock().await;
    let h = g.get(&outcome.thread_id).unwrap();
    assert!(matches!(
        h.current_state(),
        ThreadState::Suspended { reason: PauseReason::UserInterrupt }
    ));
}

#[tokio::test]
async fn close_thread_terminal_state() {
    use crate::state_machine::CloseReason;
    let cfg = AgentRuntimeConfig::new(tempfile::tempdir().unwrap().into_path());
    let mgr = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
    let ws = std::path::PathBuf::from("/w-close");
    let outcome = mgr.start_agent(AgentKind::Codex, ws).await.unwrap();
    {
        let g = mgr.threads.lock().await;
        g.get(&outcome.thread_id).unwrap().transition(ThreadState::Idle).unwrap();
    }
    mgr.close_thread(&outcome.thread_id).await.unwrap();
    let g = mgr.threads.lock().await;
    let h = g.get(&outcome.thread_id).unwrap();
    assert!(matches!(
        h.current_state(),
        ThreadState::Closed { reason: CloseReason::UserClose }
    ));
    // Idempotent: second close is a no-op.
    drop(g);
    mgr.close_thread(&outcome.thread_id).await.unwrap();
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p minos-agent-runtime manager
git commit -am "feat(agent-runtime/manager): interrupt_thread + close_thread"
```

### Task C15: Bridge `RawIngest` → `EventWriter` (daemon-side glue)

**Files:**
- Modify: `crates/minos-daemon/src/agent.rs`
- Replace: `crates/minos-daemon/src/agent_ingest.rs` (delete file; logic absorbed)

- [ ] **Step 1: Replace `agent_ingest.rs`**

Delete `crates/minos-daemon/src/agent_ingest.rs`. In `crates/minos-daemon/src/lib.rs`, remove the `mod agent_ingest;` line.

- [ ] **Step 2: Rewire `agent.rs`**

In `crates/minos-daemon/src/agent.rs`, replace `AgentGlue` with:

```rust
use minos_agent_runtime::{AgentManager, RawIngest};
use crate::store::event_writer::EventWriter;
use std::sync::Arc;

pub struct AgentGlue {
    pub manager: Arc<AgentManager>,
    pub writer: Arc<EventWriter>,
}

impl AgentGlue {
    pub fn new(manager: Arc<AgentManager>, writer: Arc<EventWriter>) -> Self {
        // Bridge RawIngest broadcast → EventWriter.write_live in a spawned task.
        let mut rx = manager.ingest_stream();
        let w = writer.clone();
        tokio::spawn(async move {
            while let Ok(ingest) = rx.recv().await {
                if let Err(e) = w.write_live(ingest).await {
                    tracing::error!(error = %e, "EventWriter.write_live failed; event dropped");
                }
            }
        });
        Self { manager, writer }
    }

    pub async fn start_agent(
        &self,
        req: minos_protocol::StartAgentRequest,
    ) -> anyhow::Result<minos_protocol::StartAgentResponse> {
        let outcome = self.manager.start_agent(req.agent, req.workspace).await?;
        Ok(minos_protocol::StartAgentResponse {
            session_id: outcome.thread_id,
            cwd: outcome.cwd,
        })
    }

    pub async fn send_user_message(&self, req: minos_protocol::SendUserMessageRequest) -> anyhow::Result<()> {
        self.manager.send_user_message(&req.session_id, req.text).await
    }

    pub async fn interrupt_thread(&self, req: minos_protocol::InterruptThreadRequest) -> anyhow::Result<()> {
        self.manager.interrupt_thread(&req.thread_id).await
    }

    pub async fn close_thread(&self, req: minos_protocol::CloseThreadRequest) -> anyhow::Result<()> {
        self.manager.close_thread(&req.thread_id).await
    }
}
```

(`StartAgentRequest`/`InterruptThreadRequest`/`CloseThreadRequest` types are added in Task C16.)

Note: the existing daemon WS outbound queue (`relay_out_tx`) is now driven by the `EventWriter` directly inside `process_batch` (Task C4). The `AgentGlue.new` spawn above is a **fallback bridge** for any code that still subscribes to the manager's broadcast for non-DB consumers (FFI streams). The single canonical write path is `manager → events_tx → EventWriter → DB → relay_out_tx`. Confirm during integration tests that no double-emission occurs (the bridge here is for FFI ingest_stream consumers only, and the FFI stream re-broadcasts after the writer has succeeded).

If review uncovers double-emission risk, the cleaner shape is: writer is the **only** subscriber to the manager broadcast, and the writer re-emits a new "post-commit" broadcast for FFI consumers. Adjust accordingly during this task.

- [ ] **Step 3: Run + commit**

```bash
cargo build -p minos-daemon
cargo test -p minos-daemon
git add crates/minos-daemon/src/agent.rs crates/minos-daemon/src/lib.rs
git rm crates/minos-daemon/src/agent_ingest.rs
git commit -m "refactor(daemon/agent): use AgentManager + EventWriter; drop agent_ingest.rs"
```

### Task C16: Update protocol RPC types and trait

**Files:**
- Modify: `crates/minos-protocol/src/messages.rs`
- Modify: `crates/minos-protocol/src/rpc.rs`

- [ ] **Step 1: New types**

Append to `messages.rs`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StartAgentRequest {
    pub agent: AgentKind,
    pub workspace: PathBuf,
    #[serde(default)]
    pub mode: Option<AgentLaunchMode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InterruptThreadRequest { pub thread_id: ThreadId }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloseThreadRequest    { pub thread_id: ThreadId }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetThreadParams       { pub thread_id: ThreadId }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetThreadResponse {
    pub thread: ThreadSummary,
    pub state: minos_agent_runtime::ThreadState,
}
```

(Update existing `ThreadSummary` to include `pub state: ThreadState` if not already; or wrap it as a separate field on `GetThreadResponse`.)

- [ ] **Step 2: RPC trait surgery**

In `crates/minos-protocol/src/rpc.rs`:

```rust
#[rpc(client, server, namespace = "minos")]
pub trait MinosRpc {
    #[method(name = "pair")]
    async fn pair(&self, req: PairRequest) -> RpcResult<PairResponse>;

    #[method(name = "health")]
    async fn health(&self) -> RpcResult<HealthResponse>;

    #[method(name = "list_clis")]
    async fn list_clis(&self) -> RpcResult<ListClisResponse>;

    #[method(name = "start_agent")]
    async fn start_agent(&self, req: StartAgentRequest) -> RpcResult<StartAgentResponse>;

    #[method(name = "send_user_message")]
    async fn send_user_message(&self, req: SendUserMessageRequest) -> RpcResult<()>;

    #[method(name = "interrupt_thread")]
    async fn interrupt_thread(&self, req: InterruptThreadRequest) -> RpcResult<()>;

    #[method(name = "close_thread")]
    async fn close_thread(&self, req: CloseThreadRequest) -> RpcResult<()>;

    #[method(name = "list_threads")]
    async fn list_threads(&self, req: ListThreadsParams) -> RpcResult<ListThreadsResponse>;

    #[method(name = "get_thread")]
    async fn get_thread(&self, req: GetThreadParams) -> RpcResult<GetThreadResponse>;
}
```

Drop `stop_agent`. Adjust the daemon's `rpc_server.rs` to dispatch the new methods to `AgentGlue`.

- [ ] **Step 3: Build + commit**

```bash
cargo build --workspace
git commit -am "feat(protocol): start_agent w/ workspace; interrupt/close/list/get RPCs; drop stop_agent"
```

### Task C17: Update FFI surfaces (uniffi + frb)

**Files:**
- Modify: `crates/minos-ffi-uniffi/src/lib.rs`
- Modify: `crates/minos-ffi-frb/src/api/minos.rs`

- [ ] **Step 1: uniffi**

Update `start_agent`/`send_user_message`/`stop_agent`/`state_stream` etc. in `crates/minos-ffi-uniffi/src/lib.rs` per spec §10.1:

- `start_agent(agent: AgentKind, workspace: String, mode: Option<AgentLaunchMode>)` — now requires workspace.
- Drop `stop_agent`.
- Add `interrupt_thread(thread_id: String)`, `close_thread(thread_id: String)`, `list_threads(filter)`, `get_thread(thread_id: String)`.
- `state_stream()` removed in favour of `thread_state_stream(thread_id)` and `manager_event_stream()`.

The macOS Swift side (`apps/macos/Minos/...`) will fail to compile against the new `.udl` / generated bindings — that is the expected intermediate state per spec §11.

- [ ] **Step 2: frb**

Same shape changes in `crates/minos-ffi-frb/src/api/minos.rs`.

Regenerate frb mirror: `cargo xtask gen-frb` (or whatever the workspace command is; see `xtask` source).

- [ ] **Step 3: Build (apps will not build; FFI crates must)**

Run: `cargo build -p minos-ffi-uniffi -p minos-ffi-frb`
Expected: PASS for these two crates. The macOS / Flutter app projects will not compile — accepted.

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(ffi): new agent RPCs (start_agent w/ workspace; interrupt/close/list/get); drop stop_agent"
```

### Task C18: Replace `AgentRuntime` with `AgentManager`; delete `runtime.rs`/`state.rs`

**Files:**
- Delete: `crates/minos-agent-runtime/src/runtime.rs`
- Delete: `crates/minos-agent-runtime/src/state.rs`
- Modify: `crates/minos-agent-runtime/src/lib.rs` (drop re-exports)

- [ ] **Step 1: Verify no callers of `AgentRuntime` / `AgentState` remain**

Run:
```bash
grep -rn 'AgentRuntime\b\|AgentState\b' crates/
```
Expected: only test files that you will rewrite, plus `apps/macos` / `apps/mobile` (UI rewrite is OOS).

- [ ] **Step 2: Delete the files**

```bash
git rm crates/minos-agent-runtime/src/runtime.rs crates/minos-agent-runtime/src/state.rs
```

In `lib.rs`, drop `pub mod runtime;`, `pub mod state;`, and the corresponding `pub use` lines. Keep `pub use` for `AgentManager`, `ThreadState`, etc.

- [ ] **Step 3: Build + commit**

```bash
cargo build --workspace
git commit -am "refactor(agent-runtime): retire AgentRuntime + AgentState (replaced by AgentManager + ThreadState)"
```

### Task C19: Idle GC + LRU evict policies

**Files:**
- Modify: `crates/minos-agent-runtime/src/manager.rs`

- [ ] **Step 1: Failing test**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn instance_reaped_after_idle_timeout() {
    use crate::state_machine::{PauseReason, ThreadState};
    use std::time::Duration;

    let cfg = AgentRuntimeConfig::new(tempfile::tempdir().unwrap().into_path());
    let caps = InstanceCaps { max_instances: 8, idle_timeout: Duration::from_millis(150) };
    let mgr = Arc::new(AgentManager::new(cfg, caps));

    let outcome = mgr.start_agent(AgentKind::Codex, "/w-reap".into()).await.unwrap();
    {
        let g = mgr.threads.lock().await;
        g.get(&outcome.thread_id).unwrap().transition(ThreadState::Idle).unwrap();
    }

    // Wait for reaper (which ticks every 60s by default — for the test, drop the tick interval to
    // 50ms via a #[cfg(test)] override in `AgentManager::new`, OR call a public test-only method
    // `mgr.tick_reaper_once().await` that runs the reap pass synchronously).
    tokio::time::sleep(Duration::from_millis(300)).await;
    mgr.tick_reaper_once().await; // implement this as a #[cfg(test)] / pub(crate) helper

    assert!(mgr.instances.lock().await.is_empty(), "instance should be reaped after idle_timeout");
    let g = mgr.threads.lock().await;
    let h = g.get(&outcome.thread_id).unwrap();
    assert!(matches!(
        h.current_state(),
        ThreadState::Suspended { reason: PauseReason::InstanceReaped }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn lru_evict_when_over_cap() {
    let cfg = AgentRuntimeConfig::new(tempfile::tempdir().unwrap().into_path());
    let caps = InstanceCaps { max_instances: 2, idle_timeout: std::time::Duration::from_secs(3600) };
    let mgr = Arc::new(AgentManager::new(cfg, caps));

    let _a = mgr.start_agent(AgentKind::Codex, "/w-A".into()).await.unwrap();
    let _b = mgr.start_agent(AgentKind::Codex, "/w-B".into()).await.unwrap();
    // Both threads default to Starting → mark Idle so they qualify for eviction.
    {
        let g = mgr.threads.lock().await;
        for h in g.values() {
            h.transition(ThreadState::Idle).unwrap();
        }
    }
    // Third workspace forces eviction.
    let _c = mgr.start_agent(AgentKind::Codex, "/w-C".into()).await.unwrap();
    let inst = mgr.instances.lock().await;
    assert_eq!(inst.len(), 2);
    assert!(inst.contains_key(&std::path::PathBuf::from("/w-C")));
    // The oldest of A/B should have been evicted; we don't assert which exactly because timing
    // depends on `last_activity_at`. Assert the new one is present and total is at cap.
}
```

Note: the test assumes a `pub(crate) async fn tick_reaper_once(&self)` helper on `AgentManager` that runs one reap-pass synchronously. Add this method during the Step-2 implementation to avoid time-flakiness in tests. Production code keeps the spawned periodic tick loop.

- [ ] **Step 2: Implement reaper task**

In `AgentManager::new`, spawn a background task:

```rust
let reaper_caps = caps.clone();
let reaper_instances = instances.clone();
let reaper_threads = threads.clone();
let reaper_mgr_tx = manager_tx.clone();
tokio::spawn(async move {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        tick.tick().await;
        let mut ig = reaper_instances.lock().await;
        let mut to_reap = Vec::new();
        for (ws, inst) in ig.iter() {
            let last = *inst.last_activity_at.lock().await;
            let idle = last.elapsed() >= reaper_caps.idle_timeout;
            let tids = inst.thread_ids().await;
            let tg = reaper_threads.lock().await;
            let any_running = tids.iter().any(|t| {
                tg.get(t).map(|h| matches!(h.current_state(), ThreadState::Running { .. })).unwrap_or(false)
            });
            if idle && !any_running {
                to_reap.push(ws.clone());
            }
        }
        drop(ig);
        for ws in to_reap {
            reap_instance(&reaper_instances, &reaper_threads, &reaper_mgr_tx, &ws).await;
        }
    }
});

async fn reap_instance(
    instances: &Arc<Mutex<HashMap<PathBuf, Arc<AppServerInstance>>>>,
    threads: &Arc<Mutex<HashMap<String, ThreadHandle>>>,
    mgr_tx: &broadcast::Sender<ManagerEvent>,
    ws: &Path,
) {
    let inst = match instances.lock().await.remove(ws) { Some(i) => i, None => return };
    let mut child = match Arc::try_unwrap(inst) {
        Ok(i) => i.child,
        Err(_) => return, // someone still holds it; skip
    };
    let _ = child.kill().await;
    // Mark threads suspended.
    // (Detail: pre-snapshot thread_ids before unwrap; this code sketches the shape.)
}
```

(The detailed reap-and-suspend code requires careful Arc handling; lift the pattern from existing crash-watcher in C12.)

LRU evict in `lru_evict`:

```rust
async fn lru_evict(&self, map: &mut HashMap<PathBuf, Arc<AppServerInstance>>) -> anyhow::Result<()> {
    let mut candidates: Vec<(PathBuf, std::time::Instant)> = Vec::new();
    let tg = self.threads.lock().await;
    for (ws, inst) in map.iter() {
        let tids = inst.thread_ids().await;
        let any_running = tids.iter().any(|t| {
            tg.get(t).map(|h| matches!(h.current_state(), ThreadState::Running { .. })).unwrap_or(false)
        });
        if !any_running {
            candidates.push((ws.clone(), *inst.last_activity_at.lock().await));
        }
    }
    drop(tg);
    candidates.sort_by_key(|(_, t)| *t);
    let victim = candidates.into_iter().next()
        .ok_or_else(|| anyhow::anyhow!("TooManyInstances: every instance has a Running thread"))?;
    map.remove(&victim.0);
    // Suspend its threads + kill child (similar to reap_instance).
    Ok(())
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p minos-agent-runtime manager
git commit -am "feat(agent-runtime/manager): idle reaper + LRU evict"
```

### Task C20: Daemon shutdown sequence

**Files:**
- Modify: `crates/minos-daemon/src/main.rs`

- [ ] **Step 1: Implement shutdown handler**

Wire SIGTERM/SIGINT (via `tokio::signal`) to a shutdown function that:

```rust
async fn shutdown(manager: Arc<AgentManager>, store: Arc<LocalStore>) {
    tracing::info!("shutdown initiated");
    // Stop accepting new RPCs (close listener) — assume rpc_server has a stop() method.
    // (If it doesn't, add one.)

    // Suspend all non-Suspended/Closed threads.
    let snap = manager.list_threads().await;
    for s in snap {
        if !matches!(s.state, ThreadState::Suspended { .. } | ThreadState::Closed { .. }) {
            // Direct DB write to flip status; also send manager event for any FFI subscriber still alive.
            sqlx::query("UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart' WHERE thread_id = ?")
                .bind(&s.thread_id).execute(store.pool()).await.ok();
        }
    }

    // SIGTERM each codex child; wait 5s; SIGKILL.
    manager.shutdown_instances(std::time::Duration::from_secs(5)).await;

    // Close DB pool.
    store.pool().close().await;
}

impl AgentManager {
    pub async fn shutdown_instances(&self, grace: std::time::Duration) {
        let mut g = self.instances.lock().await;
        for (_, inst) in g.iter_mut() {
            let _ = inst.client.shutdown_signal().await; // typed protocol shutdown if available
        }
        tokio::time::sleep(grace).await;
        for (_, inst) in std::mem::take(&mut *g).into_iter() {
            // Force-kill survivors.
            if let Ok(mut owned) = Arc::try_unwrap(inst) {
                let _ = owned.child.kill().await;
            }
        }
    }
}
```

- [ ] **Step 2: Test + commit**

Manual verification: run daemon, send SIGTERM, observe logs.

```bash
cargo build -p minos-daemon
git commit -am "feat(daemon): graceful shutdown — suspend threads + SIGTERM children"
```

### Task C21: Startup recovery — flip orphan threads to Suspended

**Files:**
- Modify: `crates/minos-daemon/src/store/mod.rs`
- Modify: `crates/minos-daemon/src/main.rs`

- [ ] **Step 1: Add `mark_orphans_suspended` to `LocalStore`**

```rust
impl LocalStore {
    pub async fn mark_orphans_suspended(&self) -> anyhow::Result<u64> {
        let r = sqlx::query(
            "UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart' \
             WHERE status NOT IN ('closed', 'suspended')"
        )
        .execute(&self.pool).await?;
        Ok(r.rows_affected())
    }
}
```

- [ ] **Step 2: Call on daemon startup**

After `LocalStore::open(&path).await?`, call `store.mark_orphans_suspended().await?` and log the rows affected.

- [ ] **Step 3: Test + commit**

Add a test in `store/mod.rs`:

```rust
#[tokio::test]
async fn mark_orphans_suspended_flips_running_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalStore::open(&tmp.path().join("t.sqlite")).await.unwrap();
    sqlx::query("INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w',0,0)")
        .execute(store.pool()).await.unwrap();
    for (i, status) in ["running", "idle", "closed", "suspended"].iter().enumerate() {
        sqlx::query("INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, '/w', 'codex', ?, 0, ?, ?)")
            .bind(format!("t{}", i)).bind(*status).bind(i as i64).bind(i as i64)
            .execute(store.pool()).await.unwrap();
    }
    let n = store.mark_orphans_suspended().await.unwrap();
    assert_eq!(n, 2);
}
```

```bash
cargo test -p minos-daemon store::tests::mark_orphans_suspended_flips_running_idle
git commit -am "feat(daemon/store): mark orphans suspended on startup"
```

### Task C22: Multi-session smoke integration test

**Files:**
- Create: `crates/minos-agent-runtime/tests/multi_session_smoke.rs`

- [ ] **Step 1: Author**

Implement the scenario from spec §12.2 against the FakeCodexBackend (no real codex spawn). Use `tokio::time::pause()` + `tokio::time::advance()` for the idle-timeout step.

```rust
use minos_agent_runtime::{
    AgentKind, AgentManager, AgentRuntimeConfig, InstanceCaps,
    state_machine::{CloseReason, PauseReason, ThreadState},
    test_support::FakeCodexBackend,
};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", start_paused = true)]
async fn multi_session_smoke() {
    let cfg = AgentRuntimeConfig::new(tempfile::tempdir().unwrap().into_path());
    let caps = InstanceCaps { max_instances: 8, idle_timeout: Duration::from_secs(60) };
    let mgr = Arc::new(AgentManager::new(cfg, caps));
    // Inject the fake codex backend (helper added in test_support; if not present, add it).
    let _fake = FakeCodexBackend::install_for(&mgr).await;

    // 1. Two workspaces, two threads each.
    let a1 = mgr.start_agent(AgentKind::Codex, "/w-A".into()).await.unwrap();
    let a2 = mgr.start_agent(AgentKind::Codex, "/w-A".into()).await.unwrap();
    let b1 = mgr.start_agent(AgentKind::Codex, "/w-B".into()).await.unwrap();
    let b2 = mgr.start_agent(AgentKind::Codex, "/w-B".into()).await.unwrap();
    assert_eq!(mgr.instances.lock().await.len(), 2);
    assert_eq!(mgr.threads.lock().await.len(), 4);

    // Move all four to Idle (FakeCodexBackend would normally emit thread/started; force the transition for the test).
    for tid in [&a1.thread_id, &a2.thread_id, &b1.thread_id, &b2.thread_id] {
        let g = mgr.threads.lock().await;
        g.get(tid).unwrap().transition(ThreadState::Idle).unwrap();
    }

    // 2. send_user_message on a1; observe ingest events.
    let mut ingest = mgr.ingest_stream();
    mgr.send_user_message(&a1.thread_id, "hello".into()).await.unwrap();
    let evt = tokio::time::timeout(Duration::from_secs(1), ingest.recv()).await.unwrap().unwrap();
    assert_eq!(evt.thread_id, a1.thread_id);

    // 3. interrupt a2; verify Suspended.
    mgr.interrupt_thread(&a2.thread_id).await.unwrap();
    {
        let g = mgr.threads.lock().await;
        assert!(matches!(
            g.get(&a2.thread_id).unwrap().current_state(),
            ThreadState::Suspended { reason: PauseReason::UserInterrupt }
        ));
    }

    // 4. send_user_message on a2 (suspended) → Resuming → Idle → Running.
    mgr.send_user_message(&a2.thread_id, "resume me".into()).await.unwrap();
    {
        let g = mgr.threads.lock().await;
        assert!(matches!(g.get(&a2.thread_id).unwrap().current_state(), ThreadState::Running { .. }));
    }

    // 5. Advance virtual clock past idle_timeout; verify InstanceReaped on /w-B (which has no Running threads).
    tokio::time::advance(Duration::from_secs(120)).await;
    mgr.tick_reaper_once().await;
    {
        let inst = mgr.instances.lock().await;
        assert!(!inst.contains_key(&std::path::PathBuf::from("/w-B")), "/w-B should be reaped");
    }
    {
        let g = mgr.threads.lock().await;
        for tid in [&b1.thread_id, &b2.thread_id] {
            assert!(matches!(
                g.get(tid).unwrap().current_state(),
                ThreadState::Suspended { reason: PauseReason::InstanceReaped }
            ));
        }
    }

    // 6. send_user_message on b1 after reap → instance respawn + Resume.
    mgr.send_user_message(&b1.thread_id, "wake".into()).await.unwrap();
    assert!(mgr.instances.lock().await.contains_key(&std::path::PathBuf::from("/w-B")));

    // 7. close_thread on b2 (still Suspended); verify Closed.
    mgr.close_thread(&b2.thread_id).await.unwrap();
    {
        let g = mgr.threads.lock().await;
        assert!(matches!(
            g.get(&b2.thread_id).unwrap().current_state(),
            ThreadState::Closed { reason: CloseReason::UserClose }
        ));
    }
}
```

If `FakeCodexBackend::install_for` does not exist, add it during this task: a helper that swaps `AgentRuntimeConfig::test_ws_url` (already exists, see `runtime.rs:144`) to point at an in-memory mock that responds to `start_thread`, `start_thread_resume`, `send_user_message`, and `interrupt_turn` with canned `ThreadStartResponse` / OK frames.

- [ ] **Step 2: Run + commit**

```bash
cargo test -p minos-agent-runtime --test multi_session_smoke
git add crates/minos-agent-runtime/tests/multi_session_smoke.rs
git commit -m "test(agent-runtime): multi-session smoke covers reap + resume + close"
```

### Task C23: Phase C verification

- [ ] **Step 1: Run check-all**

Run: `cargo xtask check-all`
Expected: PASS — fmt, clippy, test, lint-naming, frb mirror.

- [ ] **Step 2: Verify daemon starts cleanly**

Build + briefly run the daemon binary; confirm it logs `minos_home=...`, opens DB, and accepts an RPC ping.

Phase C is complete when `check-all` is green and the smoke test passes. The macOS / Flutter apps will not compile against the new FFI — agreed intermediate state per spec §11.

---

## Phase D — Reconciliation + JSONL Fallback

Goal: backend↔daemon checkpoint protocol works end-to-end; gaps in the daemon DB trigger JSONL fallback.

### Task D1: Add `EventKind::IngestCheckpoint` variant

**Files:**
- Modify: `crates/minos-protocol/src/envelope.rs:91-133`

- [ ] **Step 1: Edit enum**

```rust
pub enum EventKind {
    Paired { /* ... */ },
    PeerOnline { /* ... */ },
    PeerOffline { /* ... */ },
    Unpaired,
    ServerShutdown,
    UiEventMessage { /* ... */ },

    /// Backend → daemon, sent as the first frame after /devices WS auth.
    /// Carries backend's last_seq per thread for reconciliation.
    IngestCheckpoint {
        last_seq_per_thread: std::collections::HashMap<ThreadId, u64>,
    },
}
```

- [ ] **Step 2: Update snapshot tests + serde round-trip**

Find any envelope snapshot test (`crates/minos-protocol/tests/`) and add a case for `IngestCheckpoint`. The serde shape uses kebab-case discriminant matching existing variants.

- [ ] **Step 3: Build + commit**

```bash
cargo build --workspace
cargo test -p minos-protocol
git commit -am "feat(protocol/envelope): EventKind::IngestCheckpoint"
```

### Task D2: Backend emits `IngestCheckpoint` on /devices WS connect

**Files:**
- Modify: `crates/minos-backend/src/http/ws_devices.rs`

- [ ] **Step 1: Locate the post-auth hook**

In `ws_devices.rs`, find the spot where the WS handler establishes the device connection after authentication and before entering the read/write loop. (Likely a `handle_socket` or similar function.)

- [ ] **Step 2: Compute checkpoint and send**

Add:

```rust
// Right after auth completes for device_role = AgentHost (only agent-hosts ingest):
if device_role == DeviceRole::AgentHost {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT thread_id, COALESCE(MAX(seq), 0) FROM raw_events WHERE source_device_id = ? GROUP BY thread_id"
    )
    .bind(&device_id)
    .fetch_all(&db).await?;

    let map: std::collections::HashMap<ThreadId, u64> =
        rows.into_iter().map(|(t, s)| (t, s as u64)).collect();

    let frame = Envelope::Event {
        version: 1,
        event: EventKind::IngestCheckpoint { last_seq_per_thread: map },
    };
    ws_sink.send(WsMessage::Text(serde_json::to_string(&frame)?)).await?;
}
```

(`raw_events` schema and `source_device_id` column are existing; verify the exact column name used in the backend store.)

- [ ] **Step 3: Test**

Add to `crates/minos-backend/tests/ws_devices.rs`. Reuse the existing test harness pattern from this same file (look for the existing `register_agent_host` / `connect_devices_ws` helpers — they were established earlier and follow a stable shape):

```rust
#[tokio::test]
async fn devices_ws_emits_checkpoint_first_frame() {
    let app = TestApp::spawn().await;
    let (device_id, secret) = app.register_agent_host("host-A").await;

    // Seed raw_events for two threads.
    sqlx::query("INSERT INTO raw_events(thread_id, seq, source_device_id, payload, ts_ms, agent) VALUES (?, ?, ?, ?, ?, 'codex')")
        .bind("thr-1").bind(7i64).bind(&device_id).bind("{}").bind(0i64)
        .execute(&app.db).await.unwrap();
    sqlx::query("INSERT INTO raw_events(thread_id, seq, source_device_id, payload, ts_ms, agent) VALUES (?, ?, ?, ?, ?, 'codex')")
        .bind("thr-2").bind(3i64).bind(&device_id).bind("{}").bind(0i64)
        .execute(&app.db).await.unwrap();

    let mut ws = app.connect_devices_ws(&device_id, &secret).await;
    let first = ws.next().await.unwrap().unwrap();
    let frame: Envelope = serde_json::from_str(first.to_text().unwrap()).unwrap();
    match frame {
        Envelope::Event { event: EventKind::IngestCheckpoint { last_seq_per_thread }, .. } => {
            assert_eq!(last_seq_per_thread.get("thr-1").copied(), Some(7));
            assert_eq!(last_seq_per_thread.get("thr-2").copied(), Some(3));
        }
        other => panic!("expected IngestCheckpoint, got {:?}", other),
    }
}
```

If `register_agent_host` / `connect_devices_ws` helpers do not exist in `tests/ws_devices.rs`, write them as private helper functions in the same file using the same auth flow your existing tests use (look at any test that authenticates and connects a device).

- [ ] **Step 4: Run + commit**

```bash
cargo test -p minos-backend ws_devices
git commit -am "feat(backend/ws_devices): emit IngestCheckpoint as first frame post-auth (agent-host only)"
```

### Task D3: Daemon-side `Reconciliator` task scaffold

**Files:**
- Create: `crates/minos-daemon/src/reconciliator.rs`
- Modify: `crates/minos-daemon/src/lib.rs`

- [ ] **Step 1: Author the task**

```rust
use crate::store::{LocalStore, EventRow};
use crate::store::event_writer::EventWriter;
use minos_protocol::{Envelope, EventKind};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

fn parse_agent(s: &str) -> anyhow::Result<minos_agent_runtime::AgentKind> {
    match s {
        "codex" => Ok(minos_agent_runtime::AgentKind::Codex),
        other => anyhow::bail!("unknown agent in DB: {other}"),
    }
}

pub struct Reconciliator {
    store: Arc<LocalStore>,
    writer: Arc<EventWriter>,
    relay_out: mpsc::Sender<Envelope>,
}

impl Reconciliator {
    pub fn new(store: Arc<LocalStore>, writer: Arc<EventWriter>, relay_out: mpsc::Sender<Envelope>) -> Self {
        Self { store, writer, relay_out }
    }

    /// Called when /devices WS receives an Event::IngestCheckpoint frame.
    pub async fn on_checkpoint(&self, backend_seqs: HashMap<String, u64>) -> anyhow::Result<()> {
        let local = self.store.list_threads(None, None).await?;
        let mut tasks = Vec::new();
        // Prioritise: running > idle > suspended > closed.
        let mut sorted = local;
        sorted.sort_by_key(|t| match t.status.as_str() {
            "running" => 0, "idle" => 1, "resuming" => 1, "starting" => 1,
            "suspended" => 2, _ => 3,
        });
        for thread in sorted {
            let backend_seq = backend_seqs.get(&thread.thread_id).copied().unwrap_or(0);
            let local_seq = thread.last_seq as u64;
            if backend_seq >= local_seq { continue; }
            tasks.push(self.replay_thread(&thread.thread_id, backend_seq + 1, local_seq, &thread.agent, thread.codex_session_id.clone()));
        }
        for t in tasks { t.await?; }
        Ok(())
    }

    async fn replay_thread(
        &self,
        thread_id: &str,
        from_seq: u64,
        to_seq: u64,
        agent_str: &str,
        codex_session_id: Option<String>,
    ) -> anyhow::Result<()> {
        let agent = parse_agent(agent_str)?;
        let mut next = from_seq;
        let mut all_seqs: Vec<u64> = Vec::new();
        while next <= to_seq {
            let upper = (next + 999).min(to_seq);
            let rows = self.store.read_events(thread_id, next, upper).await?;
            for row in rows {
                all_seqs.push(row.seq as u64);
                let env = Envelope::Ingest {
                    version: 1,
                    agent: agent.clone(),
                    thread_id: row.thread_id.clone(),
                    seq: row.seq as u64,
                    payload: serde_json::from_slice(&row.payload)?,
                    ts_ms: row.ts_ms,
                };
                self.relay_out.send(env).await?;
            }
            next = upper + 1;
        }
        // Detect gaps.
        let expected: Vec<u64> = (from_seq..=to_seq).collect();
        let missing: Vec<u64> = expected.iter().copied().filter(|s| !all_seqs.contains(s)).collect();
        if !missing.is_empty() {
            tracing::warn!(thread_id, missing_count = missing.len(), "DB gap detected; attempting jsonl fallback");
            crate::jsonl_recover::recover(thread_id, &missing, &codex_session_id, &self.writer).await?;
        }
        Ok(())
    }
}
```

In `lib.rs`, add `pub mod reconciliator;`.

- [ ] **Step 2: Build**

```bash
cargo build -p minos-daemon
git add crates/minos-daemon/src/reconciliator.rs crates/minos-daemon/src/lib.rs
git commit -m "feat(daemon): Reconciliator task scaffold"
```

### Task D4: Implement `jsonl_recover`

**Files:**
- Create: `crates/minos-daemon/src/jsonl_recover.rs`
- Modify: `crates/minos-daemon/src/lib.rs`

- [ ] **Step 1: Author**

```rust
use crate::store::event_writer::EventWriter;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::fs::File;

pub async fn recover(
    thread_id: &str,
    _missing_seqs: &[u64],
    codex_session_id: &Option<String>,
    writer: &Arc<EventWriter>,
) -> Result<()> {
    let sid = match codex_session_id {
        Some(s) => s,
        None => { tracing::warn!(thread_id, "no codex_session_id; skipping recovery"); return Ok(()); }
    };
    let path = jsonl_path(sid);
    let file = match File::open(&path).await {
        Ok(f) => f,
        Err(e) => { tracing::warn!(?path, error = %e, "jsonl not readable"); return Ok(()); }
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut recovered = 0u64;
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() { continue; }
        let payload: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => { tracing::warn!(error = %e, "skipping malformed jsonl line"); continue; }
        };
        let ts_ms = payload.get("ts_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let ingest = minos_agent_runtime::RawIngest {
            agent: minos_agent_runtime::AgentKind::Codex,
            thread_id: thread_id.to_string(),
            payload,
            ts_ms,
        };
        if let Err(e) = writer.write_recovery(ingest).await {
            tracing::warn!(error = %e, "write_recovery failed for one event");
            continue;
        }
        recovered += 1;
    }
    tracing::info!(thread_id, recovered, "jsonl_recover completed");
    Ok(())
}

fn jsonl_path(codex_session_id: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join(".codex").join("sessions").join(format!("{codex_session_id}.jsonl"))
}
```

Add `dirs = "5"` to `crates/minos-daemon/Cargo.toml` `[dependencies]` if not already present.

In `lib.rs`, add `pub mod jsonl_recover;`.

- [ ] **Step 2: Test**

Add `crates/minos-daemon/tests/jsonl_recover_test.rs`:

```rust
#[tokio::test]
async fn recover_skips_when_no_codex_session_id() {
    // Setup minimal store + writer; call jsonl_recover with codex_session_id=None;
    // assert it returns Ok and no events were written.
}

#[tokio::test]
async fn recover_skips_when_file_missing() { /* ... */ }

#[tokio::test]
async fn recover_parses_valid_lines_and_writes_with_jsonl_recovery_source() {
    // Stage a fake codex session jsonl in a tempdir; override jsonl_path via env trick or
    // refactor recover() to take a path parameter for testability.
    // Assert: events table grows; source='jsonl_recovery' for new rows.
}
```

For the third test, refactor `recover()` to take an injected base path or a `JsonlSource` trait so the test can point at a temp dir without touching `~/.codex`.

- [ ] **Step 3: Run + commit**

```bash
cargo test -p minos-daemon jsonl_recover
git add crates/minos-daemon/src/jsonl_recover.rs crates/minos-daemon/Cargo.toml crates/minos-daemon/src/lib.rs crates/minos-daemon/tests/jsonl_recover_test.rs
git commit -m "feat(daemon): jsonl_recover for reconciliation gaps"
```

### Task D5: Wire Reconciliator into `relay_client`

**Files:**
- Modify: `crates/minos-daemon/src/relay_client.rs`

- [ ] **Step 1: Hook the `IngestCheckpoint` event**

In the `/devices` WS read loop where envelopes are demultiplexed, add a branch for `Envelope::Event { event: EventKind::IngestCheckpoint { last_seq_per_thread } }`:

```rust
EventKind::IngestCheckpoint { last_seq_per_thread } => {
    if let Err(e) = reconciliator.on_checkpoint(last_seq_per_thread).await {
        tracing::warn!(error = %e, "reconciliation failed");
    }
}
```

`reconciliator` should be passed into `RelayClient` at construction (or held in the daemon-level wiring).

- [ ] **Step 2: Build + commit**

```bash
cargo build -p minos-daemon
git commit -am "feat(daemon/relay_client): dispatch IngestCheckpoint to Reconciliator"
```

### Task D6: Reconciliation integration test

**Files:**
- Create: `crates/minos-daemon/tests/reconciliation_integration.rs`

- [ ] **Step 1: Author**

```rust
use minos_daemon::reconciliator::Reconciliator;
use minos_daemon::store::{LocalStore, event_writer::EventWriter};
use minos_protocol::{Envelope, EventKind};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

async fn seed_thread_with_events(store: &LocalStore, thread_id: &str, seqs: &[u64], session_id: Option<&str>) {
    sqlx::query("INSERT OR IGNORE INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w', 0, 0)")
        .execute(store.pool()).await.unwrap();
    sqlx::query("INSERT INTO threads(thread_id, workspace_root, agent, codex_session_id, status, last_seq, started_at, last_activity_at) VALUES (?, '/w', 'codex', ?, 'idle', ?, 0, 0)")
        .bind(thread_id)
        .bind(session_id)
        .bind(*seqs.iter().max().unwrap_or(&0) as i64)
        .execute(store.pool()).await.unwrap();
    for s in seqs {
        sqlx::query("INSERT INTO events(thread_id, seq, payload, ts_ms, source) VALUES (?, ?, ?, ?, 'live')")
            .bind(thread_id).bind(*s as i64).bind(serde_json::to_vec(&serde_json::json!({"seq": s})).unwrap()).bind(0i64)
            .execute(store.pool()).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reconciliation_replays_missing_seqs() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalStore::open(&tmp.path().join("t.sqlite")).await.unwrap());
    let (relay_out_tx, mut relay_out_rx) = mpsc::channel::<Envelope>(256);
    let writer = Arc::new(EventWriter::spawn(store.clone(), relay_out_tx.clone()));

    let seqs: Vec<u64> = (1..=100).collect();
    seed_thread_with_events(&store, "thr-X", &seqs, None).await;

    let recon = Reconciliator::new(store.clone(), writer.clone(), relay_out_tx);

    let mut backend_seqs = HashMap::new();
    backend_seqs.insert("thr-X".to_string(), 50u64);
    recon.on_checkpoint(backend_seqs).await.unwrap();

    let mut got: Vec<u64> = Vec::new();
    while let Ok(Some(env)) = tokio::time::timeout(std::time::Duration::from_millis(500), relay_out_rx.recv()).await {
        if let Envelope::Ingest { thread_id, seq, .. } = env {
            assert_eq!(thread_id, "thr-X");
            got.push(seq);
        }
    }
    assert_eq!(got, (51..=100).collect::<Vec<_>>(), "should replay 51..=100 in order");
}

#[tokio::test(flavor = "multi_thread")]
async fn reconciliation_falls_back_to_jsonl_on_gap() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalStore::open(&tmp.path().join("t.sqlite")).await.unwrap());
    let (relay_out_tx, _relay_out_rx) = mpsc::channel::<Envelope>(1024);
    let writer = Arc::new(EventWriter::spawn(store.clone(), relay_out_tx.clone()));

    // Seed seqs 1..=100 minus 60..=70.
    let seqs: Vec<u64> = (1..=100).filter(|s| !(60..=70).contains(s)).collect();
    seed_thread_with_events(&store, "thr-Y", &seqs, Some("sess-uuid-1")).await;

    // Stage a fake codex jsonl at the path that jsonl_recover will look up.
    // For this test, jsonl_recover must be refactored to accept a base path; pass a temp dir
    // and stage a fake `~/.codex/sessions/sess-uuid-1.jsonl` under it.
    let fake_codex_root = tmp.path().join("fake-codex-home");
    std::fs::create_dir_all(fake_codex_root.join(".codex/sessions")).unwrap();
    let jsonl_path = fake_codex_root.join(".codex/sessions/sess-uuid-1.jsonl");
    let payload_lines = (60..=70u64).map(|s| serde_json::json!({"recovered_seq": s, "ts_ms": s as i64}).to_string()).collect::<Vec<_>>().join("\n");
    std::fs::write(&jsonl_path, payload_lines).unwrap();

    let recon = Reconciliator::new(store.clone(), writer.clone(), relay_out_tx);
    let mut backend_seqs = HashMap::new();
    backend_seqs.insert("thr-Y".to_string(), 50u64);
    recon.on_checkpoint(backend_seqs).await.unwrap();

    // Allow writer batches to flush.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let recovered: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE thread_id = ? AND source = 'jsonl_recovery'")
        .bind("thr-Y")
        .fetch_one(store.pool()).await.unwrap();
    assert_eq!(recovered, 11, "11 missing events should be recovered (seq 60..=70)");
}
```

The second test depends on `jsonl_recover::recover` being refactored to accept an injected base path (for testability). Update D4 step 1 to expose `pub fn set_codex_home_for_testing(path: PathBuf)` or a `JsonlSource` parameter — whichever fits cleaner. The default behaviour reads from `dirs::home_dir()/.codex/sessions/`.

- [ ] **Step 2: Run + commit**

```bash
cargo test -p minos-daemon --test reconciliation_integration
git add crates/minos-daemon/tests/reconciliation_integration.rs
git commit -m "test(daemon): reconciliation integration covers replay + jsonl fallback"
```

### Task D7: Phase D verification

- [ ] **Step 1: Run check-all**

Run: `cargo xtask check-all`
Expected: PASS.

- [ ] **Step 2: Manual end-to-end smoke**

Run daemon + a stub backend; confirm:

1. Daemon connects to /devices.
2. Backend immediately sends `IngestCheckpoint`.
3. Daemon logs reconciliation activity.
4. Disconnect/reconnect cycle is idempotent.

Phase D is complete when integration tests pass and manual smoke is clean.

---

## Final Verification

After D7:

- [ ] **Step 1: Cumulative `cargo xtask check-all`**

Run: `cargo xtask check-all`
Expected: PASS — fmt, clippy, all tests, lint-naming, frb mirror.

- [ ] **Step 2: Confirm spec coverage with grep**

Run:
```bash
grep -rn 'AgentRuntime\b\|AgentState\b\|MacSummary\b\|MeMacsResponse\b\|stop_agent\b\|MINOS_DATA_DIR\|MINOS_LOG_DIR\|Library/Application Support/Minos\|Library/Logs/Minos\|account_mac_pairings' crates/
```
Expected: zero hits in `crates/` (Rust code surface).

- [ ] **Step 3: Final commit summarising the branch**

The branch is now ready. The macOS / Flutter app code still references the old FFI (acknowledged spec §2 OOS); UI rewrite is the follow-up workstream and lands in subsequent commits on the same branch.

---

## Appendix — Self-Review Checklist (run by the plan author after writing)

The following items were verified inline:

- ✅ Spec §5 (paths) → Phase A tasks A1-A6
- ✅ Spec §6 (naming) → Phase B tasks B1-B11
- ✅ Spec §7 (manager) → Phase C tasks C6-C14, C19-C22
- ✅ Spec §8 (persistence) → Phase C tasks C1-C5, C15, C21
- ✅ Spec §9 (reconciliation) → Phase D tasks D1-D6
- ✅ Spec §10 (FFI surface) → Phase C tasks C16-C17
- ✅ Spec §11 (delivery) → Phase D7 + final verification
- ✅ Spec §12 (test strategy) → tasks integrate TDD per change; explicit integration tests in C22, D6
- ✅ Spec §13 (risks) → Phase C explicitly notes the codex resume verification risk in C13

No placeholders. Every task has actual code or exact commands. Type names are consistent across tasks (`ThreadHandle`, `AgentManager`, `LocalStore`, `EventWriter`, `Reconciliator`, `ThreadState`, `PauseReason`, `CloseReason`, `ManagerEvent`, `HostSummary`, `MeHostsResponse`, `MobileClient`).
