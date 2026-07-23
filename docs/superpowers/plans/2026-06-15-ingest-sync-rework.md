# Rework Agent Ingest Sync

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` for parallel implementation or `superpowers:executing-plans` for single-worker execution. Track progress with the checkbox tasks below.

## Breaking Change Notice

This rework intentionally breaks the current host ingest wire shape and daemon local ingest internals. Minos is in latest-only development, so do not add compatibility shims for the old `IngestCheckpoint` / `Reconciliator` flow or the old `EventWriter -> relay_out` path.

Migration steps:

1. Replace legacy `ClientFrame::HostStreamEvent` ingest upload with `HostIngestLiveBatch`.
2. Replace legacy reconnect reconciliation with `HostGapManifest` plus backend-driven `PullIngestRange`.
3. Remove daemon `Reconciliator` and its tests once the new manifest/pull path is wired.
4. Update backend raw ingest storage to be idempotent on host-local sequence identity.
5. Update clients and HTTP read surfaces to understand partial history while the backend pulls host-local gaps.

## Execution Status — 2026-06-15

Implemented in the current worktree:

- Protocol frame types for `HostIngestLiveBatch`, `HostGapManifest`, `HostIngestPullResponse`, `HostIngestAck`, `PullIngestRange`, and `PullAck`.
- `IngestSink.emit()` is ordered and bounded; the old `try_send Full -> unbounded spawn` branch is removed.
- daemon ingest now flows through `IngestCoalescer -> IngestChunk -> EventWriter + IngestSyncHandle`.
- `EventWriter` is local-only and no longer owns or awaits relay outbound senders.
- daemon relay outbound is split into control/live/backfill lanes, with dispatch priority `control -> live -> backfill`.
- WS disconnect no longer blocks local SQLite persistence; live upload marks local dirty ranges instead of buffering payload backlog.
- Reconnect sends metadata-only `HostGapManifest`; pull requests are served by reading SQLite and returning `HostIngestPullResponse`.
- backend `/ws/host` accepts live batches, manifests, and pull responses, and writes host ingest chunks idempotently.
- daemon `Reconciliator` and protocol `IngestCheckpoint` have been removed.

Still intentionally not complete in this pass:

- Deep semantic coalescing of adjacent token deltas. The current `IngestCoalescer` assigns seq/projection/checksum and creates canonical chunks, but it does not yet merge multiple provider delta events into a larger chunk.
- Client-opened-history trigger. The backend records manifest metadata, immediately pulls reported manifest ranges, and accepts pull responses.
- Client partial-history response fields and UI restoring state.
- Bounded retry / ingest-degraded state for terminal local SQLite failures.

## Feasibility Assessment

The rework is feasible because the repo already has all required primitives: agent raw ingest, daemon SQLite persistence, a single host WebSocket, backend raw event storage, formal realtime topics, and client subscription replay. The unsafe part is the current coupling: `EventWriter` commits SQLite and then awaits relay mpsc, while reconnect reconciliation is dead. The new design keeps normal online upload in memory, uses SQLite only as the local durable source of truth during disconnect and pull response, and lets the backend schedule historical backfill. Fully feasible.

## Current Surface Inventory

- `crates/minos-agent-runtime/src/manager.rs::IngestSink::emit` -- currently uses `try_send` plus unbounded `tokio::spawn` on Full.
- `crates/minos-agent-runtime/src/{codex_client,claude_driver,gemini_driver,opencode_driver,pty_agent,manager}.rs` -- current raw ingest call sites that must await ordered durable emission.
- `crates/minos-daemon/src/store/event_writer.rs::EventWriter` -- currently assigns seq, writes SQLite, builds projection, then awaits relay send.
- `crates/minos-daemon/src/agent.rs::AgentGlue` -- currently bridges `RawIngest -> EventWriter` and broadcasts `LocalIngestFrame` after commit.
- `crates/minos-daemon/src/handle.rs` -- currently builds `agent_out_tx -> relay.outbound_sender()` non-durable bridge.
- `crates/minos-daemon/src/relay_client.rs` -- currently owns one FIFO outbound queue and does not route any checkpoint/pull sync.
- `crates/minos-daemon/src/reconciliator.rs` -- legacy local replay implementation; currently not integrated into production routing.
- `crates/minos-protocol/src/realtime.rs` -- shared WebSocket frame schema.
- `crates/minos-backend/src/realtime/gateway.rs` -- current host gateway handling for `HostStreamEvent`.
- `crates/minos-backend/src/store/raw_events.rs` -- current raw event insert/dedup behavior.
- `crates/minos-backend/src/realtime.rs` -- current live fanout and durable replay implementation.
- `crates/minos-backend/src/agent_sessions/use_case.rs` and `crates/minos-backend/src/http/v1/agent_sessions.rs` -- history read surfaces that must expose partial-history state.
- `docs/architecture-{daemon,backend,business-flow}.md` -- architecture docs that must be updated with the new sync model.

## Design

### Target Flow

Normal online:

```text
Agent CLI output
  -> ordered IngestSink
  -> IngestCoalescer
       - coalesce high-frequency deltas
       - assign host-local seq before fanout
       - produce canonical IngestChunk
  -> ChunkBus
       -> LocalPersistWorker: batch write SQLite
       -> LiveUploadWorker: realtime upload over WS, best-effort non-blocking
  -> Backend stores idempotently and fans out to clients
```

WS disconnected:

```text
Agent continues
  -> IngestCoalescer continues assigning seq
  -> LocalPersistWorker continues writing SQLite
  -> LiveUploadWorker pauses upload and records dirty ranges only
  -> No payload backlog is held in memory
```

WS reconnected:

```text
Host sends HostGapManifest metadata only
  -> backend records: "thread X has local range A..B available on host"
  -> new live chunks keep uploading first
  -> backend sends PullIngestRange for reported manifest ranges
  -> host reads SQLite range and responds with HostIngestPullResponse chunks
  -> backend sends HostIngestAck / PullAck and advances watermarks
```

### Key Design Decisions

1. **Assign seq before local/live fanout.** `EventWriter` must stop being the only seq allocator because live upload and local persistence need the same identity. The alternative of letting DB assign seq and then broadcasting committed frames serializes live upload behind SQLite commit latency.

2. **Use one canonical `IngestChunk` for both persistence and upload.** `LocalIngestFrame` only has UI projection and is not enough for backend raw event storage. The live uploader and pull responder must use chunks that include raw payload metadata, projection, timestamps, checksum, and size.

3. **Normal online upload does not read from SQLite.** Live upload sends the in-memory chunk produced by the coalescer. SQLite is read only for backend pull responses, local UI catch-up, and diagnostics.

4. **Live upload never awaits a full relay FIFO.** If the relay is disconnected or its live lane is full, the uploader marks dirty ranges and drops the live attempt. This prevents relay backpressure from reaching SQLite persistence or the agent event pump.

5. **Local persistence can still backpressure the agent.** Relay/network backpressure must not affect the agent. Local durable persistence failure is different: if SQLite cannot accept data, the daemon must enter ingest-degraded state and surface the failure instead of silently dropping events.

6. **Reconnect sends metadata, not history payload.** A long offline backlog must not occupy bandwidth ahead of new active sessions. The backend decides which missing ranges to pull based on client demand, idle budget, audit, or priority.

7. **Backend owns historical backfill scheduling.** The backend knows which clients are online and which session they are viewing. It should pull "client-opened history" before idle backfill and pause pull traffic when live traffic is present.

8. **All ingest writes are idempotent on stable host-local identity.** Use `(host_device_id, session_id, seq)` or `event_id` as the uniqueness key. Same key with different payload is an invariant violation, not a reason to assign a new backend seq.

9. **Clients see partial history explicitly.** If backend has only part of a session and host has missing ranges, read APIs return available data plus `history_state = partial`. The UI can show that older/missing content is being restored.

10. **Delete the old Reconciliator instead of patching it.** It is built around a backend checkpoint frame the current gateway intentionally does not emit. Keeping it would create a false safety story.

### Protocol Types

Add the shared wire types in `crates/minos-protocol/src/realtime.rs`. Names below are canonical for this plan; use enum variants, not string priorities, to keep routing typed.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PullPriority {
    LiveCritical,
    ClientOpenedHistory,
    IdleBackfill,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PullReason {
    ClientOpenedHistory,
    IdleBackfill,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeqRange {
    pub from: u64,
    pub to: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostIngestChunk {
    pub event_id: String,
    pub session_id: String,
    pub seq: u64,
    pub agent: minos_domain::AgentName,
    pub kind: String,
    pub payload: serde_json::Value,
    pub projection: Vec<minos_ui_protocol::UiEventMessage>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub byte_len: u64,
    pub checksum_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostIngestLiveBatch {
    pub batch_id: String,
    pub host_id: minos_domain::DeviceId,
    pub chunks: Vec<HostIngestChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionGapManifest {
    pub session_id: String,
    pub backend_acked_seq: u64,
    pub local_from_seq: u64,
    pub local_to_seq: u64,
    pub missing_ranges: Vec<SeqRange>,
    pub bytes: u64,
    pub event_count: u64,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostGapManifest {
    pub manifest_id: String,
    pub host_id: minos_domain::DeviceId,
    pub sessions: Vec<SessionGapManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostIngestPullResponse {
    pub request_id: String,
    pub session_id: String,
    pub from_seq: u64,
    pub to_seq: u64,
    pub chunks: Vec<HostIngestChunk>,
    pub has_more: bool,
}
```

Frame additions:

```rust
pub enum ClientFrame {
    HostIngestLiveBatch { batch: HostIngestLiveBatch },
    HostGapManifest { manifest: HostGapManifest },
    HostIngestPullResponse { response: HostIngestPullResponse },
    // existing variants
}

pub enum ServerFrame {
    HostIngestAck {
        session_id: String,
        accepted_to_seq: u64,
        batch_id: Option<String>,
    },
    PullIngestRange {
        request_id: String,
        session_id: String,
        from_seq: u64,
        to_seq: u64,
        max_bytes: u64,
        priority: PullPriority,
        reason: PullReason,
    },
    PullAck {
        request_id: String,
        session_id: String,
        accepted_to_seq: u64,
    },
    // existing variants
}
```

### Daemon Local Types

Create `crates/minos-daemon/src/ingest_chunk.rs` or place these in `sync.rs` if the module stays small.

```rust
pub struct IngestChunk {
    pub event_id: String,
    pub session_id: String,
    pub seq: u64,
    pub agent: minos_domain::AgentName,
    pub kind: String,
    pub raw_payload: serde_json::Value,
    pub projection: Vec<minos_ui_protocol::UiEventMessage>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub byte_len: u64,
    pub checksum_sha256: String,
}

pub struct ThreadSyncWatermark {
    pub session_id: String,
    pub local_last_seq: u64,
    pub local_persisted_seq: u64,
    pub backend_acked_seq: u64,
    pub dirty_ranges: Vec<SeqRange>,
}
```

### Backend Partial History State

Backend must persist host availability separately from raw events:

```sql
CREATE TABLE thread_sync_state (
    session_id TEXT NOT NULL,
    host_device_id TEXT NOT NULL,
    backend_acked_seq INTEGER NOT NULL DEFAULT 0,
    host_local_from_seq INTEGER NOT NULL DEFAULT 0,
    host_local_to_seq INTEGER NOT NULL DEFAULT 0,
    missing_ranges_json TEXT NOT NULL DEFAULT '[]',
    available_on_host BOOLEAN NOT NULL DEFAULT FALSE,
    running BOOLEAN NOT NULL DEFAULT FALSE,
    byte_count INTEGER NOT NULL DEFAULT 0,
    event_count INTEGER NOT NULL DEFAULT 0,
    first_ts_ms INTEGER,
    last_ts_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (session_id, host_device_id)
);
```

Raw event idempotency must include host identity:

```sql
-- Shape may be adapted to existing migrations, but the invariant is required.
CREATE UNIQUE INDEX raw_events_host_thread_seq_idx
ON raw_events(host_device_id, session_id, seq);
```

## Phased Implementation

## Phase 1: Add Protocol Frames And Storage Shape

**File: `crates/minos-protocol/src/realtime.rs`**

- Add `HostIngestChunk`, `HostIngestLiveBatch`, `HostGapManifest`, `SessionGapManifest`, `SeqRange`, `HostIngestPullResponse`.
- Add `PullPriority` and `PullReason` enums.
- Add `ClientFrame::{HostIngestLiveBatch, HostGapManifest, HostIngestPullResponse}`.
- Add `ServerFrame::{HostIngestAck, PullIngestRange, PullAck}`.
- Add roundtrip tests for all new frames.

**File: `crates/minos-backend/migrations/*`**

- Add `thread_sync_state`.
- Add host identity to `raw_events` if missing.
- Add unique constraint for `(host_device_id, session_id, seq)`.

**Verification**

- [ ] `cargo test -p minos-protocol realtime`
- [ ] `cargo build -p minos-backend`

## Phase 2: Make IngestSink Ordered And Explicitly Fallible

**File: `crates/minos-agent-runtime/src/manager.rs`**

- Change durable `emit` to async and return `Result<(), IngestClosed>`.
- Remove Full-branch detached `tokio::spawn`.
- On Full, await the durable mpsc send in caller order.
- Keep broadcast non-authoritative; durable path decides whether the event is accepted.

Do not use `blocking_send`: current call sites run inside tokio tasks.

**Files: `crates/minos-agent-runtime/src/*.rs` call sites**

- Update all `.emit(...)` call sites to `.emit(...).await`.
- Where an event cannot be persisted because the durable sink is closed, propagate a runtime error or mark the session degraded. Do not log-and-continue as if the event was handled.

**Tests**

- [ ] Add a concurrent drain test: producer awaits `emit`, receiver drains concurrently, assert ordered `i = 0..N`.
- [ ] Add a closed durable sink test: `emit` returns `Err(IngestClosed)`.
- [ ] `cargo test -p minos-agent-runtime ingest_sink`

Important correction from the previous draft: do not add `RawIngest::as_inline_json() -> Option<&Value>`. Parsing JSON returns an owned `Value`; tests should deserialize the inline bytes directly or inspect `RawIngest` fields already available.

## Phase 3: Introduce IngestCoalescer And ChunkBus

**File: `crates/minos-daemon/src/ingest_coalescer.rs`**

- Add per-session sequencer initialized from local `sessions.last_seq`.
- Convert `RawIngest` into canonical `IngestChunk`.
- Coalesce only safe high-frequency event classes:
  - assistant text delta
  - reasoning text delta
  - tool stdout/stderr fragments
  - progress/status patches
- Flush immediately for boundary events:
  - message started/completed
  - tool call started/completed
  - approval requested/resolved
  - error
  - turn/session ended
- Flush by time window, max event count, or max bytes.

**File: `crates/minos-daemon/src/agent.rs`**

- Replace the current `RawIngest -> EventWriter` bridge with `RawIngest -> IngestCoalescer -> ChunkBus`.
- Expose a broadcast or mpsc stream of `IngestChunk` to both local persistence and live upload.
- Keep `persisted_ingest_stream()` for TUI, but make lag recovery read SQLite by seq.

**Tests**

- [ ] Coalesces adjacent text deltas into one chunk.
- [ ] Boundary events flush existing buffered deltas.
- [ ] Seq assignment is monotonic across chunks and survives daemon restart initialization.
- [ ] `cargo test -p minos-daemon ingest_coalescer`

## Phase 4: Make EventWriter Local-Only And Preassigned-Seq Aware

**File: `crates/minos-daemon/src/store/event_writer.rs`**

- Remove relay sender from `EventWriter::spawn`.
- Accept `IngestChunk` or a write job with preassigned seq.
- Stop selecting `last_seq + 1` inside the writer for live chunks; the coalescer owns live seq.
- Keep `write_recovery` only if still needed for local JSONL repair, but do not use it for backend sync.
- Write raw payload metadata, projection, checksum, byte length, and timestamps.
- Batch SQLite writes.
- Retry transient SQLite failures with bounded retry.
- On terminal failure, surface ingest-degraded state; do not drop and continue.

**File: `crates/minos-daemon/src/store/mod.rs`**

- Add `read_ingest_chunks(session_id, from_seq, to_seq, max_bytes)` for pull response.
- The method must return raw payload, not only projection JSON.
- If a local row points to an artifact body, resolve the artifact or return a typed pull error so the backend can retry later.

**Tests**

- [ ] Writer no longer takes relay sender.
- [ ] Writer inserts preassigned seq exactly.
- [ ] Batch writes keep same-thread seq ordering.
- [ ] Pull read returns raw payload and projection for inline and artifact-backed rows.
- [ ] `cargo test -p minos-daemon --lib store::event_writer`

## Phase 5: Replace Relay FIFO With Priority Outbound Lanes

**File: `crates/minos-daemon/src/relay_client.rs`**

- Replace the single undifferentiated outbound path for ingest sync with priority lanes:
  - control lane: subscribe, ping, manifest, ack-critical frames
  - live ingest lane: `HostIngestLiveBatch`
  - backfill lane: `HostIngestPullResponse`
- `dispatch_loop` drains lanes in priority order.
- Live upload must use `try_send` or bounded non-blocking enqueue. If the live lane is full, mark dirty and skip that live attempt.
- Backfill lane must be token-bucket limited and paused when live lane is non-empty.

**File: `crates/minos-daemon/src/sync.rs`**

- Add `SyncState` with `local_last_seq`, `local_persisted_seq`, `backend_acked_seq`, and dirty ranges per session.
- Add `LiveUploadWorker`:
  - consumes `IngestChunk` from ChunkBus
  - if connected and live lane accepts, sends `HostIngestLiveBatch`
  - if disconnected/full, records dirty range and does not buffer payload
  - handles `HostIngestAck` to advance backend ack watermark
- Add `ManifestWorker`:
  - on reconnect, builds `HostGapManifest` from `SyncState` and local store metadata
  - sends metadata only
- Add `PullResponseWorker`:
  - handles `PullIngestRange`
  - reads SQLite via `read_ingest_chunks`
  - sends `HostIngestPullResponse` through backfill lane only
  - respects `max_bytes`

**File: `crates/minos-daemon/src/handle.rs`**

- Delete `agent_out_tx -> relay.outbound_sender()` bridge.
- Wire ChunkBus to `LocalPersistWorker` and `LiveUploadWorker`.
- Wire pull request/ack channels from `RelayClient` to `sync.rs`.

**Tests**

- [ ] Relay disconnected does not block local writer.
- [ ] Full live lane marks dirty instead of awaiting.
- [ ] Backfill pauses while live lane has frames.
- [ ] Reconnect emits manifest before any backfill payload.
- [ ] `cargo test -p minos-daemon --test sync_live_upload`

## Phase 6: Wire Manifest And Pull Routing On Host

**File: `crates/minos-daemon/src/relay_client.rs`**

- Route `ServerFrame::HostIngestAck` to `SyncState::mark_acked`.
- Route `ServerFrame::PullIngestRange` to `PullResponseWorker`.
- Route `ServerFrame::PullAck` to `SyncState::mark_acked`.
- On host topic `SubscribeAck`, notify `ManifestWorker`.
- Remove `PersistenceCtx.reconciliator` and `DispatchCtx.reconciliator`.

**File: `crates/minos-daemon/src/reconciliator.rs`**

- Delete after new sync routing is compiled.

**Tests**

- [ ] `HostIngestAck` advances ack watermark.
- [ ] `PullAck` advances ack watermark.
- [ ] `PullIngestRange` causes SQLite range read and response.
- [ ] Reconciliator tests are deleted or replaced with manifest/pull tests.
- [ ] `cargo test -p minos-daemon relay_client`

## Phase 7: Backend Live Ingest, Manifest, Pull, And Partial History

**File: `crates/minos-backend/src/realtime/gateway.rs`**

- Handle `ClientFrame::HostIngestLiveBatch`.
- Handle `ClientFrame::HostGapManifest`.
- Handle `ClientFrame::HostIngestPullResponse`.
- Validate `manifest.host_id == upgrade.device_id`.
- Validate every chunk belongs to a session owned by that host; return an error frame for mismatches instead of silently dropping.
- Store live and pulled chunks through the same idempotent ingestion function.
- Send `HostIngestAck` for live batches and `PullAck` for pull responses.

**File: `crates/minos-backend/src/store/raw_events.rs`**

- Add `insert_host_ingest_chunk`.
- Enforce idempotency on `(host_device_id, session_id, seq)` or `event_id`.
- Same key + same checksum is duplicate success.
- Same key + different checksum is a hard invariant error.
- Remove the current "same seq different payload gets assigned next seq" behavior for host-ingest chunks.

**File: `crates/minos-backend/src/store/thread_sync_state.rs`**

- New store module for manifest state.
- Upsert host-available ranges from manifest.
- Advance ack watermark after live or pull persistence.
- Mark ranges unavailable when host disconnects permanently or session is deleted.

**File: `crates/minos-backend/src/realtime.rs`**

- Keep live fanout immediate for live chunks.
- For pulled historical chunks, fan out only to clients currently waiting on that missing range.
- Add scheduler hooks for pull priority:
  - `client_opened_history`
  - `idle_backfill`
  - `audit`

**File: `crates/minos-backend/src/agent_sessions/use_case.rs`**

- When reading turns/events, detect gaps from `thread_sync_state`.
- Return available events plus partial-history metadata.
- Trigger `PullIngestRange` when a client opens a session with host-available missing ranges.

**File: `crates/minos-backend/src/http/v1/agent_sessions.rs`**

- Add response fields for partial history, for example:

```rust
pub struct HistorySyncStateResponse {
    pub state: String, // "complete" | "partial" | "restoring" | "unavailable"
    pub missing_ranges: Vec<SeqRange>,
    pub available_on_host: bool,
}
```

**Tests**

- [ ] Live batch insert sends ack and fans out.
- [ ] Duplicate live + pulled chunks do not duplicate raw rows or UI events.
- [ ] Same seq with different checksum fails.
- [ ] Manifest records partial history without pulling payload immediately.
- [ ] Client history read triggers pull only for requested session/range.
- [ ] `cargo test -p minos-backend ws_gateway ingest_roundtrip v1_agent_sessions`

## Phase 8: Client And UI Partial-History Handling

**Files: `apps/mobile/lib/**` and `crates/minos-mobile/src/**`**

- Parse new partial-history fields from agent session read responses.
- Keep realtime display focused on new live data.
- Show restoring state only for missing historical ranges.
- Do not block live updates while history is restoring.

**Files: `apps/web/**` if web session history consumes the affected API**

- Mirror mobile partial-history behavior.

**Tests**

- [ ] Unit test response parsing for complete/partial/restoring/unavailable.
- [ ] Widget or integration coverage only if the existing app test stack already supports it.

## Phase 9: Verification And Documentation

**Commands**

- [ ] `cargo test -p minos-agent-runtime`
- [ ] `cargo test -p minos-daemon`
- [ ] `cargo test -p minos-protocol`
- [ ] `cargo test -p minos-backend`
- [ ] `cargo clippy --workspace`

**New integration tests**

- [ ] Relay disconnect does not block SQLite commits.
- [ ] Reconnect sends manifest metadata only.
- [ ] Backend does not auto-pull the full backlog on reconnect.
- [ ] Backend pull response does not starve live upload.
- [ ] Backend live/pull overlap is idempotent.
- [ ] Client read gets partial-history state and later observes restored history.

**Docs**

- [ ] `docs/architecture-daemon.md` -- describe coalescer, chunk bus, local writer, sync workers, and priority outbound lanes.
- [ ] `docs/architecture-backend.md` -- describe live ingest batch, manifest, pull scheduler, partial history state.
- [ ] `docs/architecture-business-flow.md` -- describe online, disconnected, reconnect manifest, backend pull, client partial-history flow.
- [ ] Remove stale text that claims `IngestCheckpoint/Reconciliator` is the production safety mechanism.

## Architectural Notes

- Semver impact: breaking wire protocol and storage schema change.
- No compatibility layer: latest-only development policy applies.
- Relay backpressure no longer blocks local persistence or agent ingest.
- Local persistence failure still blocks/degrades agent ingest because otherwise the host would lose its durable source of truth.
- Normal online upload is memory-to-WS from `IngestChunk`; SQLite read happens only for pull/backfill and local catch-up.
- Backfill is explicitly lower priority than live data.
- Manifest is metadata-only and can be resent safely.
- Pull requests are backend scheduled and may be retried safely.
- The backend must keep enough state to know "available on host" even before it has pulled the payload.
- The old `Reconciliator` module and tests should be deleted, not patched.

## Explicit Corrections To The Previous Draft

- Do not defer `IngestCoalescer` as future work. It is required because seq must be assigned before local/live fanout.
- Do not make `LiveUploadWorker` send `LocalIngestFrame`; it lacks raw payload and cannot rebuild backend raw events.
- Do not use `relay_out.send(...).await` from live or pull workers. Use priority lanes with non-blocking live enqueue and throttled backfill.
- Do not use string priorities. Use `PullPriority` and `PullReason` enums.
- Do not add `RawIngest::as_inline_json() -> Option<&Value>`; that API is invalid for parsed JSON.
- Do not leave `PullAck` as "logged for later". It must update `backend_acked_seq`.
- Do not build manifest byte counts by reading every full row when a store aggregate can provide metadata. For very large gaps, manifest building must be bounded.
- Do not silently drop host/session mismatch on the backend. Return an error frame and keep metrics.
- Do not keep the old "same seq different payload gets next seq" behavior for new host-ingest chunks.

## File Change Summary

- `apps/mobile/lib/**` -- parse and render partial-history state if affected by agent session history APIs.
- `apps/web/**` -- parse and render partial-history state if affected by agent session history APIs.
- `crates/minos-agent-runtime/src/manager.rs` -- ordered async durable ingest with explicit error.
- `crates/minos-agent-runtime/src/{codex_client,claude_driver,gemini_driver,opencode_driver,pty_agent}.rs` -- await fallible ingest emission.
- `crates/minos-backend/migrations/*` -- add host ingest idempotency and `thread_sync_state`.
- `crates/minos-backend/src/agent_sessions/use_case.rs` -- expose partial-history state and trigger pulls.
- `crates/minos-backend/src/http/v1/agent_sessions.rs` -- include partial-history response fields.
- `crates/minos-backend/src/realtime.rs` -- live-priority fanout and pull scheduling hooks.
- `crates/minos-backend/src/realtime/gateway.rs` -- handle live batch, manifest, pull response, and acks.
- `crates/minos-backend/src/store/raw_events.rs` -- idempotent host ingest insert with checksum conflict detection.
- `crates/minos-backend/src/store/thread_sync_state.rs` -- new store module for host-available missing ranges.
- `crates/minos-daemon/src/agent.rs` -- bridge raw ingest through coalescer/chunk bus; no log-and-drop on persist failure.
- `crates/minos-daemon/src/handle.rs` -- wire chunk bus, local writer, live uploader, manifest worker, pull responder.
- `crates/minos-daemon/src/ingest_chunk.rs` -- canonical daemon chunk type shared by local writer and upload workers.
- `crates/minos-daemon/src/ingest_coalescer.rs` -- coalesce deltas and assign host-local seq.
- `crates/minos-daemon/src/relay_client.rs` -- route new ack/pull frames and support priority outbound lanes.
- `crates/minos-daemon/src/reconciliator.rs` -- delete.
- `crates/minos-daemon/src/store/event_writer.rs` -- local-only batch writer accepting preassigned chunks.
- `crates/minos-daemon/src/store/mod.rs` -- add chunk range read and metadata aggregation helpers.
- `crates/minos-daemon/src/sync.rs` -- sync watermarks, dirty ranges, live upload, manifest, pull response.
- `crates/minos-protocol/src/realtime.rs` -- new ingest sync protocol types and frame variants.
- `docs/architecture-backend.md` -- backend ingest sync architecture.
- `docs/architecture-business-flow.md` -- reconnect manifest and backend-driven pull flow.
- `docs/architecture-daemon.md` -- daemon coalescer/local/live/pull architecture.
