# Minos · Mobile Migration + Unified UI Protocol — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL — use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Mark each step complete only after its acceptance criterion passes.

**Goal:** Rename `minos-relay` → `minos-backend`; introduce `minos-ui-protocol` (`UiEventMessage` + per-CLI translators); add `Envelope::Ingest` + `EventKind::UiEventMessage` + history LocalRpc methods; rebuild pairing to let the backend hand the CF Access Service Token to the phone via the QR payload; rewrite `minos-mobile` to talk envelopes; land a plain two-page debug viewer on Flutter. Plan ends when the 16-box smoke checklist (spec §12.5) is green on a real iPhone against a real `cloudflared` tunnel.

**Architecture:** `minos-backend` is the single source of truth: it stores raw agent-host events (`raw_events`), runs `minos-ui-protocol::translate_*` on read/fan-out to emit `UiEventMessage` to mobile, and hands out fully-assembled pairing QR payloads (including CF Access tokens from its env vars) to agent-hosts. `minos-agent-runtime` stops translating and becomes an ingest pipe. Mobile becomes an envelope client and a passive consumer of `UiEventMessage` — no chat UI, no input.

**Tech Stack:** Rust workspace (same as plan 04), `axum`, `sqlx` + SQLite, `tokio-tungstenite`, `rstest`, `pretty_assertions`. Flutter 3.41, `flutter_rust_bridge` 2, `flutter_riverpod`, `shadcn_ui`, `mobile_scanner`, `flutter_secure_storage` ^9.2.

**Reference documents (READ BEFORE CODING):**

1. `docs/superpowers/specs/mobile-migration-and-ui-protocol-design.md` — authoritative. Every task traces to a section.
2. `docs/superpowers/specs/minos-relay-backend-design.md` — the prior art this spec partially supersedes (§6.1 pairing RPC and §9.4 CF token location).
3. `docs/superpowers/specs/codex-app-server-integration-design.md` — the agent-runtime shape this plan rewires.
4. `docs/adr/0011-broker-envelope-protocol.md` — envelope discriminator policy.
5. `docs/adr/0012-sqlite-via-sqlx.md` — migrations + offline prepare.

**Working directory:** This plan runs in a fresh worktree at `../minos-worktrees/mobile-and-ui-protocol/` on branch `feat/mobile-and-ui-protocol`. Task A1 creates it.

**Version drift policy:** Versions in the spec are accurate as of 2026-04-24. If `cargo add` or `flutter pub add` resolves higher minor versions, prefer the resolved version unless compilation fails.

**Pre-commit gate:** Run `cargo xtask check-all` from the worktree root before every commit except worktree/branch setup. Memory note: crate-scoped acceptance missed frb mirror drift once; workspace-level gate is binding.

**Pre-review gate:** Each **Phase** (A–E below) ends with a reviewer checkpoint. Do not start the next phase until the current phase has been reviewed via `superpowers:requesting-code-review`.

---

## File Structure

```
minos/
├── crates/
│   ├── minos-backend/                                 [RENAMED from minos-relay]
│   │   ├── Cargo.toml                                 [modified: dep on minos-ui-protocol]
│   │   ├── migrations/
│   │   │   ├── 0001_devices.sql                       [modified: role CHECK enum rename]
│   │   │   ├── 0002_pairings.sql                      [unchanged]
│   │   │   ├── 0003_pairing_tokens.sql                [unchanged]
│   │   │   ├── 0004_threads.sql                       [new]
│   │   │   └── 0005_raw_events.sql                    [new]
│   │   ├── src/
│   │   │   ├── main.rs                                [modified: env var prefix rename]
│   │   │   ├── config.rs                              [modified: + cf_access_* fields]
│   │   │   ├── http/
│   │   │   │   ├── mod.rs                             [unchanged]
│   │   │   │   ├── health.rs                          [unchanged]
│   │   │   │   └── ws_devices.rs                      [modified: route Ingest + new LocalRpcs]
│   │   │   ├── envelope/
│   │   │   │   ├── mod.rs                             [modified: dispatch ingest]
│   │   │   │   └── local_rpc.rs                       [modified: rename + new methods]
│   │   │   ├── ingest/                                [new directory]
│   │   │   │   ├── mod.rs                             [new]
│   │   │   │   └── translate.rs                       [new]
│   │   │   ├── pairing/
│   │   │   │   ├── mod.rs                             [modified: request_pairing_qr]
│   │   │   │   └── secret.rs                          [unchanged]
│   │   │   ├── session/                               [unchanged]
│   │   │   ├── store/
│   │   │   │   ├── mod.rs                             [modified: + threads + raw_events]
│   │   │   │   ├── devices.rs                         [unchanged]
│   │   │   │   ├── pairings.rs                        [unchanged]
│   │   │   │   ├── tokens.rs                          [unchanged]
│   │   │   │   ├── threads.rs                         [new]
│   │   │   │   └── raw_events.rs                      [new]
│   │   │   └── error.rs                               [modified: + new MinosError arms]
│   │   └── tests/
│   │       ├── e2e.rs                                 [modified: rename identifiers]
│   │       ├── ingest_roundtrip.rs                    [new]
│   │       └── list_threads.rs                        [new]
│   │
│   ├── minos-ui-protocol/                             [NEW crate]
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs                                 [new: exports + one-shot translate]
│   │   │   ├── message.rs                             [new: UiEventMessage + related enums]
│   │   │   ├── codex.rs                               [new: translator state + fn]
│   │   │   ├── claude.rs                              [new: stub]
│   │   │   ├── gemini.rs                              [new: stub]
│   │   │   └── error.rs                               [new: TranslationError]
│   │   └── tests/
│   │       ├── golden.rs                              [new: rstest harness]
│   │       └── golden/
│   │           └── codex/                             [new: N input/expected pairs]
│   │
│   ├── minos-protocol/
│   │   ├── Cargo.toml                                 [modified: dep on minos-ui-protocol]
│   │   ├── src/
│   │   │   ├── envelope.rs                            [modified: + Ingest; + UiEventMessage in EventKind; + LocalRpcMethods]
│   │   │   ├── messages.rs                            [modified: + PairingQrPayload, ThreadSummary, List*/Read* req/resp]
│   │   │   ├── rpc.rs                                 [modified: remove subscribe_events]
│   │   │   ├── events.rs                              [deleted: file becomes empty after AgentEvent removal]
│   │   │   └── lib.rs                                 [modified: mod events gone]
│   │   └── tests/
│   │       └── golden/envelope/                       [modified: + ingest.json, + ui_event_message.json]
│   │
│   ├── minos-domain/
│   │   ├── src/
│   │   │   ├── events.rs                              [deleted: AgentEvent was the only content]
│   │   │   ├── lib.rs                                 [modified: mod events gone]
│   │   │   └── error.rs                               [modified: + 5 new variants + user_message strings]
│   │   └── tests/
│   │       └── golden/agent_event_raw.json            [deleted]
│   │
│   ├── minos-agent-runtime/
│   │   ├── src/
│   │   │   ├── translate.rs                           [deleted — moves to minos-ui-protocol/codex.rs]
│   │   │   ├── ingest.rs                              [new]
│   │   │   ├── runtime.rs                             [modified: Ingestor wiring; broadcast carries raw tuples]
│   │   │   ├── codex_client.rs                        [modified: pass notifications to Ingestor]
│   │   │   └── lib.rs                                 [modified: remove pub mod translate]
│   │   └── src/test_support.rs                        [modified: seed fixtures point to ui-protocol path]
│   │
│   ├── minos-daemon/
│   │   └── src/
│   │       ├── rpc_server.rs                          [modified: remove subscribe_events impl]
│   │       ├── agent.rs                               [modified: state observer unchanged, event_stream removed]
│   │       └── handle.rs                              [modified: start_ingest_link wiring]
│   │
│   ├── minos-mobile/
│   │   └── src/
│   │       ├── client.rs                              [rewrite: envelope-aware; list/read/ui_events]
│   │       └── store.rs                               [modified: PairingStore trait extensions]
│   │
│   ├── minos-pairing/
│   │   └── src/
│   │       └── token.rs                               [modified: PairingQrPayload v2 export]
│   │
│   ├── minos-ffi-uniffi/                              [modified: no AgentEvent surface]
│   └── minos-ffi-frb/
│       └── src/
│           ├── api/minos.rs                           [modified: mirror UiEventMessage + list/read methods]
│           └── frb_generated.rs                       [REGENERATED]
│
├── apps/
│   ├── mobile/
│   │   ├── pubspec.yaml                               [modified: + flutter_secure_storage]
│   │   ├── lib/
│   │   │   ├── src/rust/                              [regenerated by frb]
│   │   │   ├── domain/minos_core_protocol.dart       [modified: + listThreads/readThread/uiEvents]
│   │   │   ├── infrastructure/
│   │   │   │   ├── minos_core.dart                    [modified]
│   │   │   │   ├── secure_pairing_store.dart          [new: Dart-side FlutterSecureStoragePairingStore]
│   │   │   │   └── app_paths.dart                     [unchanged]
│   │   │   ├── application/
│   │   │   │   ├── minos_providers.dart               [modified: backendUrlProvider]
│   │   │   │   ├── thread_list_provider.dart          [new]
│   │   │   │   └── thread_events_provider.dart        [new]
│   │   │   ├── presentation/
│   │   │   │   ├── app.dart                           [modified: _Router reads backendUrlProvider]
│   │   │   │   ├── pages/
│   │   │   │   │   ├── pairing_page.dart              [modified: parse QR v2]
│   │   │   │   │   ├── home_page.dart                 [deleted]
│   │   │   │   │   ├── thread_list_page.dart          [new]
│   │   │   │   │   └── thread_view_page.dart          [new]
│   │   │   │   └── widgets/
│   │   │   │       ├── thread_list_tile.dart          [new]
│   │   │   │       └── ui_event_tile.dart             [new]
│   │   │   └── main.dart                              [modified: + secure store initialization]
│   │   └── test/
│   │       ├── unit/
│   │       │   ├── pairing_controller_test.dart       [modified: QR v2]
│   │       │   └── thread_list_controller_test.dart   [new]
│   │       └── widget/
│   │           ├── thread_list_page_test.dart         [new]
│   │           └── thread_view_page_test.dart         [new]
│
├── xtask/src/main.rs                                  [modified: rename commands]
├── Cargo.toml                                         [modified: workspace members]
├── Cargo.lock                                         [regenerated]
├── README.md                                          [modified: relay → backend]
├── docs/
│   ├── adr/
│   │   ├── 0013-minos-ui-protocol-unified-event-shape.md    [new]
│   │   ├── 0014-backend-assembled-pairing-qr.md             [new]
│   │   └── 0015-rename-relay-to-backend.md                  [new]
│   └── ops/cloudflare-tunnel-setup.md                       [modified: + CF env vars for backend]
└── .github/workflows/ci.yml                                 [modified: + ui-protocol test step]
```

---

## Architecture in two sentences

`minos-agent-runtime` on the host reads raw codex/claude/gemini events and pushes each one verbatim to `minos-backend` as one `Envelope::Ingest` frame; `minos-backend` persists the raw event under `(thread_id, seq)`, runs the matching translator from `minos-ui-protocol` to produce `Vec<UiEventMessage>`, writes the raw to SQLite, and fans out each `UiEventMessage` to every paired mobile session as `EventKind::UiEventMessage`. Mobile consumes `UiEventMessage` via frb-generated Dart types, displays them in a deliberately plain `ThreadViewPage` (one `ListTile` per event), and asks for history via `LocalRpc::ReadThread` — the backend re-translates from `raw_events` on read.

---

# Phase A: Rename + Scaffold

**Ends when:** Renamed crate passes `cargo xtask check-all`; new `minos-ui-protocol` crate compiles with `UiEventMessage` + stubs; envelope protocol has `Ingest` + `EventKind::UiEventMessage` + renamed/new LocalRpc methods; golden fixtures round-trip; DB migrations land. **No behavior yet; structural + type-level only.**

## Task A1: Set up worktree and branch

**Files:** none (git operations only).

- [ ] **Step 1: Confirm current state**

Run: `git status && git branch --show-current`
Expected: clean working tree, on `main`, `1021703` is HEAD (the spec commit).

- [ ] **Step 2: Remove the stale `macos-relay-migration` worktree**

It was created before this spec; its intended scope is subsumed here.

```bash
git worktree remove ../minos-worktrees/macos-relay-migration
git branch -D feat/macos-relay-migration
```

Expected: the worktree disappears. If the branch deletion complains about "not merged", add `-D` (force) — no work was done on it.

- [ ] **Step 3: Create the plan's worktree**

```bash
git worktree add -b feat/mobile-and-ui-protocol ../minos-worktrees/mobile-and-ui-protocol
cd ../minos-worktrees/mobile-and-ui-protocol
```

From here onward, all `git` and `cargo` operations happen inside this worktree. (Tasks may still reference paths relative to the worktree root — e.g. `crates/minos-backend/` — because that is what `pwd` yields.)

- [ ] **Step 4: Verify the worktree compiles from clean**

Run: `cargo xtask check-all`
Expected: green. If not, stop and flag the regression.

- [ ] **Step 5: Commit (no-op marker)**

No code changed; there is nothing to commit. Skip to Task A2.

## Task A2: Rename `minos-relay` → `minos-backend`

**Files:** the entire `crates/minos-relay/` directory is renamed; all references across the workspace are updated in one atomic commit.

- [ ] **Step 1: Rename the directory**

```bash
git mv crates/minos-relay crates/minos-backend
```

- [ ] **Step 2: Update `Cargo.toml` (workspace root)**

Modify `Cargo.toml` at the repo root: in the `members = [...]` list, change `"crates/minos-relay"` to `"crates/minos-backend"`.

- [ ] **Step 3: Update the crate's `Cargo.toml`**

File: `crates/minos-backend/Cargo.toml`.

```toml
[package]
name = "minos-backend"                     # was "minos-relay"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "minos-backend"                     # was "minos-relay"
path = "src/main.rs"

# ... other sections unchanged
```

- [ ] **Step 4: Global rename inside the crate**

Rename library references from `minos_relay` → `minos_backend` and binary references from `minos-relay` → `minos-backend`. This includes `use minos_relay::...` lines in tests, doc comments, and `cargo run -p minos-relay` strings.

Use ripgrep to find every occurrence and edit manually (do not blindly sed — some matches may be in prose).

```bash
rg -n 'minos_relay|minos-relay' crates/minos-backend/
```

Update each hit. Typical targets:
- `crates/minos-backend/src/main.rs` — doc comments mentioning "minos-relay"
- `crates/minos-backend/src/lib.rs` — doc comments
- `crates/minos-backend/tests/e2e.rs` — import paths

- [ ] **Step 5: Update every other crate's references**

```bash
rg -n 'minos_relay|minos-relay' --glob '!crates/minos-backend/**' --glob '!target/**'
```

Typical external hits (update each):
- `xtask/src/main.rs` — command name constants, `cargo run -p minos-relay` → `cargo run -p minos-backend`
- `README.md` — prose mentions
- `docs/adr/` — refer to past decisions but don't mutate prose referencing the prior name in ADRs 0009/0011/0012 (those ADRs use the old name historically — leave them for §5.5 of spec)
- `docs/ops/cloudflare-tunnel-setup.md` — handled in Phase E

Scope: do **not** edit anything under `docs/superpowers/specs/minos-relay-backend-design.md` (it's the historical spec; leave its text as-is), nor under `docs/superpowers/plans/04-minos-relay-backend.md`.

- [ ] **Step 6: Update env var prefix**

Edit `crates/minos-backend/src/config.rs`. Every `env = "MINOS_RELAY_*"` → `env = "MINOS_BACKEND_*"`.

```rust
#[arg(long, env = "MINOS_BACKEND_LISTEN", default_value = "127.0.0.1:8787")]
pub listen: SocketAddr,

#[arg(long, env = "MINOS_BACKEND_DB", default_value = "./minos-backend.db")]
pub db: PathBuf,

// ... etc
```

- [ ] **Step 7: Update default DB filename**

Already covered in Step 6 (`./minos-backend.db`). Also update `crates/minos-backend/src/main.rs`'s default prod path:

```rust
// was: ~/Library/Application Support/minos-relay/db.sqlite
// is:  ~/Library/Application Support/minos-backend/db.sqlite
```

Search for the literal string in `src/main.rs` and replace.

- [ ] **Step 8: Update xlog prefix**

Edit `crates/minos-backend/src/main.rs`:

```rust
const XLOG_NAME_PREFIX: &str = "backend";  // was "relay"
```

- [ ] **Step 9: Update `xtask`**

Edit `xtask/src/main.rs`. Rename subcommand enum variants and strings:
- `RelayRun` → `BackendRun`
- `RelayDbReset` → `BackendDbReset`
- CLI name `relay-run` → `backend-run`
- `relay-db-reset` → `backend-db-reset`

All `cargo run -p minos-relay` → `cargo run -p minos-backend` inside xtask invocations.

- [ ] **Step 10: Regenerate lockfile**

```bash
cargo generate-lockfile
```

- [ ] **Step 11: Verify check-all**

```bash
cargo xtask check-all
```

Expected: green. Every test that referenced `minos-relay` now references `minos-backend` and still passes. If anything fails, fix the stragglers.

- [ ] **Step 12: Verify binary name**

```bash
cargo build -p minos-backend
ls target/debug/minos-backend
```

Expected: the file exists. `target/debug/minos-relay` should not exist.

- [ ] **Step 13: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
chore(backend): rename minos-relay → minos-backend

Prepares the crate for broadened responsibilities: raw ingest storage,
translation, and credential distribution (the reasons for the rename
are spelled out in docs/adr/0015 — to be authored in Phase E).

No behavior change; every reference updated in-place. Env vars:
MINOS_RELAY_* → MINOS_BACKEND_*. Default DB file:
./minos-relay.db → ./minos-backend.db. xlog prefix: relay → backend.
xtask commands: relay-run → backend-run, relay-db-reset → backend-db-reset.
EOF
)"
```

## Task A3: Scaffold `minos-ui-protocol` crate

**Files:**
- Create: `crates/minos-ui-protocol/Cargo.toml`
- Create: `crates/minos-ui-protocol/src/lib.rs`
- Create: `crates/minos-ui-protocol/src/error.rs`
- Create: `crates/minos-ui-protocol/src/claude.rs`
- Create: `crates/minos-ui-protocol/src/gemini.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create crate directory and manifest**

File: `crates/minos-ui-protocol/Cargo.toml`.

```toml
[package]
name = "minos-ui-protocol"
version = "0.1.0"
edition = "2021"

[dependencies]
minos-domain = { path = "../minos-domain" }
serde        = { workspace = true, features = ["derive"] }
serde_json   = { workspace = true }
thiserror    = { workspace = true }
tracing      = { workspace = true }
uuid         = { workspace = true, features = ["v4"] }

[dev-dependencies]
rstest            = { workspace = true }
pretty_assertions = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Stub `lib.rs`**

File: `crates/minos-ui-protocol/src/lib.rs`.

```rust
//! Minos unified UI event protocol.
//!
//! `UiEventMessage` is the single shape the mobile viewer and any future
//! admin surface consume to render agent activity. `translate_codex` /
//! `translate_claude` / `translate_gemini` map each CLI's native event
//! format onto this shape; the backend runs them on ingest and on
//! history read.
//!
//! See `docs/superpowers/specs/mobile-migration-and-ui-protocol-design.md`
//! §6.4 for the authoritative type definition.

#![forbid(unsafe_code)]

mod error;
mod message;
mod codex;
mod claude;
mod gemini;

pub use error::TranslationError;
pub use message::{MessageRole, ThreadEndReason, UiEventMessage};
pub use minos_domain::AgentName as AgentKind;

pub use codex::{translate as translate_codex, CodexTranslatorState};
pub use claude::translate as translate_claude;
pub use gemini::translate as translate_gemini;

/// One-shot dispatch convenience for the backend: given an agent kind
/// and one raw native event, return all resulting UI events. Used when
/// the caller does not carry per-thread translator state across calls
/// (e.g., a one-off history replay).
///
/// **Beware:** for codex, the translator is stateful across a thread
/// (tool-call argument buffering, open-message tracking). Use
/// [`CodexTranslatorState`] for live streams, not this function.
pub fn translate_stateless(
    agent: AgentKind,
    raw_payload: &serde_json::Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    match agent {
        AgentKind::Codex => {
            let mut s = CodexTranslatorState::new(String::new());
            translate_codex(&mut s, raw_payload)
        }
        AgentKind::Claude => translate_claude(raw_payload),
        AgentKind::Gemini => translate_gemini(raw_payload),
    }
}
```

- [ ] **Step 3: Scaffold `error.rs`**

File: `crates/minos-ui-protocol/src/error.rs`.

```rust
use thiserror::Error;

/// Errors emitted by translators. Lifted into `minos_domain::MinosError::
/// TranslationFailed` at the backend boundary (see `minos-backend`'s
/// ingest dispatch).
#[derive(Debug, Error)]
pub enum TranslationError {
    #[error("unsupported native event method: {method}")]
    UnsupportedMethod { method: String },

    #[error("malformed native event: {reason}")]
    Malformed { reason: String },

    #[error("translator not implemented for agent {agent:?}")]
    NotImplemented { agent: minos_domain::AgentName },
}
```

- [ ] **Step 4: Scaffold Claude / Gemini stubs**

File: `crates/minos-ui-protocol/src/claude.rs`.

```rust
use crate::error::TranslationError;
use crate::message::UiEventMessage;

pub fn translate(
    _raw: &serde_json::Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    Err(TranslationError::NotImplemented {
        agent: minos_domain::AgentName::Claude,
    })
}
```

File: `crates/minos-ui-protocol/src/gemini.rs`.

```rust
use crate::error::TranslationError;
use crate::message::UiEventMessage;

pub fn translate(
    _raw: &serde_json::Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    Err(TranslationError::NotImplemented {
        agent: minos_domain::AgentName::Gemini,
    })
}
```

- [ ] **Step 5: Placeholder `codex.rs` and `message.rs`**

Placeholders so `lib.rs` compiles. Real bodies land in A4 and B1.

File: `crates/minos-ui-protocol/src/message.rs`.

```rust
// Real definition lands in Task A4.
```

File: `crates/minos-ui-protocol/src/codex.rs`.

```rust
use crate::error::TranslationError;
use crate::message::UiEventMessage;

pub struct CodexTranslatorState {
    _thread_id: String,
}

impl CodexTranslatorState {
    pub fn new(thread_id: String) -> Self {
        Self { _thread_id: thread_id }
    }
}

pub fn translate(
    _state: &mut CodexTranslatorState,
    _raw: &serde_json::Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    Err(TranslationError::NotImplemented {
        agent: minos_domain::AgentName::Codex,
    })
}
```

(A4 overwrites `message.rs`. B1 overwrites `codex.rs`.)

- [ ] **Step 6: Add to workspace**

Edit repo root `Cargo.toml`. Under `[workspace] members = [...]`, append `"crates/minos-ui-protocol"`.

- [ ] **Step 7: Compile**

```bash
cargo check -p minos-ui-protocol
```

Expected: green (lib.rs fails to compile without `message::UiEventMessage` being visible; since `message.rs` is currently empty, expect an error). If so, revise Step 5's `message.rs` placeholder:

```rust
// Real definition lands in Task A4.

#[derive(Debug, Clone)]
pub enum UiEventMessage {
    _Placeholder,
}

#[derive(Debug, Clone)] pub enum MessageRole { _P }
#[derive(Debug, Clone)] pub enum ThreadEndReason { _P }
```

Rerun check.

- [ ] **Step 8: Commit**

```bash
git add crates/minos-ui-protocol Cargo.toml Cargo.lock
git commit -m "feat(ui-protocol): scaffold crate with placeholders"
```

## Task A4: Define `UiEventMessage` + serde round-trip tests

**Files:**
- Modify: `crates/minos-ui-protocol/src/message.rs`
- Test: `crates/minos-ui-protocol/src/message.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Write the failing tests first (inline at the bottom of `message.rs`)**

```rust
// ... UiEventMessage definition above ...

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;
    use pretty_assertions::assert_eq;

    #[test]
    fn text_delta_round_trip() {
        let ev = UiEventMessage::TextDelta {
            message_id: "msg_1".into(),
            text: "Hello".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(json, r#"{"kind":"text_delta","message_id":"msg_1","text":"Hello"}"#);
        let back: UiEventMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn thread_opened_serialises_snake_case_agent() {
        let ev = UiEventMessage::ThreadOpened {
            thread_id: "thr_1".into(),
            agent: AgentName::Codex,
            title: Some("hi".into()),
            opened_at_ms: 1_714_000_000_000,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"thread_opened""#));
        assert!(json.contains(r#""agent":"codex""#));
    }

    #[test]
    fn thread_closed_reason_crashed_has_nested_message() {
        let ev = UiEventMessage::ThreadClosed {
            thread_id: "thr_1".into(),
            reason: ThreadEndReason::Crashed { message: "oom".into() },
            closed_at_ms: 1_714_000_000_000,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""reason":{"kind":"crashed","message":"oom"}"#));
    }

    #[test]
    fn tool_call_placed_carries_full_args_json() {
        let ev = UiEventMessage::ToolCallPlaced {
            message_id: "msg_1".into(),
            tool_call_id: "tc_1".into(),
            name: "apply_patch".into(),
            args_json: r#"{"diff":"..."}"#.into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: UiEventMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn raw_is_forward_compat_escape_hatch() {
        let ev = UiEventMessage::Raw {
            kind: "item/plan/delta".into(),
            payload_json: r#"{"step":"compile"}"#.into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"raw""#));
    }

    #[test]
    fn message_role_assistant_snake_case() {
        let r = MessageRole::Assistant;
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#""assistant""#);
    }
}
```

- [ ] **Step 2: Run; confirm it fails (types don't exist)**

```bash
cargo test -p minos-ui-protocol
```

Expected: FAIL with "no variant named `TextDelta`", etc.

- [ ] **Step 3: Replace `message.rs` placeholder with the real enum**

File: `crates/minos-ui-protocol/src/message.rs`.

```rust
use minos_domain::AgentName;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEventMessage {
    // ── Thread lifecycle ─────────────
    ThreadOpened {
        thread_id: String,
        agent: AgentName,
        title: Option<String>,
        opened_at_ms: i64,
    },
    ThreadTitleUpdated {
        thread_id: String,
        title: String,
    },
    ThreadClosed {
        thread_id: String,
        reason: ThreadEndReason,
        closed_at_ms: i64,
    },

    // ── Message boundaries ───────────
    MessageStarted {
        message_id: String,
        role: MessageRole,
        started_at_ms: i64,
    },
    MessageCompleted {
        message_id: String,
        finished_at_ms: i64,
    },

    // ── Message content ──────────────
    TextDelta     { message_id: String, text: String },
    ReasoningDelta { message_id: String, text: String },

    // ── Tool calls ───────────────────
    ToolCallPlaced {
        message_id: String,
        tool_call_id: String,
        name: String,
        args_json: String,
    },
    ToolCallCompleted {
        tool_call_id: String,
        output: String,
        is_error: bool,
    },

    // ── Meta / escape hatch ──────────
    Error {
        code: String,
        message: String,
        message_id: Option<String>,
    },
    Raw {
        kind: String,
        payload_json: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThreadEndReason {
    UserStopped,
    AgentDone,
    Crashed { message: String },
    Timeout,
    HostDisconnected,
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minos-ui-protocol
```

Expected: all 6 tests pass.

- [ ] **Step 5: Run workspace check**

```bash
cargo xtask check-all
```

Expected: green.

- [ ] **Step 6: Commit**

```bash
git add crates/minos-ui-protocol/src/message.rs
git commit -m "feat(ui-protocol): UiEventMessage + serde round-trip tests"
```

## Task A5: Envelope + `LocalRpcMethod` + `messages.rs` extensions

**Files:**
- Modify: `crates/minos-protocol/Cargo.toml` (add `minos-ui-protocol` dep)
- Modify: `crates/minos-protocol/src/envelope.rs`
- Modify: `crates/minos-protocol/src/messages.rs`

- [ ] **Step 1: Add dependency**

Edit `crates/minos-protocol/Cargo.toml`, under `[dependencies]`:

```toml
minos-ui-protocol = { path = "../minos-ui-protocol" }
```

- [ ] **Step 2: Add test first (envelope with Ingest variant round-trips)**

Add an inline test at the bottom of `crates/minos-protocol/src/envelope.rs`:

```rust
#[test]
fn envelope_ingest_round_trip() {
    let e = Envelope::Ingest {
        version: 1,
        agent: minos_domain::AgentName::Codex,
        thread_id: "thr_1".into(),
        seq: 42,
        payload: serde_json::json!({"method":"item/agentMessage/delta","params":{"delta":"Hi"}}),
        ts_ms: 1_714_000_000_000,
    };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains(r#""kind":"ingest""#));
    assert!(s.contains(r#""agent":"codex""#));
    let back: Envelope = serde_json::from_str(&s).unwrap();
    assert_eq!(e, back);
}

#[test]
fn envelope_event_ui_event_message_round_trip() {
    let e = Envelope::Event {
        version: 1,
        event: EventKind::UiEventMessage {
            thread_id: "thr_1".into(),
            seq: 42,
            ui: minos_ui_protocol::UiEventMessage::TextDelta {
                message_id: "msg_1".into(),
                text: "Hi".into(),
            },
            ts_ms: 1_714_000_000_000,
        },
    };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains(r#""type":"ui_event_message""#));
    assert!(s.contains(r#""kind":"text_delta""#));
    let back: Envelope = serde_json::from_str(&s).unwrap();
    assert_eq!(e, back);
}
```

(You'll need `PartialEq` on `Envelope`; it already has `Eq`-compatible fields. If compile fails, add `PartialEq` derive.)

- [ ] **Step 3: Run tests to confirm failure**

```bash
cargo test -p minos-protocol envelope_ingest_round_trip envelope_event_ui_event_message_round_trip
```

Expected: FAIL with "no variant Ingest" and "no variant UiEventMessage".

- [ ] **Step 4: Extend `Envelope`**

Append to the `Envelope` enum in `crates/minos-protocol/src/envelope.rs`:

```rust
// after existing variants:
    /// Agent-host → Backend. Raw native event from a CLI for persistence
    /// and fan-out. No response expected. (seq, thread_id) must be unique
    /// server-side; the host treats conflicts as a no-op.
    Ingest {
        #[serde(rename = "v")]
        version: u8,
        agent: minos_domain::AgentName,
        thread_id: String,
        seq: u64,
        payload: serde_json::Value,
        ts_ms: i64,
    },
```

- [ ] **Step 5: Extend `EventKind`**

Append to `EventKind`:

```rust
// ... existing variants (Paired, PeerOnline, PeerOffline, Unpaired, ServerShutdown)
    /// Backend → Mobile. One translated UI event from backend's live
    /// fan-out. `seq` matches the underlying `raw_events` row so mobile
    /// can dedupe against its per-thread watermark.
    UiEventMessage {
        thread_id: String,
        seq: u64,
        ui: minos_ui_protocol::UiEventMessage,
        ts_ms: i64,
    },
```

- [ ] **Step 6: Rename and extend `LocalRpcMethod`**

Edit the `LocalRpcMethod` enum:

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum LocalRpcMethod {
    Ping,
    RequestPairingQr,                // renamed from RequestPairingToken
    Pair,
    ForgetPeer,
    ListThreads,                     // new
    ReadThread,                      // new
    GetThreadLastSeq,                // new (host-only helper)
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p minos-protocol
```

Expected: the two new tests pass; prior tests may have broken because `EventKind` or `LocalRpcMethod` identifiers changed — chase each failure and fix. Common hit: `RequestPairingToken` used in the fixture harness at `tests/golden/envelope/` and in `minos-backend/src/envelope/local_rpc.rs`. Rename those sites too.

Specifically:
- `crates/minos-backend/src/envelope/local_rpc.rs` — match arm `LocalRpcMethod::RequestPairingToken` → `::RequestPairingQr` (we'll adjust the body in C4; for now, leave the existing body — it still returns the old `{token, expires_at}` shape; we pass the rename through so the enum compiles, then change behavior later)
- Any `.json` golden fixtures under `crates/minos-protocol/tests/golden/envelope/` — search for `"request_pairing_token"` and update
- `crates/minos-backend/tests/e2e.rs` — similar string and fixture updates

- [ ] **Step 8: Add `PairingQrPayload` + `ThreadSummary` + List/Read request/response structs**

Edit `crates/minos-protocol/src/messages.rs`, appending:

```rust
use minos_domain::AgentName;
use minos_ui_protocol::{UiEventMessage, ThreadEndReason};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PairingQrPayload {
    pub v: u8,
    pub backend_url: String,
    pub host_display_name: String,
    pub pairing_token: String,
    pub expires_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cf_access_client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cf_access_client_secret: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RequestPairingQrParams {
    pub host_display_name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RequestPairingQrResponse {
    pub qr_payload: PairingQrPayload,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    pub thread_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<ThreadEndReason>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListThreadsParams {
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_ts_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentName>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListThreadsResponse {
    pub threads: Vec<ThreadSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_before_ts_ms: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReadThreadParams {
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_seq: Option<u64>,
    pub limit: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReadThreadResponse {
    pub ui_events: Vec<UiEventMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_end_reason: Option<ThreadEndReason>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GetThreadLastSeqParams {
    pub thread_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GetThreadLastSeqResponse {
    pub last_seq: u64,
}
```

- [ ] **Step 9: Add inline round-trip tests for each new type**

At the bottom of `messages.rs`:

```rust
#[cfg(test)]
mod new_type_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pairing_qr_payload_round_trip_with_cf() {
        let p = PairingQrPayload {
            v: 2,
            backend_url: "wss://minos.fan-nn.top/devices".into(),
            host_display_name: "Mac".into(),
            pairing_token: "tok".into(),
            expires_at_ms: 1,
            cf_access_client_id: Some("id".into()),
            cf_access_client_secret: Some("sec".into()),
        };
        let back: PairingQrPayload = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn pairing_qr_payload_without_cf_omits_fields() {
        let p = PairingQrPayload {
            v: 2, backend_url: "x".into(), host_display_name: "x".into(),
            pairing_token: "t".into(), expires_at_ms: 0,
            cf_access_client_id: None, cf_access_client_secret: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(!s.contains("cf_access_client_id"));
    }

    // similar tests for ThreadSummary, ListThreadsResponse, ReadThreadResponse omitted for brevity —
    // add at least one round-trip assertion per type.
}
```

- [ ] **Step 10: Run check + tests**

```bash
cargo xtask check-all
```

Expected: green. Fixture churn in `minos-backend` test fixtures may flag a missing `request_pairing_qr` golden entry; add a minimal golden file:

File: `crates/minos-backend/tests/golden/envelope/local_rpc_request_pairing_qr.json` (create if the directory exists):

```json
{"kind":"local_rpc","v":1,"id":1,"method":"request_pairing_qr","params":{"host_display_name":"Mac"}}
```

Leave other fixtures alone at this stage.

- [ ] **Step 11: Commit**

```bash
git add crates/minos-protocol crates/minos-backend Cargo.lock
git commit -m "feat(protocol): envelope Ingest + EventKind::UiEventMessage + new LocalRpcs"
```

## Task A6: Golden envelope fixtures

**Files:**
- Create/modify: `crates/minos-protocol/tests/golden/envelope/ingest.json`
- Create/modify: `crates/minos-protocol/tests/golden/envelope/event_ui_event_message.json`
- Modify: `crates/minos-protocol/tests/schema_golden.rs` (or equivalent harness name)

- [ ] **Step 1: Check the existing harness**

Read `crates/minos-protocol/tests/schema_golden.rs`. Identify the pattern (likely: load each `*.json`, deserialise as `Envelope`, serialise back, compare normalised).

- [ ] **Step 2: Create `ingest.json`**

```json
{
  "kind": "ingest",
  "v": 1,
  "agent": "codex",
  "thread_id": "thr_abc",
  "seq": 42,
  "payload": {
    "method": "item/agentMessage/delta",
    "params": { "delta": "Hi" }
  },
  "ts_ms": 1714000000000
}
```

- [ ] **Step 3: Create `event_ui_event_message.json`**

```json
{
  "kind": "event",
  "v": 1,
  "type": "ui_event_message",
  "thread_id": "thr_abc",
  "seq": 42,
  "ui": {
    "kind": "text_delta",
    "message_id": "msg_def",
    "text": "Hi"
  },
  "ts_ms": 1714000000000
}
```

- [ ] **Step 4: Run the golden harness**

```bash
cargo test -p minos-protocol --test schema_golden
```

Expected: PASS. If the harness auto-discovers files, no code change needed. If it has a static list, append the new filenames to that list.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-protocol/tests/golden/envelope
git commit -m "test(protocol): golden fixtures for ingest + ui_event_message envelopes"
```

## Task A7: Backend DB migrations — new tables + role rename

**Files:**
- Modify: `crates/minos-backend/migrations/0001_devices.sql`
- Create: `crates/minos-backend/migrations/0004_threads.sql`
- Create: `crates/minos-backend/migrations/0005_raw_events.sql`

- [ ] **Step 1: Edit 0001 in place (role rename)**

File: `crates/minos-backend/migrations/0001_devices.sql`. Change the CHECK clause:

```sql
CREATE TABLE devices (
    device_id      TEXT PRIMARY KEY,
    display_name   TEXT NOT NULL,
    role           TEXT NOT NULL CHECK (role IN ('agent-host','ios-client','browser-admin')),
    secret_hash    TEXT,
    created_at     INTEGER NOT NULL,
    last_seen_at   INTEGER NOT NULL
) STRICT;
```

(Only `'mac-host'` → `'agent-host'` changes.)

- [ ] **Step 2: Create 0004_threads.sql**

```sql
CREATE TABLE threads (
    thread_id         TEXT PRIMARY KEY,
    agent             TEXT NOT NULL CHECK (agent IN ('codex','claude','gemini')),
    owner_device_id   TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    title             TEXT,
    first_ts_ms       INTEGER NOT NULL,
    last_ts_ms        INTEGER NOT NULL,
    ended_at_ms       INTEGER,
    end_reason        TEXT,
    message_count     INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX idx_threads_last_ts  ON threads(last_ts_ms DESC);
CREATE INDEX idx_threads_owner    ON threads(owner_device_id, last_ts_ms DESC);
```

- [ ] **Step 3: Create 0005_raw_events.sql**

```sql
CREATE TABLE raw_events (
    thread_id    TEXT NOT NULL REFERENCES threads(thread_id) ON DELETE CASCADE,
    seq          INTEGER NOT NULL,
    agent        TEXT NOT NULL CHECK (agent IN ('codex','claude','gemini')),
    payload_json TEXT NOT NULL,
    ts_ms        INTEGER NOT NULL,
    PRIMARY KEY (thread_id, seq)
) STRICT;

CREATE INDEX idx_raw_events_thread_seq ON raw_events(thread_id, seq);
```

- [ ] **Step 4: Run migrations locally**

From the worktree root:

```bash
rm -f ./minos-backend.db
cargo run -p minos-backend -- --exit-after-migrate --listen 127.0.0.1:9999 --db ./minos-backend.db
```

Expected: the binary logs `migrations applied` and exits. Verify with:

```bash
sqlite3 ./minos-backend.db ".schema threads"
```

Should dump the threads DDL.

- [ ] **Step 5: Regenerate sqlx offline metadata**

If the project uses sqlx's offline feature, run:

```bash
DATABASE_URL=sqlite://./minos-backend.db cargo sqlx prepare --workspace
```

Expected: a fresh `.sqlx/` directory with query descriptors. Commit those files too.

(If `cargo sqlx` isn't installed yet in this session: `cargo install sqlx-cli --no-default-features --features sqlite,rustls` first.)

- [ ] **Step 6: Verify check-all**

```bash
cargo xtask check-all
```

- [ ] **Step 7: Commit**

```bash
git add crates/minos-backend/migrations .sqlx
git commit -m "feat(backend): threads + raw_events tables; role enum renamed to agent-host"
```

## Task A8: New `MinosError` variants

**Files:**
- Modify: `crates/minos-domain/src/error.rs`

- [ ] **Step 1: Add test cases for the five new variants**

Append to the existing test module in `crates/minos-domain/src/error.rs`:

```rust
#[cfg(test)]
mod new_variant_tests {
    use super::*;

    #[test]
    fn every_new_kind_has_messages_in_both_langs() {
        for kind in [
            ErrorKind::CfAccessMisconfigured,
            ErrorKind::IngestSeqConflict,
            ErrorKind::ThreadNotFound,
            ErrorKind::TranslationNotImplemented,
            ErrorKind::TranslationFailed,
        ] {
            assert!(!kind.user_message(Lang::Zh).is_empty(), "zh missing for {kind:?}");
            assert!(!kind.user_message(Lang::En).is_empty(), "en missing for {kind:?}");
        }
    }

    #[test]
    fn ingest_seq_conflict_display() {
        let e = MinosError::IngestSeqConflict { thread_id: "t".into(), seq: 42 };
        assert_eq!(format!("{e}"), "ingest seq conflict for thread t: seq 42 already present");
    }
}
```

- [ ] **Step 2: Run; confirm failure**

```bash
cargo test -p minos-domain new_variant_tests
```

Expected: FAIL (variants not present).

- [ ] **Step 3: Add variants**

Edit `crates/minos-domain/src/error.rs`:

```rust
// Inside the MinosError enum:
    #[error("cf access misconfigured at backend: {reason}")]
    CfAccessMisconfigured { reason: String },

    #[error("ingest seq conflict for thread {thread_id}: seq {seq} already present")]
    IngestSeqConflict { thread_id: String, seq: u64 },

    #[error("thread not found: {thread_id}")]
    ThreadNotFound { thread_id: String },

    #[error("translation not implemented for agent {agent:?}")]
    TranslationNotImplemented { agent: AgentName },

    #[error("translation failed for agent {agent:?}: {message}")]
    TranslationFailed { agent: AgentName, message: String },
```

Inside `ErrorKind` enum, add matching variants (plain unit, no fields).

Inside the `kind()` match expression, map each `MinosError` variant to its `ErrorKind` partner.

Inside `user_message`, add pairs:

```rust
ErrorKind::CfAccessMisconfigured => match lang {
    Lang::Zh => "后端未正确配置 Cloudflare Access 凭据",
    Lang::En => "Backend Cloudflare Access credentials are not configured",
},
ErrorKind::IngestSeqConflict => match lang {
    Lang::Zh => "事件序号冲突",
    Lang::En => "Event sequence conflict",
},
ErrorKind::ThreadNotFound => match lang {
    Lang::Zh => "找不到该线程",
    Lang::En => "Thread not found",
},
ErrorKind::TranslationNotImplemented => match lang {
    Lang::Zh => "该 CLI 尚未接入协议翻译",
    Lang::En => "Translator not implemented for this CLI",
},
ErrorKind::TranslationFailed => match lang {
    Lang::Zh => "事件翻译失败",
    Lang::En => "Event translation failed",
},
```

- [ ] **Step 4: Run**

```bash
cargo xtask check-all
```

Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-domain/src/error.rs
git commit -m "feat(domain): add 5 MinosError variants for ingest / translation / cf_access"
```

## ✅ Phase A Reviewer Checkpoint

Before starting Phase B, dispatch code review. Invoke skill `superpowers:requesting-code-review` on the range `main..feat/mobile-and-ui-protocol`. Focus areas:

1. Is the rename complete and consistent (no stray `minos-relay` or `minos_relay` references)?
2. Is `UiEventMessage` shape faithful to spec §6.4?
3. Are new DB tables + migrations conformant (STRICT, FK cascades, indexes present)?
4. Any missing golden fixtures?

Only start Task B1 after review is resolved.

---

# Phase B: Translation + Ingest Pipeline

**Ends when:** `translate_codex` is fully implemented with ≥12 golden fixtures passing; `AgentEvent` is deleted and the workspace compiles; `minos-agent-runtime::Ingestor` connects to the backend and pushes raw events; the backend ingest handler persists + translates + fans out; an integration test `crates/minos-backend/tests/ingest_roundtrip.rs` exercises the full ingest → `UiEventMessage` stream path.

## Task B1: Codex translator state machine

**Files:**
- Modify: `crates/minos-ui-protocol/src/codex.rs` (overwrite placeholder)

Read first: the existing `crates/minos-agent-runtime/src/translate.rs` — its mapping to `AgentEvent` is the prior art. Most logic carries over with the output type changed to `Vec<UiEventMessage>`.

- [ ] **Step 1: Write the highest-signal test first (full happy-path stream)**

Create a fixture-driven test (add to `crates/minos-ui-protocol/src/codex.rs` as an inline `#[cfg(test)] mod` or in a dedicated `tests/codex_state.rs`). Below is the inline style:

```rust
#[cfg(test)]
mod state_tests {
    use super::*;
    use crate::message::*;
    use minos_domain::AgentName;
    use pretty_assertions::assert_eq;

    fn val(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn thread_started_emits_thread_opened() {
        let mut state = CodexTranslatorState::new("thr_x".into());
        let raw = val(r#"{
            "method":"thread/started",
            "params":{"threadId":"thr_x","createdAtMs":1714000000000}
        }"#);
        let out = translate(&mut state, &raw).unwrap();
        assert_eq!(out.len(), 1);
        match &out[0] {
            UiEventMessage::ThreadOpened { thread_id, agent, opened_at_ms, .. } => {
                assert_eq!(thread_id, "thr_x");
                assert_eq!(*agent, AgentName::Codex);
                assert_eq!(*opened_at_ms, 1714000000000);
            }
            _ => panic!("unexpected {:?}", out[0]),
        }
    }

    #[test]
    fn unknown_method_falls_through_to_raw() {
        let mut state = CodexTranslatorState::new("thr_x".into());
        let raw = val(r#"{"method":"item/plan/delta","params":{"step":"compile"}}"#);
        let out = translate(&mut state, &raw).unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], UiEventMessage::Raw { kind, .. } if kind == "item/plan/delta"));
    }

    #[test]
    fn agent_message_sequence() {
        // item/started(role=agent) → MessageStarted
        // item/agentMessage/delta → TextDelta
        // item/agentMessage/delta → TextDelta
        // turn/completed → MessageCompleted (for open assistant message)
        let mut s = CodexTranslatorState::new("thr".into());

        let o1 = translate(&mut s, &val(r#"{"method":"item/started","params":{"itemId":"i1","role":"agent","startedAtMs":1}}"#)).unwrap();
        assert!(matches!(o1.as_slice(), [UiEventMessage::MessageStarted { role: MessageRole::Assistant, .. }]));

        let o2 = translate(&mut s, &val(r#"{"method":"item/agentMessage/delta","params":{"itemId":"i1","delta":"Hel"}}"#)).unwrap();
        assert!(matches!(o2.as_slice(), [UiEventMessage::TextDelta { text, .. }] if text == "Hel"));

        let o3 = translate(&mut s, &val(r#"{"method":"item/agentMessage/delta","params":{"itemId":"i1","delta":"lo"}}"#)).unwrap();
        assert!(matches!(o3.as_slice(), [UiEventMessage::TextDelta { text, .. }] if text == "lo"));

        let o4 = translate(&mut s, &val(r#"{"method":"turn/completed","params":{"finishedAtMs":2}}"#)).unwrap();
        assert!(matches!(o4.as_slice(), [UiEventMessage::MessageCompleted { finished_at_ms: 2, .. }]));
    }

    #[test]
    fn tool_call_buffers_args_then_emits_placed() {
        let mut s = CodexTranslatorState::new("thr".into());

        // Bracket with a MessageStarted so the tool is associated.
        let _ = translate(&mut s, &val(r#"{"method":"item/started","params":{"itemId":"i1","role":"agent","startedAtMs":1}}"#)).unwrap();

        // tool call starts
        let o1 = translate(&mut s, &val(r#"{"method":"item/toolCall/started","params":{"itemId":"i1","toolCallId":"tc_1","name":"run_command"}}"#)).unwrap();
        assert!(o1.is_empty(), "emitted too early: {:?}", o1);

        let o2 = translate(&mut s, &val(r#"{"method":"item/toolCall/arguments","params":{"toolCallId":"tc_1","argumentsDelta":"{\"cmd\":\"ls"}}"#)).unwrap();
        assert!(o2.is_empty());

        let o3 = translate(&mut s, &val(r#"{"method":"item/toolCall/arguments","params":{"toolCallId":"tc_1","argumentsDelta":"\"}"}}"#)).unwrap();
        assert!(o3.is_empty());

        let o4 = translate(&mut s, &val(r#"{"method":"item/toolCall/argumentsCompleted","params":{"toolCallId":"tc_1"}}"#)).unwrap();
        assert_eq!(o4.len(), 1);
        match &o4[0] {
            UiEventMessage::ToolCallPlaced { tool_call_id, name, args_json, .. } => {
                assert_eq!(tool_call_id, "tc_1");
                assert_eq!(name, "run_command");
                assert_eq!(args_json, r#"{"cmd":"ls"}"#);
            }
            _ => panic!(),
        }

        let o5 = translate(&mut s, &val(r#"{"method":"item/toolCall/completed","params":{"toolCallId":"tc_1","output":"file1\nfile2","isError":false}}"#)).unwrap();
        assert!(matches!(o5.as_slice(), [UiEventMessage::ToolCallCompleted { output, is_error: false, .. }] if output == "file1\nfile2"));
    }
}
```

- [ ] **Step 2: Run; confirm failure**

```bash
cargo test -p minos-ui-protocol state_tests
```

Expected: FAIL — codex translator is still the stub.

- [ ] **Step 3: Implement `CodexTranslatorState` + `translate`**

Replace the body of `crates/minos-ui-protocol/src/codex.rs`:

```rust
use crate::error::TranslationError;
use crate::message::{MessageRole, ThreadEndReason, UiEventMessage};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// Per-thread state the translator accumulates while streaming raw codex
/// notifications. Not thread-safe; one instance per `thread_id`.
pub struct CodexTranslatorState {
    thread_id: String,
    /// Currently-open assistant message (only one at a time for codex).
    open_assistant_message_id: Option<String>,
    /// Tool call → buffered raw arguments JSON string + the stable
    /// UUID the translator assigned when started.
    tool_calls: HashMap<String, OpenToolCall>,
}

struct OpenToolCall {
    message_id: String,
    tool_call_id_stable: String,   // our UUID; we use the CLI's id as key here
    name: String,
    args_buf: String,
}

impl CodexTranslatorState {
    pub fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            open_assistant_message_id: None,
            tool_calls: HashMap::new(),
        }
    }
}

/// Translate one raw codex WS notification (or response) into zero or more
/// UI events. State is threaded through `state`.
pub fn translate(
    state: &mut CodexTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let method = raw.get("method").and_then(|v| v.as_str())
        .ok_or_else(|| TranslationError::Malformed { reason: "missing method".into() })?;
    let params = raw.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "thread/started" => {
            let thread_id = params.get("threadId")
                .and_then(Value::as_str).unwrap_or(&state.thread_id).to_string();
            let opened_at_ms = params.get("createdAtMs")
                .and_then(Value::as_i64).unwrap_or(0);
            Ok(vec![UiEventMessage::ThreadOpened {
                thread_id,
                agent: AgentName::Codex,
                title: None,
                opened_at_ms,
            }])
        }
        "thread/archived" => {
            let closed_at_ms = params.get("archivedAtMs")
                .and_then(Value::as_i64).unwrap_or(0);
            Ok(vec![UiEventMessage::ThreadClosed {
                thread_id: state.thread_id.clone(),
                reason: ThreadEndReason::AgentDone,
                closed_at_ms,
            }])
        }
        "item/started" => {
            let role_raw = params.get("role").and_then(Value::as_str).unwrap_or("agent");
            let started_at_ms = params.get("startedAtMs").and_then(Value::as_i64).unwrap_or(0);
            let role = match role_raw {
                "user" => MessageRole::User,
                "agent" | "assistant" => MessageRole::Assistant,
                _ => MessageRole::System,
            };
            let message_id = Uuid::new_v4().to_string();
            if matches!(role, MessageRole::Assistant) {
                state.open_assistant_message_id = Some(message_id.clone());
            }
            Ok(vec![UiEventMessage::MessageStarted { message_id, role, started_at_ms }])
        }
        "item/agentMessage/delta" => {
            let text = params.get("delta").and_then(Value::as_str).unwrap_or("").to_string();
            let Some(msg_id) = state.open_assistant_message_id.clone() else {
                return Ok(vec![]); // delta without an open message — drop silently
            };
            Ok(vec![UiEventMessage::TextDelta { message_id: msg_id, text }])
        }
        "item/reasoning/delta" => {
            let text = params.get("delta").and_then(Value::as_str).unwrap_or("").to_string();
            let Some(msg_id) = state.open_assistant_message_id.clone() else {
                return Ok(vec![]);
            };
            Ok(vec![UiEventMessage::ReasoningDelta { message_id: msg_id, text }])
        }
        "item/toolCall/started" => {
            let cli_id = params.get("toolCallId").and_then(Value::as_str)
                .ok_or_else(|| TranslationError::Malformed { reason: "toolCallId missing".into() })?.to_string();
            let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let Some(msg_id) = state.open_assistant_message_id.clone() else {
                return Ok(vec![]);
            };
            let stable_id = Uuid::new_v4().to_string();
            state.tool_calls.insert(cli_id, OpenToolCall {
                message_id: msg_id,
                tool_call_id_stable: stable_id,
                name,
                args_buf: String::new(),
            });
            Ok(vec![])
        }
        "item/toolCall/arguments" => {
            let cli_id = params.get("toolCallId").and_then(Value::as_str)
                .ok_or_else(|| TranslationError::Malformed { reason: "toolCallId missing".into() })?;
            if let Some(tc) = state.tool_calls.get_mut(cli_id) {
                if let Some(delta) = params.get("argumentsDelta").and_then(Value::as_str) {
                    tc.args_buf.push_str(delta);
                }
            }
            Ok(vec![])
        }
        "item/toolCall/argumentsCompleted" => {
            let cli_id = params.get("toolCallId").and_then(Value::as_str)
                .ok_or_else(|| TranslationError::Malformed { reason: "toolCallId missing".into() })?;
            if let Some(tc) = state.tool_calls.get(cli_id) {
                Ok(vec![UiEventMessage::ToolCallPlaced {
                    message_id: tc.message_id.clone(),
                    tool_call_id: tc.tool_call_id_stable.clone(),
                    name: tc.name.clone(),
                    args_json: tc.args_buf.clone(),
                }])
            } else {
                Ok(vec![])
            }
        }
        "item/toolCall/completed" => {
            let cli_id = params.get("toolCallId").and_then(Value::as_str)
                .ok_or_else(|| TranslationError::Malformed { reason: "toolCallId missing".into() })?;
            let output = params.get("output").and_then(Value::as_str).unwrap_or("").to_string();
            let is_error = params.get("isError").and_then(Value::as_bool).unwrap_or(false);
            if let Some(tc) = state.tool_calls.remove(cli_id) {
                Ok(vec![UiEventMessage::ToolCallCompleted {
                    tool_call_id: tc.tool_call_id_stable,
                    output,
                    is_error,
                }])
            } else {
                Ok(vec![])
            }
        }
        "turn/completed" => {
            let finished_at_ms = params.get("finishedAtMs").and_then(Value::as_i64).unwrap_or(0);
            let Some(msg_id) = state.open_assistant_message_id.take() else {
                return Ok(vec![]);
            };
            Ok(vec![UiEventMessage::MessageCompleted { message_id: msg_id, finished_at_ms }])
        }
        other => {
            // Unknown method — forward as Raw so the UI can surface it.
            Ok(vec![UiEventMessage::Raw {
                kind: other.to_string(),
                payload_json: serde_json::to_string(&params).unwrap_or_default(),
            }])
        }
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minos-ui-protocol
```

Expected: all state_tests pass. If one fails, check the exact `method` string matched in the arm.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-ui-protocol/src/codex.rs
git commit -m "feat(ui-protocol): codex translator state machine + core event coverage"
```

## Task B2: Codex golden fixtures

**Files:**
- Create: `crates/minos-ui-protocol/tests/golden.rs`
- Create: `crates/minos-ui-protocol/tests/golden/codex/*.input.json` + `.expected.json`

- [ ] **Step 1: Write the harness**

File: `crates/minos-ui-protocol/tests/golden.rs`.

```rust
use std::fs;
use std::path::PathBuf;

use minos_ui_protocol::{translate_codex, CodexTranslatorState, UiEventMessage};
use rstest::rstest;

/// One fixture per file pair: `<name>.input.json` is a sequence (JSON array)
/// of raw codex notifications; `<name>.expected.json` is a JSON array of
/// the full concatenated `Vec<UiEventMessage>` produced by feeding all
/// inputs through a fresh `CodexTranslatorState`.
#[rstest]
fn codex_golden(
    #[files("tests/golden/codex/*.input.json")]
    input_path: PathBuf,
) {
    let expected_path = PathBuf::from(
        input_path.to_string_lossy().replace(".input.json", ".expected.json")
    );

    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&input_path).unwrap()).unwrap();
    let expected: Vec<UiEventMessage> =
        serde_json::from_str(&fs::read_to_string(&expected_path).unwrap()).unwrap();

    let mut state = CodexTranslatorState::new("thr_fixture".into());
    let mut got = Vec::new();
    for ev in &inputs {
        got.extend(translate_codex(&mut state, ev).unwrap());
    }
    pretty_assertions::assert_eq!(got, expected, "fixture {}", input_path.display());
}
```

- [ ] **Step 2: Create the minimum set of fixtures (12)**

File pair `tests/golden/codex/thread_started.input.json`:

```json
[
  {"method":"thread/started","params":{"threadId":"thr_fixture","createdAtMs":1714000000000}}
]
```

File pair `tests/golden/codex/thread_started.expected.json`:

```json
[
  {
    "kind":"thread_opened",
    "thread_id":"thr_fixture",
    "agent":"codex",
    "title":null,
    "opened_at_ms":1714000000000
  }
]
```

Repeat for the remaining methods (minimum set; add more as discovered):

- `user_message.input.json` / `.expected.json` — user role item started
- `agent_message_delta.input.json` / `.expected.json` — item started(agent) + two deltas + turn completed
- `reasoning_delta.input.json` / `.expected.json` — analogous but `item/reasoning/delta`
- `tool_call_full.input.json` / `.expected.json` — started + args + completed + output
- `tool_call_error.input.json` / `.expected.json` — isError true
- `unknown_method.input.json` / `.expected.json` — e.g. `item/plan/delta` → Raw
- `empty_delta.input.json` / `.expected.json` — delta with empty text
- `orphan_delta.input.json` / `.expected.json` — delta with no open message → empty result
- `two_turns.input.json` / `.expected.json` — two full assistant turns back-to-back
- `thread_archived.input.json` / `.expected.json` — ThreadClosed
- `mixed_reasoning_text.input.json` / `.expected.json` — reasoning delta followed by text delta

For each expected output file, you write the expected `Vec<UiEventMessage>` JSON by hand. Run the harness to verify correctness; when `assert_eq` diffs, copy the actual output if you trust it, or fix the translator if the actual output is wrong.

**Hand-construction tip:** generate the expected file by temporarily printing the actual result: `dbg!(&got);` and pasting. Then remove the debug line.

- [ ] **Step 3: Run**

```bash
cargo test -p minos-ui-protocol --test golden
```

Expected: all 12 fixtures pass.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-ui-protocol/tests
git commit -m "test(ui-protocol): 12 codex translator golden fixtures"
```

## Task B3: Delete `AgentEvent`

**Files:**
- Delete: `crates/minos-domain/src/events.rs`, `crates/minos-domain/tests/golden/agent_event_raw.json`
- Modify: `crates/minos-domain/src/lib.rs`, `crates/minos-domain/tests/golden.rs`
- Delete: `crates/minos-protocol/src/events.rs`
- Modify: `crates/minos-protocol/src/lib.rs`
- Modify: `crates/minos-protocol/src/rpc.rs` (remove `subscribe_events`)
- Modify: `crates/minos-daemon/src/rpc_server.rs`, `crates/minos-daemon/src/agent.rs`, `crates/minos-daemon/src/handle.rs`
- Modify: `crates/minos-agent-runtime/src/translate.rs` (delete), `crates/minos-agent-runtime/src/runtime.rs`, `crates/minos-agent-runtime/src/lib.rs`

- [ ] **Step 1: Delete the source files**

```bash
git rm crates/minos-domain/src/events.rs
git rm crates/minos-domain/tests/golden/agent_event_raw.json
git rm crates/minos-protocol/src/events.rs
git rm crates/minos-agent-runtime/src/translate.rs
```

- [ ] **Step 2: Remove `pub mod events;`**

Edit:
- `crates/minos-domain/src/lib.rs` — remove `pub mod events;` and any `pub use events::*`
- `crates/minos-protocol/src/lib.rs` — remove `pub mod events;` and re-export
- `crates/minos-agent-runtime/src/lib.rs` — remove `pub(crate) mod translate;` or equivalent

- [ ] **Step 3: Remove `subscribe_events` from the RPC trait**

Edit `crates/minos-protocol/src/rpc.rs`. Delete:

```rust
#[subscription(name = "subscribe_events", item = AgentEvent, unsubscribe = "unsubscribe_events")]
async fn subscribe_events(&self) -> SubscriptionResult;
```

- [ ] **Step 4: Delete the impl in daemon**

Edit `crates/minos-daemon/src/rpc_server.rs` — remove the `async fn subscribe_events` implementation entirely.

Edit `crates/minos-daemon/src/agent.rs` — the `AgentGlue` still exists for state observer; remove `event_stream()` method and any field that held a `broadcast::Receiver<AgentEvent>`.

Edit `crates/minos-daemon/src/handle.rs` — remove any `DaemonHandle::event_stream` method (if present) and its callers.

- [ ] **Step 5: Rewire agent-runtime's broadcast channel**

Edit `crates/minos-agent-runtime/src/runtime.rs`:

Replace:
```rust
event_bus: broadcast::Sender<AgentEvent>,
```

With:
```rust
/// Outbound channel of raw (agent, thread_id, seq, payload, ts_ms) tuples
/// for the ingest pipeline. The translate-to-`AgentEvent` step is gone.
ingest_tx: broadcast::Sender<RawIngest>,
```

Where `RawIngest` is a new struct:

```rust
#[derive(Debug, Clone)]
pub struct RawIngest {
    pub agent: minos_domain::AgentName,
    pub thread_id: String,
    pub payload: serde_json::Value,
    pub ts_ms: i64,
}
```

The seq is assigned by the `Ingestor` in B4. `RawIngest` does NOT carry seq because the runtime is the source; the seq is a transport concern.

Remove `AgentRuntime::event_stream` method. Callers (there are none after B3 Step 4) go away.

- [ ] **Step 6: Update `codex_client.rs` to route notifications into `ingest_tx`**

Edit `crates/minos-agent-runtime/src/codex_client.rs`. Where a codex `Notification` is parsed, replace any `translate_codex_notification()` call with:

```rust
// Forward raw for ingest — no translation here.
let _ = self.ingest_tx.send(RawIngest {
    agent: AgentName::Codex,
    thread_id: self.current_thread_id.clone().unwrap_or_default(),
    payload: raw_notification_value,
    ts_ms: current_unix_ms(),
});
```

(Exact call site depends on the current `codex_client` shape; preserve error handling and logging.)

- [ ] **Step 7: Run check-all**

```bash
cargo xtask check-all
```

Expected: if broadcast send doesn't compile (no subscribers yet), fall back to `let _ = ...` — the channel exists but nothing listens. In B4/B5 we wire the `Ingestor` and the broadcast becomes consumed.

Fix any stragglers. Common:
- `minos_domain::events` still imported in some module — remove.
- Re-exports of `AgentEvent` in ffi crates — remove.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor: delete AgentEvent and subscribe_events; runtime emits raw tuples"
```

## Task B4: Agent-runtime `Ingestor`

**Files:**
- Create: `crates/minos-agent-runtime/src/ingest.rs`
- Modify: `crates/minos-agent-runtime/src/runtime.rs`
- Modify: `crates/minos-agent-runtime/src/lib.rs`
- Modify: `crates/minos-agent-runtime/Cargo.toml` (add `tokio-tungstenite`, `minos-protocol` if not already)

Scope note: the `Ingestor` is a small, bespoke envelope-speaking WS client for the agent-host. It is not `minos-transport::WsClient` because it doesn't speak jsonrpsee.

- [ ] **Step 1: Add dependencies**

Edit `crates/minos-agent-runtime/Cargo.toml`:

```toml
tokio-tungstenite = { workspace = true }
minos-protocol    = { path = "../minos-protocol" }
```

- [ ] **Step 2: Write the unit test — a fake backend + a single ingested event**

Create `crates/minos-agent-runtime/tests/ingest_integration.rs` with a fake tokio-tungstenite server that accepts one connection and asserts the frame format:

```rust
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn ingestor_sends_one_envelope() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (_write, mut read) = ws.split();
        let msg = read.next().await.unwrap().unwrap();
        let text = msg.into_text().unwrap();
        let env: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(env["kind"], "ingest");
        assert_eq!(env["agent"], "codex");
        assert_eq!(env["thread_id"], "thr_1");
        assert_eq!(env["seq"], 1);
    });

    let (ingestor, _handle) = minos_agent_runtime::ingest::Ingestor::connect(
        &format!("ws://{}", addr), "device-id", None,
    ).await.unwrap();

    ingestor.push(
        minos_domain::AgentName::Codex,
        "thr_1",
        serde_json::json!({"method":"item/started"}),
    ).await.unwrap();

    server.await.unwrap();
}
```

- [ ] **Step 3: Run; confirm failure**

```bash
cargo test -p minos-agent-runtime --test ingest_integration
```

Expected: FAIL (crate::ingest doesn't exist).

- [ ] **Step 4: Implement `ingest.rs`**

File: `crates/minos-agent-runtime/src/ingest.rs`.

```rust
//! Agent-host → backend ingest WS client. Bespoke envelope-speaking
//! loop; not `minos-transport::WsClient` (that crate wraps jsonrpsee).
//!
//! On boot, the Ingestor:
//! 1. Connects to `ws://127.0.0.1:8787/devices` with
//!    `X-Device-Id` + `X-Device-Secret` (+ `X-Device-Role: agent-host`)
//!    headers pulled from the agent-host's PairingStore.
//! 2. Receives `Event::Paired` or `Event::Unpaired` (via envelope).
//!    (Pairing itself still flows via `LocalRpc::Pair` — see spec §7.3;
//!    first-boot pairing is driven by the outer AgentRuntime.)
//! 3. Exposes `push(agent, thread_id, payload)` — builds an
//!    `Envelope::Ingest`, fires it at the outbound mpsc channel.
//!
//! Per-thread seq counters live here. On reconnect, the counters persist
//! in memory; the backend idempotent-inserts so retransmits are safe.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use minos_domain::{AgentName, DeviceId, DeviceSecret, MinosError};
use minos_protocol::Envelope;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

pub struct Ingestor {
    tx: mpsc::Sender<Envelope>,
    seqs: Arc<DashMap<String, u64>>,
}

pub struct IngestorHandle {
    /// JoinHandle for the inbound receive task.
    _recv_handle: tokio::task::JoinHandle<()>,
    /// JoinHandle for the outbound send task.
    _send_handle: tokio::task::JoinHandle<()>,
}

impl Ingestor {
    /// Connect to backend; returns a `(Ingestor, IngestorHandle)` pair.
    /// Drop the handle to close the WS (graceful).
    pub async fn connect(
        url: &str,
        device_id: &str,
        device_secret: Option<&str>,
    ) -> Result<(Self, IngestorHandle), MinosError> {
        let mut req: Request = url.into_client_request().map_err(|e| MinosError::ConnectFailed {
            url: url.to_string(),
            message: e.to_string(),
        })?;
        req.headers_mut().insert("X-Device-Id", device_id.parse().unwrap());
        req.headers_mut().insert("X-Device-Role", "agent-host".parse().unwrap());
        if let Some(sec) = device_secret {
            req.headers_mut().insert("X-Device-Secret", sec.parse().unwrap());
        }
        let (ws, _resp) = connect_async(req).await.map_err(|e| MinosError::ConnectFailed {
            url: url.to_string(),
            message: e.to_string(),
        })?;
        let (mut write, mut read) = ws.split();

        let (tx, mut rx) = mpsc::channel::<Envelope>(256);

        let send_handle = tokio::spawn(async move {
            while let Some(env) = rx.recv().await {
                let text = match serde_json::to_string(&env) {
                    Ok(s) => s,
                    Err(e) => { tracing::warn!(?e, "ingest envelope serialise failed"); continue; }
                };
                if let Err(e) = write.send(Message::Text(text)).await {
                    tracing::warn!(?e, "ingest WS write failed; channel closed");
                    break;
                }
            }
        });

        let recv_handle = tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(t)) => {
                        // ignore Event frames for now — future: forward to
                        // a subscriber channel if agent-host needs
                        // `PeerOnline` / `ServerShutdown` events.
                        tracing::debug!(text = %t, "ingest WS inbound");
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(e) => { tracing::warn!(?e, "ingest WS read"); break; }
                }
            }
        });

        Ok((
            Self { tx, seqs: Arc::new(DashMap::new()) },
            IngestorHandle { _recv_handle: recv_handle, _send_handle: send_handle },
        ))
    }

    /// Send one raw event for ingest. Blocks on the outbound mpsc channel
    /// if the send task is slow (bounded backpressure).
    pub async fn push(
        &self,
        agent: AgentName,
        thread_id: &str,
        payload: serde_json::Value,
    ) -> Result<(), MinosError> {
        let seq = self.next_seq(thread_id);
        let env = Envelope::Ingest {
            version: 1,
            agent,
            thread_id: thread_id.to_string(),
            seq,
            payload,
            ts_ms: current_unix_ms(),
        };
        self.tx.send(env).await.map_err(|_| MinosError::Disconnected)?;
        Ok(())
    }

    fn next_seq(&self, thread_id: &str) -> u64 {
        let mut entry = self.seqs.entry(thread_id.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }
}

fn current_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
```

- [ ] **Step 5: Wire into `lib.rs`**

Edit `crates/minos-agent-runtime/src/lib.rs`:

```rust
pub mod ingest;
```

- [ ] **Step 6: Run**

```bash
cargo test -p minos-agent-runtime --test ingest_integration
```

Expected: PASS. If the server side doesn't fully accept due to missing auth, the test server in Step 2 is tolerant (it just reads the first frame). Any failure here probably indicates serialisation drift.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(agent-runtime): Ingestor envelope WS client"
```

## Task B5: Backend store — `threads` + `raw_events`

**Files:**
- Create: `crates/minos-backend/src/store/threads.rs`
- Create: `crates/minos-backend/src/store/raw_events.rs`
- Modify: `crates/minos-backend/src/store/mod.rs`

- [ ] **Step 1: Write unit tests for threads store**

Create `crates/minos-backend/src/store/threads.rs` stub + tests in-file:

```rust
//! Thread CRUD (see spec §9.1).

use minos_domain::{AgentName, MinosError};
use minos_ui_protocol::ThreadEndReason;
use sqlx::SqlitePool;

pub async fn upsert(
    pool: &SqlitePool,
    thread_id: &str,
    agent: AgentName,
    owner_device_id: &str,
    ts_ms: i64,
) -> Result<(), MinosError> {
    let agent_s = agent_str(agent);
    sqlx::query(
        r#"INSERT INTO threads (thread_id, agent, owner_device_id, first_ts_ms, last_ts_ms, message_count)
           VALUES (?1, ?2, ?3, ?4, ?4, 0)
           ON CONFLICT(thread_id) DO UPDATE SET last_ts_ms = ?4"#,
    )
    .bind(thread_id)
    .bind(agent_s)
    .bind(owner_device_id)
    .bind(ts_ms)
    .execute(pool).await
    .map_err(|e| MinosError::StoreIo { path: "threads".into(), message: e.to_string() })?;
    Ok(())
}

pub async fn mark_ended(
    pool: &SqlitePool,
    thread_id: &str,
    reason: &ThreadEndReason,
    ts_ms: i64,
) -> Result<(), MinosError> {
    let reason_json = serde_json::to_string(reason).unwrap();
    sqlx::query(r#"UPDATE threads SET ended_at_ms = ?1, end_reason = ?2 WHERE thread_id = ?3"#)
        .bind(ts_ms).bind(reason_json).bind(thread_id)
        .execute(pool).await
        .map_err(|e| MinosError::StoreIo { path: "threads".into(), message: e.to_string() })?;
    Ok(())
}

pub async fn update_title(
    pool: &SqlitePool,
    thread_id: &str,
    title: &str,
) -> Result<(), MinosError> {
    sqlx::query(r#"UPDATE threads SET title = ?1 WHERE thread_id = ?2"#)
        .bind(title).bind(thread_id)
        .execute(pool).await
        .map_err(|e| MinosError::StoreIo { path: "threads".into(), message: e.to_string() })?;
    Ok(())
}

pub async fn increment_message_count(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<(), MinosError> {
    sqlx::query(r#"UPDATE threads SET message_count = message_count + 1 WHERE thread_id = ?1"#)
        .bind(thread_id).execute(pool).await
        .map_err(|e| MinosError::StoreIo { path: "threads".into(), message: e.to_string() })?;
    Ok(())
}

fn agent_str(a: AgentName) -> &'static str {
    match a { AgentName::Codex => "codex", AgentName::Claude => "claude", AgentName::Gemini => "gemini" }
}

// (ListThreadsRow type + list() fn implemented in C2)

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query(r#"INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at)
                       VALUES ('dev1','Dev','agent-host',0,0)"#)
            .execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn upsert_inserts_then_bumps_last_ts() {
        let pool = fresh_pool().await;
        upsert(&pool, "thr1", AgentName::Codex, "dev1", 1000).await.unwrap();
        upsert(&pool, "thr1", AgentName::Codex, "dev1", 2000).await.unwrap();
        let last: i64 = sqlx::query_scalar("SELECT last_ts_ms FROM threads WHERE thread_id = 'thr1'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(last, 2000);
    }

    #[tokio::test]
    async fn mark_ended_stores_reason_json() {
        let pool = fresh_pool().await;
        upsert(&pool, "thr1", AgentName::Codex, "dev1", 1000).await.unwrap();
        mark_ended(&pool, "thr1", &ThreadEndReason::HostDisconnected, 2000).await.unwrap();
        let reason: String = sqlx::query_scalar("SELECT end_reason FROM threads WHERE thread_id = 'thr1'")
            .fetch_one(&pool).await.unwrap();
        assert!(reason.contains("host_disconnected"));
    }
}
```

- [ ] **Step 2: Create raw_events store**

File: `crates/minos-backend/src/store/raw_events.rs`.

```rust
use minos_domain::{AgentName, MinosError};
use serde_json::Value;
use sqlx::SqlitePool;

pub struct RawEventRow {
    pub seq: i64,
    pub agent: AgentName,
    pub payload: Value,
    pub ts_ms: i64,
}

/// Insert one raw event. If `(thread_id, seq)` already exists, returns
/// `Ok(false)` (caller decides whether that is a retransmit or a bug).
pub async fn insert_if_absent(
    pool: &SqlitePool,
    thread_id: &str,
    seq: u64,
    agent: AgentName,
    payload: &Value,
    ts_ms: i64,
) -> Result<bool, MinosError> {
    let payload_s = serde_json::to_string(payload).unwrap();
    let agent_s = match agent {
        AgentName::Codex => "codex", AgentName::Claude => "claude", AgentName::Gemini => "gemini"
    };
    let result = sqlx::query(
        r#"INSERT OR IGNORE INTO raw_events (thread_id, seq, agent, payload_json, ts_ms)
           VALUES (?1, ?2, ?3, ?4, ?5)"#,
    )
    .bind(thread_id).bind(seq as i64).bind(agent_s).bind(payload_s).bind(ts_ms)
    .execute(pool).await
    .map_err(|e| MinosError::StoreIo { path: "raw_events".into(), message: e.to_string() })?;
    Ok(result.rows_affected() == 1)
}

pub async fn read_range(
    pool: &SqlitePool,
    thread_id: &str,
    from_seq: u64,
    limit: u32,
) -> Result<Vec<RawEventRow>, MinosError> {
    let rows = sqlx::query_as::<_, (i64, String, String, i64)>(
        r#"SELECT seq, agent, payload_json, ts_ms FROM raw_events
           WHERE thread_id = ?1 AND seq >= ?2
           ORDER BY seq ASC LIMIT ?3"#,
    )
    .bind(thread_id).bind(from_seq as i64).bind(limit as i64)
    .fetch_all(pool).await
    .map_err(|e| MinosError::StoreIo { path: "raw_events".into(), message: e.to_string() })?;

    rows.into_iter().map(|(seq, agent, payload, ts_ms)| {
        let agent = match agent.as_str() {
            "codex" => AgentName::Codex, "claude" => AgentName::Claude, "gemini" => AgentName::Gemini,
            other => return Err(MinosError::StoreCorrupt { path: "raw_events.agent".into(), message: other.to_string() })
        };
        let payload = serde_json::from_str(&payload)
            .map_err(|e| MinosError::StoreCorrupt { path: "raw_events.payload_json".into(), message: e.to_string() })?;
        Ok(RawEventRow { seq, agent, payload, ts_ms })
    }).collect()
}

pub async fn last_seq(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<u64, MinosError> {
    let v: Option<i64> = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) FROM raw_events WHERE thread_id = ?1"
    ).bind(thread_id).fetch_one(pool).await
     .map_err(|e| MinosError::StoreIo { path: "raw_events".into(), message: e.to_string() })?;
    Ok(v.unwrap_or(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at) VALUES ('dev1','Dev','agent-host',0,0)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO threads (thread_id, agent, owner_device_id, first_ts_ms, last_ts_ms) VALUES ('thr1','codex','dev1',0,0)").execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_is_idempotent() {
        let pool = fresh_pool().await;
        let payload = serde_json::json!({"x":1});
        assert!(insert_if_absent(&pool, "thr1", 1, AgentName::Codex, &payload, 100).await.unwrap());
        assert!(!insert_if_absent(&pool, "thr1", 1, AgentName::Codex, &payload, 100).await.unwrap());
    }

    #[tokio::test]
    async fn read_range_returns_in_order() {
        let pool = fresh_pool().await;
        for i in 1..=5 {
            let _ = insert_if_absent(&pool, "thr1", i, AgentName::Codex, &serde_json::json!({"i":i}), (i as i64)*100).await.unwrap();
        }
        let rows = read_range(&pool, "thr1", 2, 10).await.unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].seq, 2);
        assert_eq!(rows[3].seq, 5);
    }
}
```

- [ ] **Step 3: Wire into `store/mod.rs`**

Edit `crates/minos-backend/src/store/mod.rs`:

```rust
pub mod threads;
pub mod raw_events;
```

- [ ] **Step 4: Run**

```bash
cargo test -p minos-backend store::
```

Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(backend): threads + raw_events store CRUD"
```

## Task B6: Backend ingest handler — store + translate + fan-out

**Files:**
- Create: `crates/minos-backend/src/ingest/mod.rs`
- Create: `crates/minos-backend/src/ingest/translate.rs`
- Modify: `crates/minos-backend/src/http/ws_devices.rs` (dispatch `Envelope::Ingest`)
- Modify: `crates/minos-backend/src/lib.rs`
- Modify: `crates/minos-backend/Cargo.toml` (add `minos-ui-protocol` dep)

- [ ] **Step 1: Add dep**

Edit `crates/minos-backend/Cargo.toml`:

```toml
minos-ui-protocol = { path = "../minos-ui-protocol" }
```

- [ ] **Step 2: Write the integration test first**

Create `crates/minos-backend/tests/ingest_roundtrip.rs`. See the Task B6 test body in Step 5 below; write it here first.

Key assertion: a host sends `Envelope::Ingest{agent:Codex, thread_id:"thr1", seq:1, payload:<codex thread/started>}`; the backend writes a row to `raw_events`, adds one to `threads`, and emits an `EventKind::UiEventMessage{ui: ThreadOpened}` on a mobile subscriber's outbound channel.

- [ ] **Step 3: Run; confirm failure**

```bash
cargo test -p minos-backend --test ingest_roundtrip
```

Expected: FAIL (no handler yet).

- [ ] **Step 4: Implement ingest translate helper**

File: `crates/minos-backend/src/ingest/translate.rs`.

```rust
//! Backend-side wrapper around `minos-ui-protocol`. Keeps per-thread
//! `CodexTranslatorState` in a `DashMap`; mobile subscribers see the
//! translated events via fan-out.

use std::sync::Arc;

use dashmap::DashMap;
use minos_domain::AgentName;
use minos_ui_protocol::{translate_codex, CodexTranslatorState, UiEventMessage};
use serde_json::Value;

pub struct ThreadTranslators {
    codex: DashMap<String, CodexTranslatorState>,
}

impl ThreadTranslators {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { codex: DashMap::new() })
    }

    pub fn translate(
        &self,
        agent: AgentName,
        thread_id: &str,
        payload: &Value,
    ) -> Result<Vec<UiEventMessage>, minos_ui_protocol::TranslationError> {
        match agent {
            AgentName::Codex => {
                let mut state = self.codex.entry(thread_id.to_string())
                    .or_insert_with(|| CodexTranslatorState::new(thread_id.to_string()));
                translate_codex(&mut state, payload)
            }
            AgentName::Claude => minos_ui_protocol::translate_claude(payload),
            AgentName::Gemini => minos_ui_protocol::translate_gemini(payload),
        }
    }

    pub fn drop_thread(&self, thread_id: &str) {
        self.codex.remove(thread_id);
    }
}
```

- [ ] **Step 5: Implement ingest dispatcher**

File: `crates/minos-backend/src/ingest/mod.rs`.

```rust
use std::sync::Arc;

use minos_domain::AgentName;
use minos_protocol::{Envelope, EventKind};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::ingest::translate::ThreadTranslators;
use crate::session::SessionRegistry;
use crate::store::{threads, raw_events};

pub mod translate;

/// Dispatch one `Envelope::Ingest`. Called from the WS handler.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch(
    pool: &SqlitePool,
    registry: &SessionRegistry,
    translators: &ThreadTranslators,
    agent: AgentName,
    thread_id: &str,
    seq: u64,
    payload: &Value,
    ts_ms: i64,
    owner_device_id: &str,
) -> Result<(), crate::error::RelayError> {
    // 1. Ensure threads row.
    threads::upsert(pool, thread_id, agent, owner_device_id, ts_ms).await?;

    // 2. Persist raw; dedupe on (thread_id, seq).
    let inserted = raw_events::insert_if_absent(pool, thread_id, seq, agent, payload, ts_ms).await?;
    if !inserted {
        tracing::debug!(thread_id, seq, "ingest seq retransmit, dropping");
        return Ok(());
    }

    // 3. Translate + fan out.
    let translated = match translators.translate(agent, thread_id, payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(?e, thread_id, "translation failed");
            // Emit a synthesized Error ui event so mobile sees something.
            vec![minos_ui_protocol::UiEventMessage::Error {
                code: "translation_failed".into(),
                message: format!("{e}"),
                message_id: None,
            }]
        }
    };

    for ui in translated {
        // Side effect: if this ui event opens a thread, we already upsert'd above.
        // If it updates title, patch the threads row here.
        if let minos_ui_protocol::UiEventMessage::ThreadTitleUpdated { title, .. } = &ui {
            let _ = threads::update_title(pool, thread_id, title).await;
        }

        // Fan out to all paired mobile peers for this owner.
        let env = Envelope::Event {
            version: 1,
            event: EventKind::UiEventMessage {
                thread_id: thread_id.to_string(),
                seq,
                ui,
                ts_ms,
            },
        };
        registry.broadcast_to_peers_of(owner_device_id, &env).await;
    }

    Ok(())
}
```

(`SessionRegistry::broadcast_to_peers_of` is a helper on the existing registry; see §Step 7.)

- [ ] **Step 6: Register the module**

Edit `crates/minos-backend/src/lib.rs`:

```rust
pub mod ingest;
```

- [ ] **Step 7: Add `broadcast_to_peers_of` to `SessionRegistry`**

Edit `crates/minos-backend/src/session/registry.rs`. Add:

```rust
impl SessionRegistry {
    /// Send `env` to every session that is paired with `device_id`.
    /// No-op if no peers online or no pairing exists.
    pub async fn broadcast_to_peers_of(&self, device_id: &str, env: &Envelope) {
        // Look up pairings for device_id; for each peer, find its session.
        // Implementation uses the existing pairings cache in the registry.
        // ... use the registry's existing lookup helper, e.g.
        // for peer in self.paired_peers_of(device_id) {
        //     if let Some(handle) = self.get(&peer) {
        //         let _ = handle.outbox.send(env.clone()).await;
        //     }
        // }
    }
}
```

Exact lookup code depends on the existing registry shape (read `crates/minos-backend/src/session/registry.rs` first); adapt.

- [ ] **Step 8: Wire into `ws_devices.rs`**

Edit `crates/minos-backend/src/http/ws_devices.rs`. In the envelope receive loop, add:

```rust
Envelope::Ingest { version: _, agent, thread_id, seq, payload, ts_ms } => {
    // Only allow from agent-host role.
    if session.role != Role::AgentHost {
        session.close_with(4401, "ingest forbidden for non-host").await;
        break;
    }
    crate::ingest::dispatch(
        &state.pool, &state.registry, &state.translators,
        agent, &thread_id, seq, &payload, ts_ms, &session.device_id,
    ).await?;
}
```

And construct a `ThreadTranslators` on server start-up, stashing it in `RelayState`:

```rust
let translators = ThreadTranslators::new();
let state = RelayState::new(registry.clone(), pairing.clone(), pool.clone(), cfg.token_ttl(), translators.clone());
```

Update `RelayState` accordingly.

- [ ] **Step 9: Write the integration test body**

File: `crates/minos-backend/tests/ingest_roundtrip.rs`.

```rust
use futures_util::{SinkExt, StreamExt};
use minos_protocol::{Envelope, EventKind};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn ingest_translates_and_fans_out() {
    // Spin up the backend on an ephemeral port against an in-memory SQLite.
    let pool = minos_backend::store::connect("sqlite://:memory:").await.unwrap();
    let registry = std::sync::Arc::new(minos_backend::session::SessionRegistry::new());
    let pairing = std::sync::Arc::new(minos_backend::pairing::PairingService::new(pool.clone()));
    let translators = minos_backend::ingest::translate::ThreadTranslators::new();
    let state = minos_backend::http::RelayState::new(
        registry, pairing, pool.clone(), std::time::Duration::from_secs(300), translators,
    );
    let router = minos_backend::http::router(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    // Insert a pre-paired host + mobile device pair directly in the DB
    // (skip the full pairing dance for test brevity).
    sqlx::query("INSERT INTO devices (device_id, display_name, role, secret_hash, created_at, last_seen_at)
                 VALUES ('hostA','Host','agent-host','--','0','0'),
                        ('phoneA','Phone','ios-client','--','0','0')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO pairings (device_a, device_b, created_at)
                 VALUES ('hostA','phoneA',0)")
        .execute(&pool).await.unwrap();

    // Open a paired mobile WS. (Backend currently checks the secret; in
    // tests we bypass by inserting a fixed known hash or by using a test
    // hook — adapt as needed.)
    let mobile_url = format!("ws://{addr}/devices");
    let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(mobile_url.as_str()).unwrap();
    req.headers_mut().insert("X-Device-Id", "phoneA".parse().unwrap());
    req.headers_mut().insert("X-Device-Role", "ios-client".parse().unwrap());
    // ... insert X-Device-Secret using whatever test helper the backend uses
    let (mut mobile_ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();

    // Open a paired host WS and send one Ingest frame.
    let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(mobile_url.as_str()).unwrap();
    req.headers_mut().insert("X-Device-Id", "hostA".parse().unwrap());
    req.headers_mut().insert("X-Device-Role", "agent-host".parse().unwrap());
    let (mut host_ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();

    let ingest = Envelope::Ingest {
        version: 1,
        agent: minos_domain::AgentName::Codex,
        thread_id: "thr_test".into(),
        seq: 1,
        payload: serde_json::json!({"method":"thread/started","params":{"threadId":"thr_test","createdAtMs":1}}),
        ts_ms: 1,
    };
    host_ws.send(Message::Text(serde_json::to_string(&ingest).unwrap())).await.unwrap();

    // Wait for the mobile side to receive the translated event.
    let recv = tokio::time::timeout(std::time::Duration::from_secs(2), mobile_ws.next()).await.unwrap().unwrap().unwrap();
    let env: Envelope = serde_json::from_str(recv.to_text().unwrap()).unwrap();
    match env {
        Envelope::Event { event: EventKind::UiEventMessage { ui, thread_id, seq, .. }, .. } => {
            assert_eq!(thread_id, "thr_test");
            assert_eq!(seq, 1);
            assert!(matches!(ui, minos_ui_protocol::UiEventMessage::ThreadOpened { .. }));
        }
        _ => panic!("unexpected envelope"),
    }
}
```

Note: this test may need adjustments to match the backend's actual public API (e.g. how `RelayState::new` is constructed). Adapt as needed based on reading `http/mod.rs` — the surface may already exist but with different names.

- [ ] **Step 10: Run**

```bash
cargo test -p minos-backend --test ingest_roundtrip
```

Expected: PASS.

- [ ] **Step 11: Run whole workspace check**

```bash
cargo xtask check-all
```

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "feat(backend): ingest dispatcher persists raw + translates + fans out UiEventMessage"
```

## ✅ Phase B Reviewer Checkpoint

Dispatch code review on everything since the end of Phase A. Focus:

1. `translate_codex` covers all 12 event kinds listed in spec §12.1.
2. `CodexTranslatorState` handles edge cases (delta with no open message, tool call with unknown id).
3. `Ingestor` robustly handles WS dropouts (for MVP, at-least-once via dedup is acceptable).
4. Backend dispatch is atomic (raw written before fan-out attempted).

---

# Phase C: Backend History + Pairing Rebuild

**Ends when:** `list_threads` / `read_thread` / `get_thread_last_seq` / `request_pairing_qr` are implemented; CF env vars are parsed; an integration test `list_threads.rs` exercises pagination + translation on read.

## Task C1: `list_threads` LocalRpc

**Files:**
- Modify: `crates/minos-backend/src/store/threads.rs` (add `list` fn)
- Modify: `crates/minos-backend/src/envelope/local_rpc.rs` (handler)

- [ ] **Step 1: Unit test for list query**

Append to `crates/minos-backend/src/store/threads.rs`:

```rust
pub async fn list(
    pool: &SqlitePool,
    owner_device_id: Option<&str>,
    agent: Option<AgentName>,
    before_ts_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<minos_protocol::ThreadSummary>, MinosError> {
    let agent_s = agent.map(agent_str);
    let rows = sqlx::query_as::<_, (String, String, Option<String>, i64, i64, i64, Option<i64>, Option<String>)>(
        r#"SELECT thread_id, agent, title, first_ts_ms, last_ts_ms, message_count, ended_at_ms, end_reason
           FROM threads
           WHERE (?1 IS NULL OR owner_device_id = ?1)
             AND (?2 IS NULL OR agent = ?2)
             AND (?3 IS NULL OR last_ts_ms < ?3)
           ORDER BY last_ts_ms DESC
           LIMIT ?4"#,
    )
    .bind(owner_device_id).bind(agent_s).bind(before_ts_ms).bind(limit as i64)
    .fetch_all(pool).await
    .map_err(|e| MinosError::StoreIo { path: "threads".into(), message: e.to_string() })?;

    rows.into_iter().map(|(thread_id, a, title, first_ts_ms, last_ts_ms, message_count, ended_at_ms, end_reason_json)| {
        let agent = match a.as_str() {
            "codex" => AgentName::Codex, "claude" => AgentName::Claude, "gemini" => AgentName::Gemini,
            other => return Err(MinosError::StoreCorrupt { path: "threads.agent".into(), message: other.into() })
        };
        let end_reason = end_reason_json.as_ref().map(|s| serde_json::from_str(s)).transpose()
            .map_err(|e| MinosError::StoreCorrupt { path: "threads.end_reason".into(), message: e.to_string() })?;
        Ok(minos_protocol::ThreadSummary {
            thread_id, agent, title,
            first_ts_ms, last_ts_ms,
            message_count: message_count as u32,
            ended_at_ms, end_reason,
        })
    }).collect()
}
```

Append test:

```rust
#[tokio::test]
async fn list_orders_by_last_ts_desc_and_limits() {
    let pool = fresh_pool().await;
    for i in 0..5 {
        upsert(&pool, &format!("thr{i}"), AgentName::Codex, "dev1", i * 1000).await.unwrap();
    }
    let r = list(&pool, Some("dev1"), None, None, 3).await.unwrap();
    assert_eq!(r.len(), 3);
    assert_eq!(r[0].thread_id, "thr4");
    assert_eq!(r[2].thread_id, "thr2");
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p minos-backend store::threads::tests
```

Expected: all pass.

- [ ] **Step 3: Wire into LocalRpc dispatcher**

Edit `crates/minos-backend/src/envelope/local_rpc.rs`. Add a match arm:

```rust
LocalRpcMethod::ListThreads => {
    let p: minos_protocol::ListThreadsParams = serde_json::from_value(params.clone())
        .map_err(|e| RelayError::BadRequest(e.to_string()))?;
    let threads = crate::store::threads::list(
        pool,
        Some(&session.device_id),
        p.agent,
        p.before_ts_ms,
        p.limit.min(500),
    ).await?;
    let next_before_ts_ms = threads.last().map(|t| t.last_ts_ms);
    let resp = minos_protocol::ListThreadsResponse { threads, next_before_ts_ms };
    serde_json::to_value(resp).unwrap()
}
```

(Adapt the exact shape of the match context to match the existing dispatcher.)

- [ ] **Step 4: Run check-all**

```bash
cargo xtask check-all
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(backend): list_threads LocalRpc"
```

## Task C2: `read_thread` LocalRpc

**Files:**
- Modify: `crates/minos-backend/src/envelope/local_rpc.rs`

- [ ] **Step 1: Implement the match arm**

```rust
LocalRpcMethod::ReadThread => {
    let p: minos_protocol::ReadThreadParams = serde_json::from_value(params.clone())
        .map_err(|e| RelayError::BadRequest(e.to_string()))?;
    let from_seq = p.from_seq.unwrap_or(0);
    let rows = crate::store::raw_events::read_range(
        pool, &p.thread_id, from_seq, p.limit.min(2000),
    ).await?;

    if rows.is_empty() {
        // Check if thread exists at all.
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM threads WHERE thread_id = ?1")
            .bind(&p.thread_id).fetch_optional(pool).await
            .map_err(|e| RelayError::StoreIo(e.to_string()))?;
        if exists.is_none() {
            return Err(RelayError::from(MinosError::ThreadNotFound { thread_id: p.thread_id.clone() }));
        }
    }

    // Translate each raw row using a *fresh* CodexTranslatorState per thread.
    // (For history reads, we don't share the live translator state —
    // we reconstruct from scratch to guarantee determinism.)
    let mut state = minos_ui_protocol::CodexTranslatorState::new(p.thread_id.clone());
    let mut ui_events = Vec::new();
    let mut last_seq_read = from_seq;
    for row in &rows {
        last_seq_read = row.seq as u64;
        // Only codex translated for now; other agents produce a single Raw placeholder.
        match row.agent {
            AgentName::Codex => {
                match minos_ui_protocol::translate_codex(&mut state, &row.payload) {
                    Ok(v) => ui_events.extend(v),
                    Err(e) => ui_events.push(minos_ui_protocol::UiEventMessage::Error {
                        code: "translation_failed".into(),
                        message: format!("{e}"),
                        message_id: None,
                    }),
                }
            }
            other => {
                ui_events.push(minos_ui_protocol::UiEventMessage::Raw {
                    kind: format!("{other:?}"),
                    payload_json: serde_json::to_string(&row.payload).unwrap_or_default(),
                });
            }
        }
    }

    let next_seq = if rows.len() as u32 == p.limit.min(2000) {
        Some(last_seq_read + 1)
    } else {
        None
    };

    // thread_end_reason lookup (separate query)
    let end_reason_json: Option<Option<String>> = sqlx::query_scalar(
        "SELECT end_reason FROM threads WHERE thread_id = ?1"
    ).bind(&p.thread_id).fetch_optional(pool).await
     .map_err(|e| RelayError::StoreIo(e.to_string()))?;
    let thread_end_reason = end_reason_json.flatten().as_ref()
        .and_then(|s| serde_json::from_str::<minos_ui_protocol::ThreadEndReason>(s).ok());

    let resp = minos_protocol::ReadThreadResponse { ui_events, next_seq, thread_end_reason };
    serde_json::to_value(resp).unwrap()
}
```

- [ ] **Step 2: Run check-all**

```bash
cargo xtask check-all
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(backend): read_thread LocalRpc translates raw on read"
```

## Task C3: `get_thread_last_seq`

**Files:**
- Modify: `crates/minos-backend/src/envelope/local_rpc.rs`

- [ ] **Step 1: Match arm**

```rust
LocalRpcMethod::GetThreadLastSeq => {
    let p: minos_protocol::GetThreadLastSeqParams = serde_json::from_value(params.clone())
        .map_err(|e| RelayError::BadRequest(e.to_string()))?;
    let last_seq = crate::store::raw_events::last_seq(pool, &p.thread_id).await?;
    serde_json::to_value(minos_protocol::GetThreadLastSeqResponse { last_seq }).unwrap()
}
```

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "feat(backend): get_thread_last_seq LocalRpc"
```

## Task C4: CF Access config + `request_pairing_qr`

**Files:**
- Modify: `crates/minos-backend/src/config.rs`
- Modify: `crates/minos-backend/src/pairing/mod.rs`
- Modify: `crates/minos-backend/src/envelope/local_rpc.rs`

- [ ] **Step 1: Extend config**

Edit `crates/minos-backend/src/config.rs`:

```rust
#[derive(clap::Parser)]
pub struct Config {
    // ... existing fields

    #[arg(long, env = "MINOS_BACKEND_CF_ACCESS_CLIENT_ID")]
    pub cf_access_client_id: Option<String>,

    #[arg(long, env = "MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET")]
    pub cf_access_client_secret: Option<String>,

    #[arg(long, env = "MINOS_BACKEND_PUBLIC_URL", default_value = "ws://127.0.0.1:8787/devices")]
    pub public_url: String,

    #[arg(long, env = "MINOS_BACKEND_ALLOW_DEV", default_value_t = false)]
    pub allow_dev: bool,
}

impl Config {
    /// Validate CF token presence. Called on startup.
    pub fn validate(&self) -> Result<(), String> {
        if !self.allow_dev
            && (self.cf_access_client_id.is_none() || self.cf_access_client_secret.is_none())
            && self.public_url.starts_with("wss://")
        {
            return Err("MINOS_BACKEND_CF_ACCESS_CLIENT_ID/SECRET required when public_url is wss://; set MINOS_BACKEND_ALLOW_DEV=1 to override".into());
        }
        Ok(())
    }
}
```

Invoke `cfg.validate()?` at the top of `main`.

- [ ] **Step 2: Pairing module — QR payload builder**

Edit `crates/minos-backend/src/pairing/mod.rs`. Rename the function that returned `{token, expires_at}` to return a full `PairingQrPayload`:

```rust
pub async fn issue_pairing_qr(
    &self,
    issuer_device_id: &str,
    host_display_name: String,
    backend_url: String,
    cf_access_client_id: Option<String>,
    cf_access_client_secret: Option<String>,
) -> Result<minos_protocol::PairingQrPayload, MinosError> {
    let token = generate_random_hex(32);
    let token_hash = sha256_hex(&token);
    let expires_at_ms = (time_now_ms() + self.token_ttl_ms());
    sqlx::query(
        r#"INSERT INTO pairing_tokens (token_hash, issuer_device_id, created_at, expires_at)
           VALUES (?1, ?2, ?3, ?4)"#,
    )
    .bind(&token_hash).bind(issuer_device_id).bind(time_now_ms()).bind(expires_at_ms)
    .execute(&self.pool).await
    .map_err(|e| MinosError::StoreIo { path: "pairing_tokens".into(), message: e.to_string() })?;

    Ok(minos_protocol::PairingQrPayload {
        v: 2,
        backend_url,
        host_display_name,
        pairing_token: token,
        expires_at_ms,
        cf_access_client_id,
        cf_access_client_secret,
    })
}
```

- [ ] **Step 3: LocalRpc handler rename**

Replace the `RequestPairingToken` match arm body with:

```rust
LocalRpcMethod::RequestPairingQr => {
    if session.role != Role::AgentHost {
        return Err(RelayError::Unauthorized("only agent-host may request pairing QR".into()));
    }
    let p: minos_protocol::RequestPairingQrParams = serde_json::from_value(params.clone())
        .map_err(|e| RelayError::BadRequest(e.to_string()))?;
    let qr = pairing_service.issue_pairing_qr(
        &session.device_id,
        p.host_display_name,
        state.config.public_url.clone(),
        state.config.cf_access_client_id.clone(),
        state.config.cf_access_client_secret.clone(),
    ).await?;
    serde_json::to_value(minos_protocol::RequestPairingQrResponse { qr_payload: qr }).unwrap()
}
```

- [ ] **Step 4: Verify check-all**

```bash
cargo xtask check-all
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(backend): request_pairing_qr assembles full QR payload incl CF tokens"
```

## Task C5: End-to-end history integration test

**Files:**
- Create: `crates/minos-backend/tests/list_threads.rs`

- [ ] **Step 1: Write the test**

```rust
// Outline:
// 1. Bootstrap backend on ephemeral port; seed a paired (host, phone) pair directly in DB.
// 2. Host sends N ingest frames across 3 threads.
// 3. Phone calls LocalRpc::ListThreads(limit=10).
// 4. Assert 3 threads returned, ordered by last_ts_ms desc.
// 5. Phone calls LocalRpc::ReadThread(thread_id=X, from_seq=0, limit=100).
// 6. Assert the translated UiEventMessage stream matches expected.

// Implementation reuses helpers from ingest_roundtrip.rs; factor to a
// tests/common.rs if duplication appears.
```

Fill in the test body following the outline. Use the same pattern as `ingest_roundtrip.rs`; expect ~100–150 lines.

- [ ] **Step 2: Run**

```bash
cargo test -p minos-backend --test list_threads
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "test(backend): list_threads + read_thread end-to-end"
```

## ✅ Phase C Reviewer Checkpoint

Dispatch code review. Focus:
1. SQL query correctness (NULL handling, LIMIT bounds, parameterisation).
2. Translator state freshness on history reads (don't share live state).
3. CF config validation triggers at startup, not at first pair.

---

# Phase D: Mobile (Rust + Flutter)

**Ends when:** mobile app launches on iPhone, scans a real QR v2, connects through CF edge, receives `EventKind::UiEventMessage` frames live, displays thread list and thread view, and survives reconnect.

## Task D1: `minos-mobile` Rust rewrite — envelope WS client

**Files:**
- Rewrite: `crates/minos-mobile/src/client.rs`
- Modify: `crates/minos-mobile/src/store.rs`
- Modify: `crates/minos-mobile/Cargo.toml` (ensure `tokio-tungstenite`, `minos-protocol`, `minos-ui-protocol`)

- [ ] **Step 1: Extend `PairingStore` trait**

Edit `crates/minos-mobile/src/store.rs`:

```rust
#[async_trait::async_trait]
pub trait PairingStore: Send + Sync {
    async fn load_backend_url(&self) -> Result<Option<String>, MinosError>;
    async fn save_backend_url(&self, url: &str) -> Result<(), MinosError>;

    async fn load_cf_access(&self) -> Result<Option<(String, String)>, MinosError>;
    async fn save_cf_access(&self, id: &str, secret: &str) -> Result<(), MinosError>;

    async fn load_device(&self) -> Result<Option<(DeviceId, DeviceSecret)>, MinosError>;
    async fn save_device(&self, id: &DeviceId, secret: &DeviceSecret) -> Result<(), MinosError>;

    async fn clear_all(&self) -> Result<(), MinosError>;
}
```

Update `InMemoryPairingStore` to implement the new methods (holding values in a `tokio::sync::RwLock<Option<...>>`).

- [ ] **Step 2: Rewrite `client.rs`**

Replace the bulk of `crates/minos-mobile/src/client.rs` with an envelope-aware implementation:

```rust
use std::sync::Arc;

use minos_domain::{ConnectionState, DeviceId, DeviceSecret, MinosError};
use minos_pairing::QrPayload;
use minos_protocol::{Envelope, EventKind, LocalRpcMethod, PairingQrPayload, UiEventMessage};
use tokio::sync::{broadcast, watch, Mutex};
use tokio_tungstenite::tungstenite::Message;

pub struct MobileClient {
    store: Arc<dyn crate::store::PairingStore>,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    ui_events_tx: broadcast::Sender<UiEventFrame>,
    outbox: Mutex<Option<tokio::sync::mpsc::Sender<Envelope>>>,
    next_rpc_id: std::sync::atomic::AtomicU64,
    pending: Arc<dashmap::DashMap<u64, tokio::sync::oneshot::Sender<Envelope>>>,
    device_id: DeviceId,
    self_name: String,
}

#[derive(Debug, Clone)]
pub struct UiEventFrame {
    pub thread_id: String,
    pub seq: u64,
    pub ui: UiEventMessage,
    pub ts_ms: i64,
}

impl MobileClient {
    pub fn new(store: Arc<dyn crate::store::PairingStore>, self_name: String) -> Self {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let (ui_events_tx, _) = broadcast::channel(256);
        Self {
            store, state_tx, state_rx, ui_events_tx,
            outbox: Mutex::new(None),
            next_rpc_id: std::sync::atomic::AtomicU64::new(1),
            pending: Arc::new(dashmap::DashMap::new()),
            device_id: DeviceId::new(),
            self_name,
        }
    }

    pub fn events_stream(&self) -> watch::Receiver<ConnectionState> { self.state_rx.clone() }
    pub fn ui_events_stream(&self) -> broadcast::Receiver<UiEventFrame> { self.ui_events_tx.subscribe() }
    pub fn current_state(&self) -> ConnectionState { *self.state_rx.borrow() }

    /// Scan a QR v2 payload (raw JSON). Stores credentials and triggers
    /// connection. Returns on successful `pair`.
    pub async fn pair_with_qr_json(&self, qr_json: String) -> Result<(), MinosError> {
        let qr: PairingQrPayload = serde_json::from_str(&qr_json)
            .map_err(|e| MinosError::StoreCorrupt { path: "qr_payload".into(), message: e.to_string() })?;
        if qr.v != 2 {
            return Err(MinosError::PairingQrVersionUnsupported { version: qr.v });
            // (add this variant to MinosError in task D1 if not present)
        }
        self.store.save_backend_url(&qr.backend_url).await?;
        if let (Some(id), Some(sec)) = (qr.cf_access_client_id.clone(), qr.cf_access_client_secret.clone()) {
            self.store.save_cf_access(&id, &sec).await?;
        }
        self.connect_and_pair(qr).await
    }

    // ... connect + pair impl details; see spec §7.3 for the state flow
}

// ... plus list_threads + read_thread impls
```

Full code omitted for brevity — the exact shape mirrors the `Ingestor` pattern from B4 but with LocalRpc request/response correlation on `id`. Factor common envelope WS client code into a shared helper if it grows; OK to duplicate in MVP.

- [ ] **Step 3: Add `PairingQrVersionUnsupported` error variant**

Edit `crates/minos-domain/src/error.rs` — add to MinosError + ErrorKind + user_message table.

- [ ] **Step 4: Unit tests for `MobileClient`**

New file `crates/minos-mobile/tests/envelope_client.rs`:

```rust
// Test: run a fake backend WS server; have MobileClient::pair_with_qr_json
// connect, send LocalRpc::Pair, server responds LocalRpcResponse;
// client's ConnectionState transitions Unpaired → Pairing → Paired.
```

- [ ] **Step 5: Run check-all**

```bash
cargo xtask check-all
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(mobile): envelope-aware MobileClient; pair v2 + ConnectionState wiring"
```

## Task D2: Mobile `list_threads` / `read_thread` helpers + `ui_events_stream`

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`

- [ ] **Step 1: Implement `list_threads`**

```rust
impl MobileClient {
    pub async fn list_threads(
        &self,
        req: minos_protocol::ListThreadsParams,
    ) -> Result<minos_protocol::ListThreadsResponse, MinosError> {
        let resp = self.local_rpc(LocalRpcMethod::ListThreads, serde_json::to_value(&req).unwrap()).await?;
        serde_json::from_value(resp).map_err(|e| MinosError::RpcCallFailed { method: "list_threads".into(), message: e.to_string() })
    }

    pub async fn read_thread(
        &self,
        req: minos_protocol::ReadThreadParams,
    ) -> Result<minos_protocol::ReadThreadResponse, MinosError> {
        let resp = self.local_rpc(LocalRpcMethod::ReadThread, serde_json::to_value(&req).unwrap()).await?;
        serde_json::from_value(resp).map_err(|e| MinosError::RpcCallFailed { method: "read_thread".into(), message: e.to_string() })
    }

    async fn local_rpc(&self, method: LocalRpcMethod, params: serde_json::Value)
        -> Result<serde_json::Value, MinosError>
    {
        let id = self.next_rpc_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let env = Envelope::LocalRpc { version: 1, id, method, params };
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.insert(id, tx);
        let outbox = self.outbox.lock().await.as_ref()
            .ok_or_else(|| MinosError::Disconnected)?.clone();
        outbox.send(env).await.map_err(|_| MinosError::Disconnected)?;
        let resp_env = rx.await.map_err(|_| MinosError::Disconnected)?;
        match resp_env {
            Envelope::LocalRpcResponse { outcome, .. } => match outcome {
                minos_protocol::LocalRpcOutcome::Ok { result } => Ok(result),
                minos_protocol::LocalRpcOutcome::Err { error } => Err(MinosError::RpcCallFailed {
                    method: "local_rpc".into(), message: format!("{}: {}", error.code, error.message),
                }),
            },
            _ => Err(MinosError::RpcCallFailed { method: "local_rpc".into(), message: "unexpected envelope".into() }),
        }
    }
}
```

- [ ] **Step 2: Wire the inbound receive task to dispatch responses + events**

In the receive loop (construction in `connect_and_pair`), for each inbound envelope:
- `Envelope::LocalRpcResponse { id, .. }` → look up `pending.remove(&id)` → send to the waiting one-shot.
- `Envelope::Event { event: EventKind::UiEventMessage { thread_id, seq, ui, ts_ms } }` → `ui_events_tx.send(UiEventFrame { .. })`.
- `Envelope::Event { event: EventKind::PeerOnline / PeerOffline / ServerShutdown / Paired / Unpaired }` → update `state_tx` accordingly.

- [ ] **Step 3: Run check-all**

```bash
cargo xtask check-all
```

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(mobile): list_threads / read_thread helpers + ui_events_stream dispatch"
```

## Task D3: frb regen + Dart mirror types

**Files:**
- Modify: `crates/minos-ffi-frb/src/api/minos.rs`
- Regenerated: `crates/minos-ffi-frb/src/frb_generated.rs`, `apps/mobile/lib/src/rust/**`

- [ ] **Step 1: Update the frb API surface**

Edit `crates/minos-ffi-frb/src/api/minos.rs`:

```rust
use flutter_rust_bridge::{frb, StreamSink};
use minos_domain::{ConnectionState, DeviceId, DeviceSecret, ErrorKind, Lang, MinosError};
use minos_protocol::{ListThreadsParams, ListThreadsResponse, PairingQrPayload,
                      ReadThreadParams, ReadThreadResponse, ThreadSummary};
use minos_ui_protocol::{UiEventMessage, MessageRole, ThreadEndReason};

#[frb(mirror(ConnectionState))]
pub enum _ConnectionState { Disconnected, Pairing, Connected, Reconnecting }

#[frb(mirror(UiEventMessage))]
pub enum _UiEventMessage { /* all 12 variants mirrored; frb doc: https://frb.jslisp... */ }

#[frb(mirror(MessageRole))]
pub enum _MessageRole { User, Assistant, System }

#[frb(mirror(ThreadEndReason))]
pub enum _ThreadEndReason { UserStopped, AgentDone, Crashed { message: String }, Timeout, HostDisconnected }

#[frb(mirror(ThreadSummary))]
pub struct _ThreadSummary { /* ... */ }

#[frb(mirror(AgentName))]
pub enum _AgentName { Codex, Claude, Gemini }

// ... similar mirrors for other protocol types used by Dart

#[frb(opaque)]
pub struct MobileClient(minos_mobile::MobileClient);

impl MobileClient {
    #[frb(sync)]
    pub fn new(self_name: String) -> Self {
        Self(minos_mobile::MobileClient::new_with_in_memory_store(self_name))
    }

    pub async fn pair_with_qr_json(&self, qr_json: String) -> Result<(), MinosError> {
        self.0.pair_with_qr_json(qr_json).await
    }

    pub async fn list_threads(&self, req: ListThreadsParams) -> Result<ListThreadsResponse, MinosError> {
        self.0.list_threads(req).await
    }

    pub async fn read_thread(&self, req: ReadThreadParams) -> Result<ReadThreadResponse, MinosError> {
        self.0.read_thread(req).await
    }

    #[frb(sync)]
    pub fn current_state(&self) -> ConnectionState { self.0.current_state() }

    pub fn subscribe_state(&self, sink: StreamSink<ConnectionState>) {
        let mut rx = self.0.events_stream();
        tokio::spawn(async move {
            if sink.add(*rx.borrow_and_update()).is_err() { return; }
            while rx.changed().await.is_ok() {
                if sink.add(*rx.borrow()).is_err() { break; }
            }
        });
    }

    pub fn subscribe_ui_events(&self, sink: StreamSink<UiEventFrame>) {
        let mut rx = self.0.ui_events_stream();
        tokio::spawn(async move {
            while let Ok(frame) = rx.recv().await {
                let f = UiEventFrame {
                    thread_id: frame.thread_id,
                    seq: frame.seq,
                    ui: frame.ui,
                    ts_ms: frame.ts_ms,
                };
                if sink.add(f).is_err() { break; }
            }
        });
    }
}

pub struct UiEventFrame {
    pub thread_id: String,
    pub seq: u64,
    pub ui: UiEventMessage,
    pub ts_ms: i64,
}
```

- [ ] **Step 2: Regenerate**

```bash
cargo xtask gen-frb
```

Expected: `apps/mobile/lib/src/rust/` regenerated with new Dart types. If the codegen fails on specific mirror forms, consult frb v2 docs and adjust.

- [ ] **Step 3: Verify drift clean**

```bash
git diff --exit-code apps/mobile/lib/src/rust crates/minos-ffi-frb/src/frb_generated.rs
```

Expected: exit 0 (after adding the fresh generated files to git).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(ffi-frb): mirror UiEventMessage + thread RPCs; regenerate bindings"
```

## Task D4: Flutter `MinosCore` facade update

**Files:**
- Modify: `apps/mobile/lib/domain/minos_core_protocol.dart`
- Modify: `apps/mobile/lib/infrastructure/minos_core.dart`

- [ ] **Step 1: Extend the protocol**

Edit `minos_core_protocol.dart`:

```dart
abstract class MinosCoreProtocol {
  Future<void> pairWithQrJson(String qrJson);

  Stream<ConnectionState> get connectionStates;
  ConnectionState get currentConnectionState;

  Future<ListThreadsResponse> listThreads(ListThreadsParams params);
  Future<ReadThreadResponse> readThread(ReadThreadParams params);
  Stream<UiEventFrame> get uiEvents;
}
```

- [ ] **Step 2: Implement in `minos_core.dart`**

Update `MinosCore` to satisfy the extended protocol; delegate each to the generated `MobileClient`.

- [ ] **Step 3: Run `dart analyze`**

```bash
(cd apps/mobile && dart analyze --fatal-infos)
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(mobile-dart): MinosCore exposes threads + uiEvents"
```

## Task D5: `flutter_secure_storage` Dart PairingStore bridge

**Files:**
- Modify: `apps/mobile/pubspec.yaml` (add dep)
- Create: `apps/mobile/lib/infrastructure/secure_pairing_store.dart`
- Modify: `apps/mobile/lib/main.dart` (wire initialisation)

- [ ] **Step 1: Add dep**

Edit `apps/mobile/pubspec.yaml`:

```yaml
dependencies:
  flutter_secure_storage: ^9.2.0
  # ...existing
```

Run `(cd apps/mobile && flutter pub get)`.

- [ ] **Step 2: Dart-side store**

File: `apps/mobile/lib/infrastructure/secure_pairing_store.dart`.

```dart
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

class SecurePairingStore {
  static const _storage = FlutterSecureStorage();

  Future<String?> loadBackendUrl() => _storage.read(key: 'backend_url');
  Future<void>    saveBackendUrl(String v) => _storage.write(key: 'backend_url', value: v);

  Future<({String id, String secret})?> loadCfAccess() async {
    final id = await _storage.read(key: 'cf_id');
    final sec = await _storage.read(key: 'cf_secret');
    return (id != null && sec != null) ? (id: id, secret: sec) : null;
  }
  Future<void> saveCfAccess(String id, String secret) async {
    await _storage.write(key: 'cf_id', value: id);
    await _storage.write(key: 'cf_secret', value: secret);
  }

  Future<({String id, String secret})?> loadDevice() async {
    final id = await _storage.read(key: 'device_id');
    final sec = await _storage.read(key: 'device_secret');
    return (id != null && sec != null) ? (id: id, secret: sec) : null;
  }
  Future<void> saveDevice(String id, String secret) async {
    await _storage.write(key: 'device_id', value: id);
    await _storage.write(key: 'device_secret', value: secret);
  }

  Future<void> clearAll() async {
    await _storage.deleteAll();
  }
}
```

(Note: the Rust `MobileClient` currently uses `InMemoryPairingStore`. For MVP, the SecurePairingStore on Dart side holds credentials redundantly outside the Rust layer — the Rust side gets them passed in through the pair call + saved by the client logic. A future pass wires a Rust-callable Dart callback PairingStore; for this plan, that indirection is deferred — the Dart-held store just mirrors.)

- [ ] **Step 3: Initialise on app launch**

Edit `main.dart` to construct a singleton `SecurePairingStore`, pass through the Riverpod providers.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(mobile-dart): SecurePairingStore for Keychain persistence"
```

## Task D6: `ThreadListPage` + provider

**Files:**
- Create: `apps/mobile/lib/application/thread_list_provider.dart`
- Create: `apps/mobile/lib/presentation/pages/thread_list_page.dart`
- Create: `apps/mobile/lib/presentation/widgets/thread_list_tile.dart`
- Modify: `apps/mobile/lib/presentation/app.dart` (router)

- [ ] **Step 1: Provider**

File: `apps/mobile/lib/application/thread_list_provider.dart`.

```dart
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

import '../infrastructure/minos_core.dart';
import '../src/rust/api/minos.dart';

part 'thread_list_provider.g.dart';

@Riverpod(keepAlive: false)
class ThreadList extends _$ThreadList {
  @override
  Future<List<ThreadSummary>> build() async {
    final core = ref.read(minosCoreProvider);
    final resp = await core.listThreads(const ListThreadsParams(limit: 50));
    return resp.threads;
  }

  Future<void> refresh() => ref.refresh(threadListProvider.future);
}
```

Run `(cd apps/mobile && dart run build_runner build --delete-conflicting-outputs)` to generate the `.g.dart` file.

- [ ] **Step 2: Widget**

File: `apps/mobile/lib/presentation/pages/thread_list_page.dart`.

```dart
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import '../../application/thread_list_provider.dart';
import '../widgets/thread_list_tile.dart';
import 'thread_view_page.dart';

class ThreadListPage extends ConsumerWidget {
  const ThreadListPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final threadsAsync = ref.watch(threadListProvider);

    return Scaffold(
      appBar: AppBar(title: const Text('会话')),
      body: threadsAsync.when(
        loading:  () => const Center(child: CircularProgressIndicator()),
        error:    (e, _) => Center(child: Text('加载失败: $e')),
        data:     (list) => RefreshIndicator(
          onRefresh: () => ref.read(threadListProvider.notifier).refresh(),
          child: ListView.builder(
            itemCount: list.length,
            itemBuilder: (_, i) => ThreadListTile(
              summary: list[i],
              onTap: () => Navigator.of(context).push(MaterialPageRoute(
                builder: (_) => ThreadViewPage(threadId: list[i].threadId),
              )),
            ),
          ),
        ),
      ),
    );
  }
}
```

- [ ] **Step 3: Tile widget**

File: `apps/mobile/lib/presentation/widgets/thread_list_tile.dart`.

```dart
import 'package:flutter/material.dart';
import '../../src/rust/api/minos.dart';

class ThreadListTile extends StatelessWidget {
  final ThreadSummary summary;
  final VoidCallback? onTap;
  const ThreadListTile({super.key, required this.summary, this.onTap});

  @override
  Widget build(BuildContext ctx) {
    return ListTile(
      leading: _AgentBadge(summary.agent),
      title: Text(summary.title ?? '<untitled>'),
      subtitle: Text(_formatTs(summary.lastTsMs)),
      trailing: summary.endedAtMs != null ? const Icon(Icons.lock) : null,
      onTap: onTap,
    );
  }

  String _formatTs(int ms) {
    final d = DateTime.fromMillisecondsSinceEpoch(ms);
    return d.toLocal().toString();
  }
}

class _AgentBadge extends StatelessWidget {
  final AgentName agent;
  const _AgentBadge(this.agent);
  @override
  Widget build(BuildContext ctx) {
    final (txt, color) = switch (agent) {
      AgentName.codex  => ('CDX', Colors.green),
      AgentName.claude => ('CLD', Colors.purple),
      AgentName.gemini => ('GEM', Colors.blue),
    };
    return Container(
      width: 40, height: 40, decoration: BoxDecoration(color: color, borderRadius: BorderRadius.circular(8)),
      alignment: Alignment.center, child: Text(txt, style: const TextStyle(color: Colors.white, fontWeight: FontWeight.bold)),
    );
  }
}
```

- [ ] **Step 4: Router update**

Edit `apps/mobile/lib/presentation/app.dart`. Replace `HomePage` navigation with `ThreadListPage` when connected.

- [ ] **Step 5: `dart analyze` + `flutter test`**

```bash
(cd apps/mobile && dart analyze --fatal-infos && flutter test)
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(mobile-ui): ThreadListPage + provider + agent-badge tile"
```

## Task D7: `ThreadViewPage` + provider

**Files:**
- Create: `apps/mobile/lib/application/thread_events_provider.dart`
- Create: `apps/mobile/lib/presentation/pages/thread_view_page.dart`
- Create: `apps/mobile/lib/presentation/widgets/ui_event_tile.dart`

- [ ] **Step 1: Provider**

File: `thread_events_provider.dart`:

```dart
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

import '../infrastructure/minos_core.dart';
import '../src/rust/api/minos.dart';

part 'thread_events_provider.g.dart';

@Riverpod(keepAlive: false)
class ThreadEvents extends _$ThreadEvents {
  int _watermark = 0;

  @override
  Future<List<UiEventMessage>> build(String threadId) async {
    final core = ref.read(minosCoreProvider);

    // Initial history load.
    final resp = await core.readThread(ReadThreadParams(threadId: threadId, limit: 500));
    if (resp.nextSeq != null) _watermark = resp.nextSeq! - 1;

    // Live append via subscription.
    final sub = core.uiEvents.listen((frame) {
      if (frame.threadId != threadId) return;
      if (frame.seq <= _watermark) return;
      _watermark = frame.seq;
      state = AsyncData([...state.value ?? [], frame.ui]);
    });
    ref.onDispose(sub.cancel);

    return resp.uiEvents;
  }
}
```

- [ ] **Step 2: Page**

```dart
class ThreadViewPage extends ConsumerWidget {
  final String threadId;
  const ThreadViewPage({super.key, required this.threadId});

  @override
  Widget build(BuildContext ctx, WidgetRef ref) {
    final asyncs = ref.watch(threadEventsProvider(threadId));
    return Scaffold(
      appBar: AppBar(title: Text('Thread $threadId')),
      body: asyncs.when(
        loading: () => const Center(child: CircularProgressIndicator()),
        error:   (e, _) => Center(child: Text('Error: $e')),
        data:    (evts) => ListView.builder(
          itemCount: evts.length,
          itemBuilder: (_, i) => UiEventTile(event: evts[i]),
        ),
      ),
    );
  }
}
```

- [ ] **Step 3: Tile**

```dart
class UiEventTile extends StatelessWidget {
  final UiEventMessage event;
  const UiEventTile({super.key, required this.event});

  @override
  Widget build(BuildContext ctx) {
    final (kindLabel, primary) = _describe(event);
    return ListTile(
      title: Text(kindLabel, style: const TextStyle(fontFamily: 'monospace', fontSize: 12)),
      subtitle: Text(primary, style: const TextStyle(fontFamily: 'monospace')),
    );
  }

  static (String, String) _describe(UiEventMessage e) => switch (e) {
    UiEventMessage_ThreadOpened(:final threadId, :final agent, :final title) =>
      ('ThreadOpened', 'thread=$threadId agent=$agent title=${title ?? ""}'),
    UiEventMessage_MessageStarted(:final messageId, :final role) =>
      ('MessageStarted', 'id=$messageId role=$role'),
    UiEventMessage_TextDelta(:final messageId, :final text) =>
      ('TextDelta', '[$messageId] $text'),
    UiEventMessage_ReasoningDelta(:final messageId, :final text) =>
      ('ReasoningDelta', '[$messageId] $text'),
    UiEventMessage_ToolCallPlaced(:final toolCallId, :final name, :final argsJson) =>
      ('ToolCallPlaced', '$name($toolCallId) args=$argsJson'),
    UiEventMessage_ToolCallCompleted(:final toolCallId, :final output, :final isError) =>
      ('ToolCallCompleted', '$toolCallId isError=$isError out=$output'),
    UiEventMessage_MessageCompleted(:final messageId) =>
      ('MessageCompleted', 'id=$messageId'),
    UiEventMessage_ThreadClosed(:final reason) =>
      ('ThreadClosed', 'reason=$reason'),
    UiEventMessage_ThreadTitleUpdated(:final title) =>
      ('ThreadTitleUpdated', 'title=$title'),
    UiEventMessage_Error(:final code, :final message) =>
      ('Error', '[$code] $message'),
    UiEventMessage_Raw(:final kind, :final payloadJson) =>
      ('Raw', '$kind: $payloadJson'),
  };
}
```

(The generated sealed-class names may differ slightly; adjust to whatever frb produces.)

- [ ] **Step 4: Run analyze + flutter test**

```bash
(cd apps/mobile && dart analyze --fatal-infos && flutter test)
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(mobile-ui): ThreadViewPage renders UiEventMessage stream as plain tiles"
```

## Task D8: PairingPage QR v2 parsing

**Files:**
- Modify: `apps/mobile/lib/presentation/pages/pairing_page.dart`

- [ ] **Step 1: Update parsing**

In the scan-detect callback, replace the old QR parsing with:

```dart
final Map<String, dynamic> payload = jsonDecode(rawBarcodeText);
final int v = payload['v'] as int? ?? 0;
if (v != 2) {
  ShadToaster.of(context).show(ShadToast.destructive(description: const Text('App 版本过旧,请升级')));
  return;
}
// Pass raw string to MinosCore for processing.
await ref.read(pairingControllerProvider.notifier).submit(rawBarcodeText);
```

- [ ] **Step 2: Update `pairing_controller_test.dart`** to feed v2 payloads.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(mobile-ui): PairingPage accepts QR v2 + rejects older versions"
```

## Task D9: Widget tests (minimal)

**Files:**
- Create: `apps/mobile/test/widget/thread_list_page_test.dart`
- Create: `apps/mobile/test/widget/thread_view_page_test.dart`

- [ ] **Step 1: ThreadListPage test**

```dart
testWidgets('ThreadListPage renders N rows', (tester) async {
  final fakeCore = _FakeCore(threads: [
    ThreadSummary(threadId: 'a', agent: AgentName.codex, title: 'Hello', firstTsMs: 0, lastTsMs: 0, messageCount: 3, endedAtMs: null, endReason: null),
    ThreadSummary(threadId: 'b', agent: AgentName.codex, title: null,    firstTsMs: 0, lastTsMs: 0, messageCount: 1, endedAtMs: null, endReason: null),
    ThreadSummary(threadId: 'c', agent: AgentName.claude, title: 'Big',  firstTsMs: 0, lastTsMs: 0, messageCount: 8, endedAtMs: 99, endReason: ThreadEndReason.agentDone),
  ]);
  await tester.pumpWidget(ProviderScope(
    overrides: [minosCoreProvider.overrideWithValue(fakeCore)],
    child: const MaterialApp(home: ThreadListPage()),
  ));
  await tester.pumpAndSettle();
  expect(find.byType(ListTile), findsNWidgets(3));
  expect(find.text('Hello'), findsOneWidget);
  expect(find.text('<untitled>'), findsOneWidget);
  expect(find.byIcon(Icons.lock), findsOneWidget);
});
```

Include a `_FakeCore extends MinosCoreProtocol` stub that returns the provided threads.

- [ ] **Step 2: ThreadViewPage test**

Similar shape: a stub returning 10 `UiEventMessage` values; assert 10 tiles render and "TextDelta" appears in at least one.

- [ ] **Step 3: Run**

```bash
(cd apps/mobile && flutter test)
```

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "test(mobile-ui): minimal widget tests for ThreadListPage + ThreadViewPage"
```

## ✅ Phase D Reviewer Checkpoint

Dispatch code review. Focus:
1. Envelope WS client handles reconnect cleanly; `ui_events_stream` dedup is strict (never duplicates).
2. Dart side's sealed-class pattern match on `UiEventMessage` is exhaustive.
3. `SecurePairingStore` properly deletes on `forget_peer`.

---

# Phase E: Wrap Up

**Ends when:** `cargo xtask check-all` + `flutter test` + 16-box smoke checklist all green; three new ADRs committed; `macos-relay-migration` worktree is gone.

## Task E1: xtask command rename + bootstrap alignment

**Files:**
- Modify: `xtask/src/main.rs`
- Modify: `README.md` (if any `cargo xtask relay-run` strings)

Already done partially in A2 Step 9; audit once more post-Phase-D.

- [ ] **Step 1: Run every xtask command**

```bash
cargo xtask check-all
cargo xtask bootstrap
cargo xtask backend-run --listen 127.0.0.1:9001 --db /tmp/t.db --allow-dev &
```

Ensure all succeed. Kill the backend after verifying.

- [ ] **Step 2: Commit (if any stragglers)**

```bash
git add -A
git commit -m "chore(xtask): finalise backend-run / backend-db-reset command surface"
```

## Task E2: ADRs 0013, 0014, 0015

**Files:**
- Create: `docs/adr/0013-minos-ui-protocol-unified-event-shape.md`
- Create: `docs/adr/0014-backend-assembled-pairing-qr.md`
- Create: `docs/adr/0015-rename-relay-to-backend.md`

Use MADR 4.0 format consistent with ADRs 0009–0012.

- [ ] **Step 1: Write 0013**

Content: Context (codex/claude/gemini differ; chat UI needs one shape); Decision (event-level enum in `minos-ui-protocol` crate); Consequences (pros: single UI code path, forward-compat via Raw; cons: translator state must be maintained per thread). Link to spec §5.2, §6.4, §6.5.

- [ ] **Step 2: Write 0014**

Context (CF token distribution was mac-keychain in relay spec §9.4; broke for open-source binary); Decision (backend holds CF token in env var, distributes via full QR payload); Consequences (pros: one place to rotate, host never sees CF; cons: remote-host bootstrap still unsolved, future spec).

- [ ] **Step 3: Write 0015**

Context (relay became more than just relay: DB, translation, credential distribution); Decision (rename `minos-relay` → `minos-backend`, no compat shim); Consequences (one atomic commit; docs and specs retain historical filenames).

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0013 docs/adr/0014 docs/adr/0015
git commit -m "docs(adr): 0013 ui-protocol; 0014 backend-assembled QR; 0015 rename"
```

## Task E3: Cloudflare tunnel runbook update

**Files:**
- Modify: `docs/ops/cloudflare-tunnel-setup.md` (if it exists; otherwise defer)

- [ ] **Step 1: Check existence**

```bash
ls docs/ops/cloudflare-tunnel-setup.md
```

If missing, this task is a no-op (doc is owed from a separate track).

- [ ] **Step 2: Add env var step (§13.3 of spec)**

Append to step 7:

```
7. (after `sudo cloudflared service install`)
   Set backend env vars via LaunchDaemon plist:
     MINOS_BACKEND_CF_ACCESS_CLIENT_ID=...
     MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET=...
   Restart the backend launchd service.
```

- [ ] **Step 3: Commit**

```bash
git add docs/ops
git commit -m "docs(ops): backend env vars for CF tunnel setup"
```

## Task E4: Real-device smoke run

Follow spec §12.5 checklist (16 boxes). This is manual; maintainer runs and reports results. Each checked box is an independent verification.

- [ ] Box 1 — Backend boot
- [ ] Box 2 — cloudflared status
- [ ] Box 3 — curl /health via CF edge
- [ ] Box 4 — Mac app launches
- [ ] Box 5 — QR visible with `v=2`
- [ ] Box 6 — QR contains CF tokens
- [ ] Box 7 — iPhone scan succeeds
- [ ] Box 8 — codex prompt reaches mobile
- [ ] Box 9 — DB has raw_events rows
- [ ] Box 10 — Pull-to-refresh yields thread
- [ ] Box 11 — Tap thread shows rows
- [ ] Box 12 — Live append works
- [ ] Box 13 — Kill codex → ThreadClosed visible
- [ ] Box 14 — Backend restart auto-recover
- [ ] Box 15 — forget_peer wipes Keychain
- [ ] Box 16 — CF token rotate re-pair

- [ ] **Report results:** paste the xlog tail + a sqlite snapshot (just the thread_id count) into the final commit body.

## Task E5: Final PR

- [ ] **Step 1: Push the branch**

```bash
git push -u origin feat/mobile-and-ui-protocol
```

- [ ] **Step 2: Open the PR**

```bash
gh pr create --title "feat(mobile+backend): relay rename, unified UI protocol, viewer UI" --body "$(cat <<'EOF'
## Summary

- Renames `minos-relay` → `minos-backend` (atomic, no shim).
- Introduces `minos-ui-protocol` with `UiEventMessage` + codex translator (claude/gemini stubs).
- Adds `Envelope::Ingest` + `EventKind::UiEventMessage` + new LocalRpcs (`ListThreads`, `ReadThread`, `GetThreadLastSeq`, `RequestPairingQr`).
- Backend now owns CF Access tokens via env var and embeds them in QR payloads.
- Mobile rewritten to talk envelopes, with a deliberately plain debug viewer (`ThreadListPage` / `ThreadViewPage`).
- Chat UI is out of scope; a follow-up spec will replace the viewer.

## Test plan
- [ ] `cargo xtask check-all`
- [ ] `(cd apps/mobile && flutter test)`
- [ ] Smoke checklist (spec §12.5): 16 boxes on real iPhone over `cloudflared` tunnel
- [ ] Real-device xlog snapshot attached to the closing commit body

## References
- spec: `docs/superpowers/specs/mobile-migration-and-ui-protocol-design.md`
- ADRs: 0013, 0014, 0015
EOF
)"
```

- [ ] **Step 3: Mark plan done**

Move `docs/superpowers/plans/05-mobile-migration-and-ui-protocol.md`'s checkboxes all to `[x]` as a final commit (optional housekeeping).

---

## Global acceptance criteria

All of the following must be true for the plan to close:

1. `cargo xtask check-all` green on `feat/mobile-and-ui-protocol` HEAD.
2. `(cd apps/mobile && flutter test)` green.
3. `cargo test -p minos-ui-protocol` passes ≥ 12 codex golden fixtures.
4. `cargo test -p minos-backend --test ingest_roundtrip` + `--test list_threads` green.
5. `cargo test -p minos-mobile` envelope client tests green.
6. Smoke checklist (§12.5) — all 16 boxes green.
7. ADRs 0013 / 0014 / 0015 present.
8. PR opened with URL reported back.

---

## Appendices

### A. Minimal command cheat sheet

```bash
# Worktree
git worktree add -b feat/mobile-and-ui-protocol ../minos-worktrees/mobile-and-ui-protocol

# Dev loop
cargo xtask check-all
cargo xtask backend-run --listen 127.0.0.1:8787 --db ./minos-backend.db --allow-dev
cargo xtask gen-frb

# Flutter
(cd apps/mobile && flutter pub get && dart run build_runner build --delete-conflicting-outputs)
(cd apps/mobile && dart analyze --fatal-infos && flutter test)
(cd apps/mobile && flutter build ios --simulator --no-codesign)
```

### B. Debugging tips

- **`cargo sqlx prepare` fails**: ensure `MINOS_BACKEND_DB` or local `./minos-backend.db` has the migrations applied; run `cargo run -p minos-backend -- --exit-after-migrate` first.
- **frb codegen drift**: `cargo xtask gen-frb` then `git diff` — commit any regenerated file.
- **WS closed 4401 on mobile**: device_secret in Keychain doesn't match the backend's hashed value; force pairing by clearing SecurePairingStore.
- **Mobile shows empty ThreadListPage after pairing**: backend may not have any threads yet — trigger a codex session on the host first.
- **Translator state bleed across history reads**: `read_thread` must create a *fresh* `CodexTranslatorState` per invocation; verify in §C2 implementation.

### C. Spec ↔ Plan traceability

| Spec section | Plan task |
|---|---|
| §2.1.1 (rename) | A2, E1 |
| §2.1.2 (minos-ui-protocol) | A3, A4, B1, B2 |
| §2.1.3 (deprecate AgentEvent) | B3 |
| §2.1.4 (Envelope::Ingest) | A5, A6 |
| §2.1.5 (EventKind::UiEventMessage) | A5, B6 |
| §2.1.6 (new LocalRpcs + rename) | A5, C1, C2, C3, C4 |
| §2.1.7 (backend persistence) | A7, B5 |
| §2.1.8 (CF centralised) | C4 |
| §2.1.9 (mobile migration) | D1, D2, D3 |
| §2.1.10 (viewer UI) | D4, D5, D6, D7, D8, D9 |
| §2.1.11 (tooling) | E1, E3 |
| §15 (ADRs) | E2 |
| §12.5 (smoke) | E4 |
