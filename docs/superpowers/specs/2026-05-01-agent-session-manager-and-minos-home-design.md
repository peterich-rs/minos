# Agent Session Manager + `$MINOS_HOME` + Protocol Naming Cleanup — Design

**Status**: Draft
**Created**: 2026-05-01
**Branch**: `feature/mobile-auth-and-agent-session`
**Supersedes (in part)**: `codex-app-server-integration-design.md` §2.2 single-session constraint
**Adjacent**: `codex-typed-protocol-design.md` (Plan 10), `mobile-auth-and-agent-session-design.md` (Plan 11)

## 1. Goal

Lift the agent runtime from a single-session, in-memory, macOS-pathed prototype into a multi-workspace, multi-thread system with durable per-thread state, cross-platform on-disk layout, and a platform-neutral protocol surface.

Concretely:

1. **Multi-session manager**: per-workspace `codex app-server` instance hosting multiple concurrent threads, with explicit interrupt / implicit resume semantics.
2. **Local persistence**: SQLite store for per-thread events and metadata so the daemon can survive restart, support gap-fill from the backend, and serve `list_threads` locally.
3. **Cross-platform paths**: consolidate every daemon path under `$MINOS_HOME` (default `$HOME/.minos`); drop the macOS-specific `Library/Application Support/Minos` and `Library/Logs/Minos` branches.
4. **Reconciliation**: a checkpoint-on-connect protocol so backend and daemon can re-synchronise event streams after either side reconnects.
5. **Protocol naming cleanup**: rename `Mac → Host` and `Ios → Mobile` everywhere they cross protocol / FFI / HTTP / SQL boundaries.

## 2. Non-Goals

- **macOS / Flutter UI rewrite**. This spec freezes the FFI surface; UI viewmodel updates are tracked separately and land in follow-up commits on the same branch.
- **Linux / Windows secret-file fallback** (`secrets/device-secret`). Deferred to its own spec; macOS Keychain remains the only secret store this spec ships.
- **Backend `raw_events` schema changes** (e.g. tombstone rows for permanent gaps). Out of scope; see §8.3 for the implication.
- **codex protocol changes** — covered by Plan 10 / `codex-typed-protocol-design.md`.
- **Mobile auth, pair semantics** — covered by Plan 11 / `server-centric-auth-and-pair.md`.
- **Backwards-compatibility shims** — no dual-write / dual-read period; hard cut.

## 3. Decisions Log

The alternatives below were evaluated and rejected during brainstorming. They are recorded here so future readers do not relitigate.

| Alternative | Why rejected |
|---|---|
| Single `codex app-server` process hosting all workspaces | One crash kills every workspace; defeats isolation goal. |
| Per-thread subprocess (one codex process per thread) | OS overhead — each codex is ~50-150 MB; multi-thread per workspace is the dominant case. |
| DB scope **A** (events only) | Cannot resume threads across daemon restart; metadata homeless. |
| DB scope **C** (full mirror + outbox) | Duplicates backend; double-write consistency overhead with no product win. |
| Explicit gap-fill RPC (request/response with `request_id`) | Subsumed by the simpler checkpoint-on-connect; needless state machine. |
| Keep `stop_agent()` for "kill all threads" | Footgun in multi-session world; per-thread `interrupt`/`close` covers every legitimate use. |
| Keep `MINOS_DATA_DIR` / `MINOS_LOG_DIR` env aliases | No production users; hard cut keeps configuration surface small. |
| Wire-level dual-read window for `Mac → Host` rename | Same. |
| Rename `thread_id` → `conversation_id` | All existing code uses `thread_id`; codex upstream uses "thread"; pure churn. |
| Mobile pre-generates `thread_id` | Conflicts with codex `item/threadStart` server-side allocation. |
| Use `User-Agent` for role/platform routing | UA is informational and spoofable (RFC 7231 §5.5.3); routing on UA is an antipattern. |
| Move `role` into request body / new header | Role is a server-side concept already implied by which auth scheme the client presents and which registration endpoint it called; wire-level role declaration is redundant. *Status quo retained:* `X-Device-Role` header keeps existing semantics, only the value `"ios-client"` → `"mobile-client"`. |

## 4. System Architecture

```
┌─────────────────── daemon (cross-platform) ───────────────────────┐
│                                                                    │
│  ┌─ AgentManager ───────────────────────────────────────────┐     │
│  │  Instances: HashMap<WorkspaceRoot, AppServerInstance>    │     │
│  │     ├─ Instance(/foo)                                    │     │
│  │     │    ├─ codex app-server child                       │     │
│  │     │    ├─ CodexClient (1 WS to child)                  │     │
│  │     │    └─ Threads: HashSet<thread_id>                  │     │
│  │     └─ Instance(/bar) ...                                │     │
│  │                                                          │     │
│  │  Threads: HashMap<thread_id, ThreadHandle>               │     │
│  │     (cross-instance index, by-id lookup for RPC)         │     │
│  │                                                          │     │
│  │  Idle GC, LRU evict, crash recovery                      │     │
│  └───────────────────┬──────────────────────────────────────┘     │
│                      │ RawIngest (broadcast)                       │
│                      ▼                                             │
│  ┌─ EventWriter (single-writer task) ───────────────────────┐     │
│  │  for each RawIngest:                                     │     │
│  │    1. tx = db.begin()                                    │     │
│  │    2. INSERT events; UPDATE threads.last_seq             │     │
│  │    3. tx.commit()         ← fsync boundary               │     │
│  │    4. relay_out_tx.send(Envelope::Ingest{...})           │     │
│  │    5. events_tx.send(RawIngest{...})  // local broadcast │     │
│  └───────────────────┬──────────────────────────────────────┘     │
│                      │                                             │
│  ┌─ LocalStore (SQLite WAL) ────────────────────────────────┐     │
│  │   tables: events, threads, workspaces, schema_version    │     │
│  │   reads: list_threads, get_thread, get_event_range       │     │
│  └──────────────────────────────────────────────────────────┘     │
│                                                                    │
│  ┌─ Reconciliator ──────────────────────────────────────────┐     │
│  │  on /devices WS connect:                                 │     │
│  │    receive Event::IngestCheckpoint                       │     │
│  │    for each (thread_id, last_seq):                       │     │
│  │      SELECT events WHERE thread_id=? AND seq>?           │     │
│  │      emit Envelope::Ingest in seq order                  │     │
│  │      if internal gap → jsonl fallback                    │     │
│  └──────────────────────────────────────────────────────────┘     │
│                                                                    │
│  ┌─ Paths ──────────────────────────────────────────────────┐     │
│  │   $MINOS_HOME (default $HOME/.minos)                     │     │
│  │   {state, secrets, db, logs, workspaces, run}/           │     │
│  └──────────────────────────────────────────────────────────┘     │
└────────────────────────────────────────────────────────────────────┘
                          │
                          │ /devices WS
                          │  · Envelope::Ingest         (daemon → backend)
                          │  · Event::IngestCheckpoint  (backend → daemon, on connect)
                          ▼
                      backend (existing sqlite, raw_events / threads, /v1)
```

## 5. `$MINOS_HOME` Directory Layout

### 5.1 Semantics

- env `MINOS_HOME` set ⇒ that directory is the root (no `.minos` suffix appended)
- env unset ⇒ default `$HOME/.minos`
- Aligned with codex's `CODEX_HOME` semantics. Already implemented at `crates/minos-daemon/src/paths.rs:5-15`.

### 5.2 Subdirectory layout

```
$MINOS_HOME (default $HOME/.minos)
├── state/
│   └── local-state.json        # current `LocalState` (self_device_id, peer)
├── secrets/                    # 0700; non-macOS only (deferred — see §2 OOS)
├── db/
│   ├── minos.sqlite
│   ├── minos.sqlite-wal
│   └── minos.sqlite-shm
├── logs/
│   ├── daemon.log              # current
│   └── daemon-YYYY-MM-DD.log   # rolled
├── workspaces/                 # codex workspace roots (already established)
│   └── <workspace-key>/
└── run/                        # transient: PID, optional unix sockets
    └── daemon.pid
```

### 5.3 New `paths` API

```rust
// crates/minos-daemon/src/paths.rs (existing minos_home() retained)
pub fn state_dir()       -> Result<PathBuf>;   // = minos_home()/state
pub fn secrets_dir()     -> Result<PathBuf>;   // = minos_home()/secrets   (mode 0700)
pub fn db_dir()          -> Result<PathBuf>;   // = minos_home()/db
pub fn db_path()         -> Result<PathBuf>;   // = db_dir()/minos.sqlite
pub fn logs_dir()        -> Result<PathBuf>;   // = minos_home()/logs
pub fn workspaces_dir()  -> Result<PathBuf>;   // = minos_home()/workspaces
pub fn run_dir()         -> Result<PathBuf>;   // = minos_home()/run
```

Each helper performs idempotent `create_dir_all` once. `secrets_dir()` chmod 0700 on creation.

### 5.4 Callsite migration

| Existing | After |
|---|---|
| `crates/minos-daemon/src/local_state.rs:21-29` `default_path()` writing `$HOME/Library/Application Support/Minos/local-state.json` | `paths::state_dir().join("local-state.json")` |
| `crates/minos-daemon/src/main.rs:146-157` `platform_data_dir()` macOS branch | Delete the function; callers use `paths::state_dir()` |
| `crates/minos-daemon/src/logging.rs:19-31` `log_dir()` macOS branch | `paths::logs_dir()` |

### 5.5 ENV cleanup

- `MINOS_DATA_DIR` — **deleted**: `main.rs:141-152` no longer reads or sets
- `MINOS_LOG_DIR` — **deleted**: `logging.rs:21` no longer reads
- `MINOS_HOME` — sole entry point

Daemon emits a startup line `minos_home={path}` for diagnostics.

### 5.6 macOS Keychain

`crates/minos-daemon/src/keychain_store.rs` retained verbatim, still gated `#[cfg(target_os = "macos")]`, service `"ai.minos.macos"`. Linux / Windows secret fallback files are deferred (see §2 OOS).

## 6. Protocol Naming Cleanup (`Mac → Host`, `Ios → Mobile`)

### 6.1 Rename map

| Where | Before | After |
|---|---|---|
| `crates/minos-protocol/src/messages.rs:30-41` DTO | `MacSummary { mac_device_id, mac_display_name, paired_at_ms, paired_via_device_id }` | `HostSummary { host_device_id, host_display_name, paired_at_ms, paired_via_device_id }` |
| `crates/minos-protocol/src/messages.rs:30-32` response | `MeMacsResponse { macs: Vec<MacSummary> }` | `MeHostsResponse { hosts: Vec<HostSummary> }` |
| `crates/minos-protocol/src/messages.rs:130-140` QR alias | `#[serde(alias = "mac_display_name")]` on `host_display_name` | Drop alias |
| `crates/minos-domain/src/role.rs:18-30` enum + wire | `DeviceRole::IosClient` / `"ios-client"` | `DeviceRole::MobileClient` / `"mobile-client"` |
| `crates/minos-mobile/src/store.rs:74,77,80,148-160` | `save_active_mac` / `load_active_mac` / `clear_active_if(mac)` / `active_mac` | `save_active_host` / `load_active_host` / `clear_active_if(host)` / `active_host` |
| `crates/minos-mobile/src/client.rs:459` | `forget_mac(mac: DeviceId)` | `forget_host(host: DeviceId)` |
| `crates/minos-mobile/src/client.rs:513` | `list_paired_macs() -> Vec<MacSummary>` | `list_paired_hosts() -> Vec<HostSummary>` |
| `crates/minos-mobile/src/client.rs:523` | `set_active_mac(mac: DeviceId)` | `set_active_host(host: DeviceId)` |
| `crates/minos-ffi-frb/src/api/minos.rs:143-247` | `MacSummaryDto`, `forget_mac`, `list_paired_macs`, `set_active_mac` | `HostSummaryDto`, `forget_host`, `list_paired_hosts`, `set_active_host` |
| `crates/minos-backend/src/http/v1/me.rs` route | `/v1/me/macs` | `/v1/me/hosts` |
| Backend table | `account_mac_pairings` (col `mac_device_id`) | `account_host_pairings` (col `host_device_id`) |
| Backend module | `crates/minos-backend/src/store/account_mac_pairings.rs` | `account_host_pairings.rs` |

### 6.2 `X-Device-Role` header — retained, value renamed

The header **stays**. Only the string value changes:

- `"agent-host"` — unchanged (already platform-neutral)
- `"ios-client"` → `"mobile-client"` at:
  - `crates/minos-mobile/src/http.rs:75`
  - `crates/minos-mobile/src/client.rs:986, 1555`

(Removing the header was considered and deferred; see §3 decisions log.)

### 6.3 SQL migration

`crates/minos-backend/migrations/0013_rename_account_mac_to_host.sql`:

```sql
ALTER TABLE account_mac_pairings RENAME TO account_host_pairings;
ALTER TABLE account_host_pairings RENAME COLUMN mac_device_id TO host_device_id;

-- Indexes auto-rename in SQLite 3.25+; verify and re-create explicitly if any
-- 0012-era index references the old column literally in its WHERE clause.
```

`sqlx::migrate!()` applies on backend startup. `RENAME COLUMN` is non-destructive.

### 6.4 Comment / ADR text adjustments

`crates/minos-protocol/src/envelope.rs:99-107` `EventKind::Paired.your_device_secret` — comment "iOS recipient" → "MobileClient recipient". ADR-0020 historical text untouched.

### 6.5 Test fixtures

String values like `"MacBook Pro"`, `"iPhone 15"` in test data (e.g. `messages.rs:250-285`) are **values, not identifiers**, and are explicitly excluded from the rename. They stay.

### 6.6 Lint gate

Add `cargo xtask lint-naming` (invoked by `cargo xtask check-all`) that greps `\b(mac|ios)_(device_id|display_name|client)\b` across `crates/minos-protocol`, `crates/minos-domain`, `crates/minos-ffi-*`, `crates/minos-mobile`, `crates/minos-daemon`, `crates/minos-backend/migrations` and fails on any hit. Internal `cfg(target_os = "macos")` is exempt.

## 7. Multi-Session Manager

### 7.1 Internal types

```rust
// crates/minos-agent-runtime/src/manager.rs (new)
pub struct AgentManager {
    config: Arc<AgentRuntimeConfig>,
    store: Arc<LocalStore>,
    instances: Arc<Mutex<HashMap<WorkspaceRoot, AppServerInstance>>>,
    threads: Arc<Mutex<HashMap<ThreadId, ThreadHandle>>>,
    events_tx: broadcast::Sender<RawIngest>,
    manager_tx: broadcast::Sender<ManagerEvent>,
    instance_caps: InstanceCaps,            // max_instances=8, idle_timeout=30min
}

pub struct AppServerInstance {
    workspace: WorkspaceRoot,
    child: Child,
    client: Arc<CodexClient>,
    threads: HashSet<ThreadId>,
    spawned_at: Instant,
    last_activity_at: Mutex<Instant>,
    drop_guard: AbortHandle,
}

pub struct ThreadHandle {
    thread_id: ThreadId,
    workspace: WorkspaceRoot,
    agent: AgentKind,
    codex_session_id: Option<String>,
    state: watch::Sender<ThreadState>,
    state_rx: watch::Receiver<ThreadState>,
    last_seq: Arc<AtomicU64>,
}
```

`Instances` is keyed for subprocess-level operations; `threads` is the cross-instance index for RPC dispatch by `thread_id`.

### 7.2 Per-thread state machine

```rust
pub enum ThreadState {
    Starting,
    Idle,
    Running { turn_started_at: Instant },
    Suspended { reason: PauseReason },
    Resuming,
    Closed { reason: CloseReason },
}

pub enum PauseReason {
    UserInterrupt,
    CodexCrashed,
    DaemonRestart,
    InstanceReaped,
}

pub enum CloseReason {
    UserClose,
    TerminalError,
}
```

Transition matrix:

| From | Trigger | To | Side effects |
|---|---|---|---|
| (none) | `start_agent` succeeds | `Starting` | INSERT `threads` row |
| `Starting` | codex `thread/started` | `Idle` | UPDATE `codex_session_id` |
| `Idle` | `send_user_message` accepted by codex | `Running { now }` | — |
| `Running` | codex `thread/turnEnd` | `Idle` | — |
| `Running` | `interrupt_thread` RPC | `Suspended { UserInterrupt }` | best-effort `TurnInterruptParams` to codex |
| `Idle` ∨ `Running` | codex child exits | `Suspended { CodexCrashed }` | Instance removed from HashMap |
| `Idle` ∨ `Running` | daemon SIGTERM | `Suspended { DaemonRestart }` | flush DB before exit |
| `Idle` (whole instance idle) | `idle_timeout` expired | `Suspended { InstanceReaped }` | Instance reaped |
| `Suspended` | `send_user_message` (implicit resume) | `Resuming` | see §7.4 |
| `Resuming` | rollout replay + `last_seq` aligned | `Idle` | — |
| `Resuming` | replay fails | `Closed { TerminalError }` | UPDATE `threads.ended_at` |
| any non-`Closed` | `close_thread` RPC | `Closed { UserClose }` | UPDATE `threads.ended_at` |

### 7.3 RPC surface

```rust
// crates/minos-protocol/src/rpc.rs - MinosRpc adjustments
pub trait MinosRpc {
    // Unchanged
    async fn pair(...) -> ...;
    async fn health(...) -> ...;
    async fn list_clis(...) -> ...;

    // Adjusted: workspace becomes mandatory
    async fn start_agent(req: StartAgentRequest) -> Result<StartAgentResponse>;

    // Adjusted: now accepts Suspended threads (implicit resume)
    async fn send_user_message(req: SendUserMessageRequest) -> Result<()>;

    // New
    async fn interrupt_thread(req: InterruptThreadRequest) -> Result<()>;
    async fn close_thread(req: CloseThreadRequest) -> Result<()>;
    async fn list_threads(req: ListThreadsParams) -> Result<ListThreadsResponse>;
    async fn get_thread(req: GetThreadParams) -> Result<GetThreadResponse>;

    // Removed
    // async fn stop_agent(...) -> ...;
}

// crates/minos-protocol/src/messages.rs additions
pub struct StartAgentRequest {
    pub agent: AgentKind,
    pub workspace: PathBuf,                 // new, mandatory
    pub mode: Option<AgentLaunchMode>,      // default Server
}

pub struct InterruptThreadRequest { pub thread_id: ThreadId }
pub struct CloseThreadRequest    { pub thread_id: ThreadId }
pub struct GetThreadParams       { pub thread_id: ThreadId }
pub struct GetThreadResponse {
    pub thread: ThreadSummary,              // existing type, gain `state` field
    pub state: ThreadState,
}
```

`ThreadSummary` gains a `state: ThreadState` field. `ListThreadsParams`/`ListThreadsResponse` retain their existing signatures; the daemon serves these locally against its own `threads` table (backend continues to serve them from its tables — both endpoints exist).

### 7.4 Implicit resume flow

Triggered by `send_user_message(thread_id, text)` against a `Suspended` thread:

```
1. ThreadHandle.state = Resuming
2. SELECT workspace_root, codex_session_id FROM threads WHERE thread_id=?
3. instances.get(workspace_root):
     None → spawn AppServerInstance(workspace_root)
            (same code path as start_agent's first call into a workspace)
     Some → reuse
4. CodexClient.start_thread(resume_from_session = Some(codex_session_id))
   codex internally replays its own rollout to restore context
5. Verify codex's thread is Idle; verify our last_seq still matches the
   highest seq we have emitted (as recorded in threads.last_seq)
6. ThreadHandle.state = Idle
7. Proceed with the original send_user_message → state = Running
```

Failure at step 4 (rollout missing / corrupt / codex rejects resume) ⇒ state → `Closed { TerminalError }`; `manager_event_stream` emits `ThreadStateChanged`; UI surfaces "this conversation cannot be resumed; please start a new one."

The daemon does **not** read `~/.codex/sessions/*.jsonl` for resume — codex resumes from its own rollout internally. JSONL fallback (§8.3) is a separate, narrower mechanism for backend reconciliation gaps.

### 7.5 Stream surface

```rust
impl AgentManager {
    pub fn ingest_stream(&self) -> broadcast::Receiver<RawIngest>;
    pub fn thread_state_stream(&self, thread_id: &ThreadId) -> Option<watch::Receiver<ThreadState>>;
    pub fn manager_event_stream(&self) -> broadcast::Receiver<ManagerEvent>;
}

pub enum ManagerEvent {
    ThreadAdded { thread_id: ThreadId, workspace: PathBuf, agent: AgentKind },
    ThreadStateChanged { thread_id: ThreadId, old: ThreadState, new: ThreadState, at: i64 },
    ThreadClosed { thread_id: ThreadId, reason: CloseReason },
    InstanceCrashed { workspace: PathBuf, affected_threads: Vec<ThreadId> },
}
```

UI consumption pattern:
- **Conversation list page** subscribes `manager_event_stream` and maintains a reactive view.
- **Single conversation page** calls `get_thread(thread_id)` once for current state, then subscribes `thread_state_stream(thread_id)` and filters `ingest_stream` by `payload.thread_id`.

### 7.6 Daemon shutdown sequence

On SIGTERM / SIGINT:

```
1. Stop accepting new RPCs (close rpc_server listener)
2. For each thread in {Starting, Idle, Running, Resuming}:
     - state = Suspended { DaemonRestart }
     - flush in-memory last_seq to DB if dirty
3. For each instance:
     - SIGTERM the codex child
     - wait 5s
     - SIGKILL survivors
4. Close DB pool (fsync WAL)
5. Close /devices WS
6. exit
```

Maximum bounded shutdown ~8s.

### 7.7 Resource caps

- `max_instances = 8` (configurable via `AgentRuntimeConfig::instance_caps`)
- `idle_timeout = 30min` (configurable)
- LRU evict triggers when at the cap and a new workspace is requested:
  1. Prefer instances whose threads are all `Suspended` ∨ `Closed` (no in-process work to disrupt).
  2. Else the instance with oldest `last_activity_at` whose threads contain no `Running` thread (an `Idle` thread can be safely paused; a `Running` thread cannot — interrupting mid-turn would lose the user's in-flight prompt).
  3. If no instance qualifies (every instance has a `Running` thread), return `RpcError::TooManyInstances`; UI prompts the user to close some conversations first.
- Eviction is equivalent to idle GC: SIGTERM child, transition affected threads to `Suspended { InstanceReaped }`.

## 8. Local Persistence

### 8.1 DB choice

- **`sqlx 0.8` + `sqlite` feature**, sharing the workspace dependency (`Cargo.toml:64`).
- File: `paths::db_path()` ⇒ `$MINOS_HOME/db/minos.sqlite`.
- `PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;`
- Single-writer task (the EventWriter) for INSERT/UPDATE; read pool for SELECT.
- Daemon's migrations directory is **separate** from the backend's: `crates/minos-daemon/migrations/`. `sqlx::migrate!()` runs on startup.

### 8.2 Schema

`crates/minos-daemon/migrations/0001_initial.sql`:

```sql
CREATE TABLE schema_version (
    version    INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);

CREATE TABLE workspaces (
    root          TEXT PRIMARY KEY,         -- canonical absolute path
    first_seen_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL
);

CREATE TABLE threads (
    thread_id          TEXT PRIMARY KEY,
    workspace_root     TEXT NOT NULL REFERENCES workspaces(root),
    agent              TEXT NOT NULL,        -- 'codex' | …
    codex_session_id   TEXT,                 -- codex-allocated UUID, used for resume
    status             TEXT NOT NULL,        -- 'starting' | 'idle' | 'running'
                                             -- | 'suspended' | 'resuming' | 'closed'
    last_pause_reason  TEXT,                 -- 'user_interrupt' | 'codex_crashed'
                                             -- | 'daemon_restart' | 'instance_reaped' | NULL
    last_close_reason  TEXT,                 -- 'user_close' | 'terminal_error' | NULL
    last_seq           INTEGER NOT NULL DEFAULT 0,
    started_at         INTEGER NOT NULL,
    last_activity_at   INTEGER NOT NULL,
    ended_at           INTEGER                -- only when status='closed'
);

CREATE INDEX threads_by_workspace ON threads(workspace_root, last_activity_at DESC);
CREATE INDEX threads_by_status    ON threads(status, last_activity_at DESC);

CREATE TABLE events (
    thread_id TEXT NOT NULL,
    seq       INTEGER NOT NULL,
    payload   BLOB NOT NULL,                 -- serde_json::to_vec of codex event JSON
    ts_ms     INTEGER NOT NULL,
    source    TEXT NOT NULL DEFAULT 'live',  -- 'live' | 'jsonl_recovery'
    PRIMARY KEY (thread_id, seq),
    FOREIGN KEY (thread_id) REFERENCES threads(thread_id)
) WITHOUT ROWID;

CREATE INDEX events_by_ts ON events(thread_id, ts_ms);
```

Notes:
- `events.payload` is BLOB (not TEXT) because codex events may contain non-UTF-8 bytes (image attachments etc.); BLOB skips the SQLite UTF-8 validation.
- `WITHOUT ROWID` reduces space ~30 % and removes one indirection on the `(thread_id, seq)` reads which dominate the workload.
- `events.source = 'jsonl_recovery'` audits rows recovered via §8.3 fallback.

### 8.3 Write-ahead pipeline

```
RawIngest →
  EventWriter task (single-writer):
    1. tx = pool.begin().await?
    2. seq = thread_handle.last_seq.load() + 1
    3. INSERT INTO events(thread_id, seq, payload, ts_ms, source='live')
    4. UPDATE threads SET last_seq = ?, last_activity_at = ?
    5. tx.commit().await?                        ← fsync boundary
    6. thread_handle.last_seq.store(seq)
    7. relay_out_tx.send(Envelope::Ingest { ..., seq, ... })
    8. events_tx.send(RawIngest { ... })          (in-process broadcast)
```

**Invariant**: any `seq` ever sent on the WS is already durably committed in `events`. The Reconciliator (§9.2) depends on this.

The reverse is allowed: a `seq` can be in DB but not yet sent (daemon crashed between commit and send). Reconciliation's checkpoint will be older, and the daemon re-sends from there. Idempotent on the backend side (`(thread_id, seq) UNIQUE` dedupes).

### 8.4 Batching

- Writer's input channel is `mpsc<RawIngest>`.
- Each loop iteration drains pending events for up to **5 ms** or **100 events** (whichever first), then commits in a single transaction.
- Throughput: WAL mode + grouped commits handles ≥ 5 k events/s on commodity SSD; codex emit rate is ~tens/s/thread.

### 8.5 Failure handling

- `INSERT` violates `(thread_id, seq) UNIQUE`: theoretically impossible (seq is monotonic via atomic). If it happens the manager has lost track of state; **panic to force daemon exit** — corruption recovery is preferable to silent divergence. The daemon process exits non-zero and is restarted by whatever launches it (user, launchd, systemd; the daemon does not self-supervise).
- `commit` returns IO error (disk full, FS error): log error, increment a `commit_failures_total` counter, **drop the event**, do not send WS. Operator-level alerting required. The ingest line for this thread continues with the next event (next seq); the dropped event is permanently lost. This is a deliberate trade-off: if commit failures are actually happening, the host is already in distress and complex recovery is unlikely to succeed.

### 8.6 Startup recovery

```
1. Open DB, apply migrations.
2. Start EventWriter task.
3. UPDATE threads
   SET status='suspended', last_pause_reason='daemon_restart'
   WHERE status NOT IN ('closed', 'suspended');     -- batch one-shot
4. Begin accepting RPCs.
```

No eager hydration. A `ThreadHandle` is materialised lazily on the first RPC that references its `thread_id`:

```
1. SELECT * FROM threads WHERE thread_id = ?
2. Construct ThreadHandle with state = Suspended.
3. If the request is send_user_message → enter §7.4 resume flow.
```

### 8.7 Volume & maintenance

- Per turn: ~5-50 events × ~200-2 000 B = ~5-100 KB.
- Heavy daily use ~10 turns × 30 events × 1 KB = 300 KB.
- A year ≈ 100-150 MB including WAL and indexes.
- No automatic pruning in this spec. A future "purge events for closed threads older than N days" task is left for a follow-up.

### 8.8 Backup & rebuild

- Backup: copy `$MINOS_HOME/db/` (incl. WAL and SHM); daemon does not need to stop.
- Reset: deleting `db/` causes the daemon to rebuild empty schema on next start. Surviving codex JSONL rollouts in `~/.codex/sessions/` can repopulate recent threads via §8.3 (best-effort).

## 9. Reconciliation Protocol

### 9.1 Protocol additions

```rust
// crates/minos-protocol/src/envelope.rs - EventKind extension
pub enum EventKind {
    // existing variants unchanged …
    /// Backend → daemon, sent as the first frame after /devices WS auth completes.
    /// Carries backend's known last_seq per thread for reconciliation.
    IngestCheckpoint {
        last_seq_per_thread: HashMap<ThreadId, u64>,
    },
}
```

- No new daemon → backend frame. Replay re-uses `Envelope::Ingest`; backend deduplicates via `(thread_id, seq) UNIQUE`.
- No `request_id` correlation, no ack frame. The protocol is a one-way state alignment.

### 9.2 Reconciliation flow

```
on /devices WS connect:
    wait for first Envelope::Event { event: IngestCheckpoint { backend_seqs } }
        timeout 10 s ⇒ log warn, proceed without checkpoint (no replay)

    threads_to_check = backend_seqs.keys() ∪ local_threads_with_events
    for thread_id in threads_to_check (concurrent, prioritised by status):
        backend_seq = backend_seqs.get(thread_id).unwrap_or(0)
        local_seq   = SELECT last_seq FROM threads WHERE thread_id = ?

        if backend_seq >= local_seq:
            # backend is ahead — should not happen; log warn, skip
            continue
        else:
            replay(thread_id, backend_seq + 1, local_seq)

replay(thread_id, from_seq, to_seq):
    rows = SELECT seq, payload, ts_ms FROM events
           WHERE thread_id = ? AND seq BETWEEN from_seq AND to_seq
           ORDER BY seq ASC                       -- streamed, paged 1 000 rows
    expected = from_seq..=to_seq
    actual   = rows.map(|r| r.seq)
    gaps     = expected.difference(actual)

    if gaps.is_empty():
        for row in rows: relay_out_tx.send(Envelope::Ingest{...})
    else:
        recovered = jsonl_recover(thread_id, gaps)?     // §9.3
        if recovered.complete:
            INSERT recovered into events with source='jsonl_recovery'
            (allocates new seq numbers — see §9.3 caveat)
            re-SELECT and emit
        else:
            emit what we have, mark thread incomplete, log error
```

Per-thread reconciliation runs concurrently (priority order: `running` / `idle` first, then `suspended`, then `closed`). Until a thread's reconciliation finishes, **new live ingests for that thread queue in the EventWriter's input channel**; the writer drains them in seq order after replay. Backend therefore observes a strict monotonic seq stream per thread during the alignment window.

### 9.3 JSONL fallback

```rust
fn jsonl_recover(
    thread_id: &ThreadId,
    missing_ranges: &[Range<u64>],
) -> Result<RecoveryOutcome>;
```

Procedure:

1. `SELECT codex_session_id FROM threads WHERE thread_id = ?`
   - `NULL` ⇒ abort recovery (no codex rollout to consult).
2. Locate `$HOME/.codex/sessions/{codex_session_id}.jsonl`.
   - File missing or unreadable ⇒ abort.
3. Stream-parse each line as a codex event JSON.
4. Map each codex event to `RawIngest { agent, thread_id, payload, ts_ms }`.
5. Submit via a dedicated `EventWriter::write_recovery(events)` entry that sets `source='jsonl_recovery'`. **Events receive *new* seq numbers** appended after `last_seq` — same single-writer pipeline as live ingest, only the source column differs.

**Caveat — missing seq are not back-filled.** The original gap (e.g. seq 60..69) remains permanently empty in `events`; recovered content lands at fresh seqs (e.g. 101..110). Backend therefore sees sparse history in the original range but the conversational content is intact under new seqs. UI sees a continuous conversation because it sorts by `ts_ms`, not seq.

If a future requirement demands strictly continuous seq (e.g. for fast index lookup), a follow-up backend-side change can write `tombstone` rows for unrecoverable gaps. Not in this spec (§2 OOS).

JSONL fallback triggers **only** when **all** of:

1. Reconciliation detects a true gap inside `events` for a thread.
2. `threads.codex_session_id IS NOT NULL`.
3. `$HOME/.codex/sessions/{codex_session_id}.jsonl` exists and is readable.

Live ingest, lazy hydration, and resume flows do **not** read the JSONL file.

### 9.4 Edge cases

| Case | Behaviour |
|---|---|
| Backend checkpoint contains `thread_id` daemon does not know | Log warn (protocol misalignment), skip that entry. |
| Daemon has thread but backend checkpoint omits it | Treat as `backend_seq = 0`, full replay. Common after backend rollback. |
| Daemon has thread in `closed` status | Still reconcile; status does not affect events table contents. |
| Reconnect storm | Each reconnect re-reconciles; idempotent thanks to backend dedupe. Performance unattractive but correct. |
| Thread with very many events (e.g. 10 000+) | Streamed SELECT, 1 000-row pages, send-while-reading; bounded memory. |
| JSONL parse error mid-file | Use successfully-parsed prefix; remainder marked incomplete; log error. |
| Backend never sends checkpoint within 10 s | Proceed without replay; live ingests still flow normally. |

### 9.5 Out-of-protocol additions explicitly rejected

- Per-event ack from backend.
- Daemon-side outbox table (write-ahead already provides authority).
- `request_id` / response correlation for checkpoint.

## 10. FFI Surface Impact

### 10.1 UniFFI (`crates/minos-ffi-uniffi/src/lib.rs`)

| Before | After |
|---|---|
| `start_agent(agent, mode)` | `start_agent(agent, workspace, mode)` |
| `send_user_message(thread_id, text)` | unchanged signature; semantics extended to accept `Suspended` |
| `stop_agent()` | **deleted** |
| — | `interrupt_thread(thread_id)` |
| — | `close_thread(thread_id)` |
| — | `list_threads(filter)` / `get_thread(thread_id)` |
| `state_stream() -> AgentState` | `thread_state_stream(thread_id) -> ThreadState`, `manager_event_stream() -> ManagerEvent` |
| `current_state() -> AgentState` | `get_thread(thread_id) -> { state, ... }` |
| `ingest_stream()` | unchanged (consumer filters by `thread_id`) |

`apps/macos/Minos/Application/AppState+Agent.swift` and `apps/macos/MinosTests/Application/AgentStateTests.swift` will not compile against the new FFI; rewriting them is OOS but lands as follow-up commits on the same branch.

### 10.2 frb (`crates/minos-ffi-frb/src/api/minos.rs`)

Same shape as 10.1, plus the host-rename surface from §6.1 (`forget_host`, `list_paired_hosts`, `set_active_host`, `HostSummaryDto`). Dart-side viewmodels (`apps/mobile/lib/`) follow up.

## 11. Delivery

Single ship on `feature/mobile-auth-and-agent-session`. The work is large but cohesive; splitting into separate PRs adds coordination overhead without reducing risk. UI viewmodel updates (macOS Swift, Flutter Dart) follow as later commits on the same branch and are not gated by this spec.

Implementation order within the branch (each step ends green on `cargo xtask check-all`):

1. **Paths + naming sweep** — §5, §6. Mostly mechanical; touches lots of files but no new logic. Land with full SQL migration `0013`.
2. **AgentManager + LocalStore + EventWriter** — §7, §8. Replaces `AgentRuntime` with `AgentManager`; FFI signatures change. macOS / Flutter apps stop compiling against the new FFI at this commit.
3. **Reconciliator + JSONL fallback** — §9. Includes backend-side first-frame `IngestCheckpoint` emission.
4. **UI viewmodel updates** (OOS for this spec; documented for the branch reviewer).

## 12. Test Strategy

### 12.1 New unit tests

- `manager::transitions` — every state transition; reject illegal transitions.
- `manager::lifecycle` — `max_instances` cap, LRU evict, `idle_timeout` (virtual clock).
- `store::migrations` — schema apply, idempotency.
- `store::write_ahead` — commit-then-send order, batching window.
- `store::startup_recovery` — `status NOT IN ('closed','suspended')` reset.
- `store::lazy_hydrate` — `ThreadHandle` materialisation on first ref.
- `event_writer::failure` — commit failure increments counter; INSERT collision panics.
- `reconciliator::checkpoint` — backend ahead, equal, behind cases.
- `reconciliator::gap_detection` — gaps trigger fallback.
- `jsonl_recover::*` — happy path, file missing, mid-file parse error.
- `paths::*` — env override, default, subdir creation.

### 12.2 New integration tests

`crates/minos-agent-runtime/tests/multi_session_smoke.rs`:

```
1. Create two workspaces, two threads each (4 threads, 2 instances).
2. send_user_message on each; verify ingest events.
3. interrupt_thread one; verify Suspended state.
4. send_user_message on the suspended thread → Resuming → Idle.
5. Advance virtual clock past idle_timeout; verify InstanceReaped.
6. send_user_message after reap; verify instance respawn + Resume.
7. close_thread; verify terminal state.
```

`crates/minos-daemon/tests/reconciliation_integration.rs`:

```
1. Start daemon with mock backend WS; ingest 100 events.
2. Disconnect WS.
3. Restart daemon.
4. Mock backend sends IngestCheckpoint{ thread_x: 50 }.
5. Assert daemon resends seq 51..100 in order.
6. DELETE rows seq 60..70 to simulate gap.
7. Reconnect; assert jsonl_recover triggers and emits recovered content
   with source='jsonl_recovery' and new seq numbers.
```

### 12.3 Existing test updates

- `crates/minos-agent-runtime/tests/*` — rewrite (single-session assumption removed).
- `crates/minos-daemon/tests/agent_smoke.rs`, `agent_ingest_smoke.rs` — route through `AgentManager`.
- `crates/minos-protocol` snapshot tests — refresh for `IngestCheckpoint` and renamed types.
- `crates/minos-backend/tests/ws_devices.rs` — assert backend sends `IngestCheckpoint` as the first frame after auth.

### 12.4 Lint

`cargo xtask check-all` extended with:
- existing `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`
- new `cargo xtask lint-naming` (§6.6)
- existing frb mirror drift check

## 13. Risks

| Risk | Mitigation |
|---|---|
| codex `app-server` resume-from-session API surface unverified | Before §11 step 2 lands, write a 30-line integration test that issues `start_thread { resume_from_session: <uuid> }` against a real `codex app-server` and asserts it accepts the param + replays internal context. If codex does not support this, the implicit-resume design (§7.4) needs a redesign — likely "treat resume as a fresh thread that links back via `parent_thread_id` metadata". |
| Multi-thread codex `app-server` concurrency stability | Integration test runs 5 concurrent threads × 10 turns overnight before merge. |
| SQLite WAL on cloud-synced filesystems (iCloud Drive, Dropbox) | Doc: do not place `~/.minos` on sync drives; daemon checks volume type at startup and warns. |
| `~/.codex/sessions/` path on Windows differs | Section §8 / §9 on Windows defers with the same Linux/Windows secret carve-out (§2 OOS). |
| FFI changes leave `apps/macos` and `apps/mobile` broken between commits 2 and 4 of §11 | Accepted; no public release during the branch. |
| `commit` failure ⇒ silent event loss | Counter + alert; dogfood one week before any external release. |
| Memory headroom with `max_instances=8` (~400-1 200 MB at peak) | Configurable; LRU evict bounds growth; documented in operator notes. |

## 14. Appendix — File Inventory (creation/edit)

**New files**

- `crates/minos-agent-runtime/src/manager.rs`
- `crates/minos-agent-runtime/src/instance.rs`
- `crates/minos-agent-runtime/src/thread_handle.rs`
- `crates/minos-agent-runtime/src/state_machine.rs`
- `crates/minos-agent-runtime/tests/multi_session_smoke.rs`
- `crates/minos-daemon/src/store.rs` (or `store/mod.rs`)
- `crates/minos-daemon/src/event_writer.rs`
- `crates/minos-daemon/src/reconciliator.rs`
- `crates/minos-daemon/src/jsonl_recover.rs`
- `crates/minos-daemon/migrations/0001_initial.sql`
- `crates/minos-daemon/tests/reconciliation_integration.rs`
- `crates/minos-backend/migrations/0013_rename_account_mac_to_host.sql`

**Edited files (non-exhaustive)**

- `crates/minos-agent-runtime/src/lib.rs` — re-exports
- `crates/minos-agent-runtime/src/runtime.rs` — replaced by manager
- `crates/minos-agent-runtime/src/state.rs` — `AgentState` → `ThreadState`
- `crates/minos-agent-runtime/src/ingest.rs` — seq sourced from manager
- `crates/minos-daemon/src/agent.rs` — glue over `AgentManager`
- `crates/minos-daemon/src/agent_ingest.rs` — replaced by `EventWriter` flow
- `crates/minos-daemon/src/paths.rs` — new helpers
- `crates/minos-daemon/src/local_state.rs` — uses `state_dir()`
- `crates/minos-daemon/src/main.rs` — drops `platform_data_dir` and `MINOS_DATA_DIR/MINOS_LOG_DIR`
- `crates/minos-daemon/src/logging.rs` — uses `logs_dir()`
- `crates/minos-daemon/src/relay_client.rs` — Reconciliator wiring
- `crates/minos-protocol/src/messages.rs` — Mac→Host rename, new RPC types
- `crates/minos-protocol/src/envelope.rs` — `EventKind::IngestCheckpoint`
- `crates/minos-protocol/src/rpc.rs` — RPC trait surgery
- `crates/minos-domain/src/role.rs` — `IosClient` → `MobileClient`
- `crates/minos-mobile/src/store.rs`, `client.rs`, `http.rs` — Mac→Host rename, role string
- `crates/minos-ffi-uniffi/src/lib.rs` — new FFI signatures
- `crates/minos-ffi-frb/src/api/minos.rs` — new FFI signatures
- `crates/minos-backend/src/store/account_*pairings.rs` — rename module
- `crates/minos-backend/src/http/v1/me.rs` — `/v1/me/hosts` route
- `crates/minos-backend/src/http/ws_devices.rs` — emit `IngestCheckpoint` first
- `xtask/src/main.rs` — `lint-naming` command
