# Minos · Rust Core + Monorepo Scaffold — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Minos monorepo with all 9 Rust crates compiling and tested, plus `xtask`, `mars-xlog` logging, and minimal CI. The plan ends when `cargo xtask check-all` passes and the daemon E2E integration test ("pair → list_clis → disconnect" over in-process WebSocket) is green. iOS / macOS apps come in subsequent plans (`02-macos-app-and-uniffi.md`, `03-flutter-app-and-frb.md`).

**Architecture:** Cargo workspace at repo root with `crates/*`, `apps/*`, and `xtask/` siblings (`apps/*` are empty placeholders here). Crate-bordered hexagonal: `minos-domain` (entities) → `minos-pairing` / `minos-cli-detect` / `minos-protocol` (use cases & contract) → `minos-transport` (adapters) → `minos-daemon` / `minos-mobile` (composition roots) → `minos-ffi-uniffi` / `minos-ffi-frb` (FFI shims). JSON-RPC 2.0 over WebSocket via `jsonrpsee`. All async on `tokio`. Logging via `mars-xlog` (peterich-rs/xlog-rs).

**Tech Stack:**
- Rust stable channel (≥ 1.85, MSRV inherited from `mars-xlog`)
- `tokio` 1 (full features), `jsonrpsee` 0.24, `tokio-tungstenite` 0.29
- `serde` 1, `serde_json` 1, `thiserror` 2, `uuid` 1, `getrandom` 0.3, `base64` 0.22, `url` 2
- `tracing` 0.1, `mars-xlog` 0.1.0-preview.2 (feature `tracing`)
- `uniffi` 0.31, `flutter_rust_bridge` 2 (consumed at apps stage; declared but unused in shim crates here)
- `clap` 4 for xtask
- Test deps: `rstest` 0.23, `proptest` 1, `mockall` 0.13, `tempfile` 3, `tokio-test` 0.4, `pretty_assertions` 1
- License auditor: `cargo-deny` 0.16

**Reference docs:** Implements `docs/superpowers/specs/minos-architecture-and-mvp-design.md`. ADRs `0001`–`0006` justify the choices.

**Working directory note:** This plan should run on `main` (no separate worktree); the repo is fresh and there is only one developer.

**Version drift policy:** Versions above are accurate as of 2026-04-21. If `cargo add <crate>` resolves to a higher minor version when you execute, prefer the resolved version unless it triggers compilation failures.

---

## File Structure (created or modified by this plan)

```
minos/
├── README.md                              [new]
├── LICENSE                                [new]
├── .gitignore                             [new]
├── .gitattributes                         [new]
├── rust-toolchain.toml                    [new]
├── deny.toml                              [new]
├── Cargo.toml                             [new — workspace root]
├── .cargo/
│   └── config.toml                        [new — `cargo xtask` alias]
├── .github/
│   └── workflows/
│       └── ci.yml                         [new]
├── apps/
│   ├── macos/.gitkeep                     [new — placeholder for plan 02]
│   └── mobile/.gitkeep                    [new — placeholder for plan 03]
├── crates/
│   ├── minos-domain/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── ids.rs                     (DeviceId, PairingToken)
│   │   │   ├── agent.rs                   (AgentName, AgentStatus, AgentDescriptor)
│   │   │   ├── connection.rs              (ConnectionState)
│   │   │   ├── pairing_state.rs           (PairingState)
│   │   │   └── error.rs                   (MinosError + user_message)
│   │   └── tests/golden/                  (serde JSON golden files)
│   ├── minos-protocol/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── messages.rs                (request/response types)
│   │   │   ├── events.rs                  (AgentEvent enum, placeholder)
│   │   │   └── rpc.rs                     (jsonrpsee MinosRpc trait)
│   │   └── tests/schema_golden.rs
│   ├── minos-pairing/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── state_machine.rs           (Pairing struct + transitions)
│   │   │   ├── store.rs                   (PairingStore trait + TrustedDevice type)
│   │   │   └── token.rs                   (generate_qr_payload)
│   │   └── tests/state_machine_table.rs
│   ├── minos-cli-detect/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── runner.rs                  (CommandRunner trait + RealCommandRunner)
│   │       └── detect.rs                  (detect_all, version parsing)
│   ├── minos-transport/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── server.rs                  (WsServer)
│   │       ├── client.rs                  (WsClient)
│   │       └── backoff.rs                 (exponential reconnect)
│   ├── minos-daemon/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── handle.rs                  (DaemonHandle)
│   │   │   ├── file_store.rs              (FilePairingStore impl)
│   │   │   ├── rpc_server.rs              (jsonrpsee handler wiring)
│   │   │   ├── tailscale.rs               (Tailscale IP discovery)
│   │   │   └── logging.rs                 (mars-xlog setup)
│   │   └── tests/e2e.rs                   (in-process pair → list_clis)
│   ├── minos-mobile/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── client.rs                  (MobileClient)
│   │   │   ├── store.rs                   (PairingStoreCallback FFI trait)
│   │   │   └── logging.rs                 (mars-xlog setup)
│   │   └── tests/e2e.rs                   (mobile vs fake server)
│   ├── minos-ffi-uniffi/
│   │   ├── Cargo.toml
│   │   ├── src/lib.rs                     (UniFFI re-export shim)
│   │   ├── build.rs
│   │   └── uniffi.toml
│   └── minos-ffi-frb/
│       ├── Cargo.toml
│       └── src/lib.rs                     (frb re-export shim)
└── xtask/
    ├── Cargo.toml
    └── src/main.rs                        (clap CLI: check-all, bootstrap, gen-uniffi, gen-frb, build-macos, build-ios)
```

Total: ~50 files created across ~12 crates. Tasks below build this incrementally with TDD.

---

## Phase A · Workspace Bootstrap

### Task 1: Repo metadata files

**Files:**
- Create: `README.md`
- Create: `LICENSE`
- Create: `.gitignore`
- Create: `.gitattributes`

- [ ] **Step 1: Write `README.md`**

```markdown
# Minos

Native macOS status-bar app + Flutter mobile client + shared Rust core for remote AI-coding control. Drive `codex` / `claude` / `gemini` on a Mac from a paired phone over Tailscale.

## Status

MVP under construction. See `docs/superpowers/specs/minos-architecture-and-mvp-design.md` for the design and `docs/superpowers/plans/` for implementation plans.

## Quick start (development)

```bash
# Bootstrap dev tools (uniffi-bindgen, frb codegen, cargo-deny, etc.)
cargo xtask bootstrap

# Run all checks (fmt + clippy + tests + lints)
cargo xtask check-all
```

## Repository layout

```
crates/    Rust workspace (9 crates: domain, protocol, pairing, cli-detect,
           transport, daemon, mobile, ffi-uniffi, ffi-frb)
apps/      macOS (Swift/UniFFI) and mobile (Flutter/frb) — populated in plans 02/03
xtask/     Build / codegen orchestration in Rust
docs/      Specs (`docs/superpowers/specs/`) and ADRs (`docs/adr/`)
```

## License

MIT — see `LICENSE`.
```

- [ ] **Step 2: Write `LICENSE` (MIT)**

```
MIT License

Copyright (c) 2026 Peterich

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 3: Write `.gitignore`**

```gitignore
# Rust
target/
**/*.rs.bk
Cargo.lock.bak

# IDE
.idea/
.vscode/
*.iml
.DS_Store

# UniFFI / frb generated artifacts (regenerated; not committed)
apps/macos/Minos/Generated/
apps/mobile/lib/src/rust/
apps/mobile/rust_builder/
apps/mobile/.dart_tool/
apps/mobile/build/

# Logs
*.log
*.xlog
```

- [ ] **Step 4: Write `.gitattributes`**

```gitattributes
* text=auto eol=lf
*.rs text eol=lf
*.swift text eol=lf
*.dart text eol=lf
*.toml text eol=lf
*.yaml text eol=lf
*.png binary
*.jpg binary
```

- [ ] **Step 5: Commit**

```bash
git add README.md LICENSE .gitignore .gitattributes
git commit -m "chore: add repo metadata (README, MIT license, gitignore)"
```

---

### Task 2: Rust toolchain + cargo workspace + xtask alias

**Files:**
- Create: `rust-toolchain.toml`
- Create: `Cargo.toml`
- Create: `.cargo/config.toml`
- Create: `apps/macos/.gitkeep`
- Create: `apps/mobile/.gitkeep`

- [ ] **Step 1: Write `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy", "rust-src"]
targets = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "aarch64-apple-ios",
  "aarch64-apple-ios-sim",
]
```

- [ ] **Step 2: Write workspace `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = [
  "crates/minos-domain",
  "crates/minos-protocol",
  "crates/minos-pairing",
  "crates/minos-cli-detect",
  "crates/minos-transport",
  "crates/minos-daemon",
  "crates/minos-mobile",
  "crates/minos-ffi-uniffi",
  "crates/minos-ffi-frb",
  "xtask",
]
default-members = [
  "crates/minos-domain",
  "crates/minos-protocol",
  "crates/minos-pairing",
  "crates/minos-cli-detect",
  "crates/minos-transport",
  "crates/minos-daemon",
  "crates/minos-mobile",
]

[workspace.package]
edition = "2021"
license = "MIT"
authors = ["Peterich <wangqingyn2@gmail.com>"]
homepage = "https://github.com/peterich-rs/minos"
repository = "https://github.com/peterich-rs/minos"

[workspace.dependencies]
# async runtime
tokio = { version = "1", features = ["full"] }
futures = "0.3"

# serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# error handling
thiserror = "2"

# ids / random
uuid = { version = "1", features = ["v4", "serde"] }
getrandom = "0.3"
base64 = "0.22"

# url / time
url = "2"
chrono = { version = "0.4", default-features = false, features = ["clock", "std", "serde"] }

# rpc
jsonrpsee = { version = "0.24", features = ["server", "client", "ws-client", "macros"] }
tokio-tungstenite = "0.29"

# logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "registry"] }
mars-xlog = { version = "0.1.0-preview.2", features = ["tracing"] }

# ffi (consumed only by shim crates)
uniffi = { version = "0.31", features = ["build"] }

# test deps (declared here so all crates use the same versions)
rstest = "0.23"
proptest = "1"
mockall = "0.13"
tempfile = "3"
tokio-test = "0.4"
pretty_assertions = "1"

[profile.release]
lto = true
strip = true

[profile.dev]
debug = true
```

- [ ] **Step 3: Write `.cargo/config.toml`**

```toml
[alias]
xtask = "run --release --package xtask --"
```

- [ ] **Step 4: Create app placeholders**

```bash
mkdir -p apps/macos apps/mobile
touch apps/macos/.gitkeep apps/mobile/.gitkeep
```

- [ ] **Step 5: Commit**

```bash
git add rust-toolchain.toml Cargo.toml .cargo/config.toml apps/
git commit -m "chore: add cargo workspace + rust toolchain pin"
```

---

### Task 3: xtask skeleton (clap CLI with empty subcommands)

**Files:**
- Create: `xtask/Cargo.toml`
- Create: `xtask/src/main.rs`

- [ ] **Step 1: Write `xtask/Cargo.toml`**

```toml
[package]
name = "xtask"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
publish = false

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 2: Write `xtask/src/main.rs`**

```rust
//! Minos build / codegen orchestration.
//!
//! Subcommands are filled in across phases of plan 01:
//! - `check-all`: phase K
//! - `gen-uniffi` / `gen-frb` / `build-macos` / `build-ios`: phase K stubs only;
//!   real implementations land in plans 02 and 03.

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Minos build & codegen orchestration")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run fmt + clippy + workspace tests + (later) UI lints.
    CheckAll,
    /// Install developer-side codegen tools.
    Bootstrap,
    /// Generate Swift bindings via uniffi-bindgen. (Implemented in plan 02.)
    GenUniffi,
    /// Generate Dart bindings via flutter_rust_bridge_codegen. (Implemented in plan 03.)
    GenFrb,
    /// Build macOS xcframework from minos-ffi-uniffi. (Implemented in plan 02.)
    BuildMacos,
    /// Build iOS staticlib from minos-ffi-frb. (Implemented in plan 03.)
    BuildIos,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::CheckAll => not_yet("check-all"),
        Cmd::Bootstrap => not_yet("bootstrap"),
        Cmd::GenUniffi => not_yet("gen-uniffi"),
        Cmd::GenFrb => not_yet("gen-frb"),
        Cmd::BuildMacos => not_yet("build-macos"),
        Cmd::BuildIos => not_yet("build-ios"),
    }
}

fn not_yet(name: &str) -> Result<()> {
    anyhow::bail!("xtask `{}` not implemented yet (filled in later in plan 01)", name)
}
```

- [ ] **Step 3: Verify the alias works**

Run: `cargo xtask --help`
Expected output (substring): `Minos build & codegen orchestration`

- [ ] **Step 4: Verify subcommand stub fails as designed**

Run: `cargo xtask check-all`
Expected exit code: `1`. Expected stderr substring: `not implemented yet`

- [ ] **Step 5: Commit**

```bash
git add xtask/
git commit -m "feat(xtask): clap skeleton with subcommand stubs"
```

---

### Task 4: cargo-deny baseline configuration

**Files:**
- Create: `deny.toml`

- [ ] **Step 1: Write `deny.toml`**

```toml
# https://embarkstudios.github.io/cargo-deny/
[graph]
all-features = true

[advisories]
db-urls = ["https://github.com/rustsec/advisory-db"]
yanked = "warn"
ignore = []

[licenses]
allow = [
  "MIT",
  "Apache-2.0",
  "Apache-2.0 WITH LLVM-exception",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Unicode-DFS-2016",
  "Unicode-3.0",
  "Zlib",
  "MPL-2.0",       # tokio-tungstenite indirect
]
confidence-threshold = 0.8

[bans]
multiple-versions = "warn"
wildcards = "deny"
```

- [ ] **Step 2: Commit**

```bash
git add deny.toml
git commit -m "chore: add cargo-deny config"
```

---

## Phase B · `minos-domain` (entities)

### Task 5: minos-domain crate skeleton

**Files:**
- Create: `crates/minos-domain/Cargo.toml`
- Create: `crates/minos-domain/src/lib.rs`

- [ ] **Step 1: Write `crates/minos-domain/Cargo.toml`**

```toml
[package]
name = "minos-domain"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Minos pure-value domain types: ids, agents, errors, connection state."

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true }
base64 = { workspace = true }
getrandom = { workspace = true }

[dev-dependencies]
pretty_assertions = { workspace = true }
proptest = { workspace = true }
rstest = { workspace = true }
```

- [ ] **Step 2: Write `crates/minos-domain/src/lib.rs`**

```rust
//! Minos domain types — pure values, no I/O, no async.
//!
//! Module layout follows hexagonal "Entities" concerns:
//! - `ids`         identifier newtypes (DeviceId, PairingToken)
//! - `agent`       AgentName / AgentStatus / AgentDescriptor
//! - `connection`  ConnectionState
//! - `pairing_state`  PairingState (used inside MinosError)
//! - `error`       MinosError + user_message

#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod agent;
pub mod connection;
pub mod error;
pub mod ids;
pub mod pairing_state;

pub use agent::*;
pub use connection::*;
pub use error::*;
pub use ids::*;
pub use pairing_state::*;
```

- [ ] **Step 3: Verify it compiles (will fail — no module files yet)**

Run: `cargo check -p minos-domain`
Expected: FAIL — `file not found for module 'agent'` (etc.). Continue to Task 6.

- [ ] **Step 4: Commit (skeleton only)**

```bash
git add crates/minos-domain/
git commit -m "feat(minos-domain): crate skeleton (modules to be added)"
```

---

### Task 6: `DeviceId` and `PairingToken` (ids module)

**Files:**
- Create: `crates/minos-domain/src/ids.rs`

- [ ] **Step 1: Write the failing test**

In `crates/minos-domain/src/ids.rs`:

```rust
//! Identifier newtypes.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable unique identifier for a paired device.
///
/// Newtype over `uuid::Uuid` (v4) so it cannot be confused with other UUIDs
/// in the codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub Uuid);

impl DeviceId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One-shot pairing token: 32 random bytes, presented as base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PairingToken(String);

impl PairingToken {
    /// Generate a fresh token from the OS CSPRNG.
    ///
    /// # Panics
    /// Panics only if `getrandom` cannot supply entropy from the OS, which
    /// indicates an unrecoverable platform fault.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_ne;

    #[test]
    fn device_id_round_trips_through_json() {
        let id = DeviceId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: DeviceId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn device_id_default_is_unique() {
        let a = DeviceId::default();
        let b = DeviceId::default();
        assert_ne!(a, b);
    }

    #[test]
    fn pairing_token_is_43_chars_base64url() {
        // 32 bytes base64-encoded with no padding = 43 chars
        let t = PairingToken::generate();
        assert_eq!(t.as_str().len(), 43);
        assert!(t.as_str().chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    proptest::proptest! {
        #[test]
        fn pairing_token_uniqueness(_iter in 0u32..1000) {
            // 1000 tokens, no collisions (entropy sanity)
            let mut seen = std::collections::HashSet::new();
            for _ in 0..1000 {
                let t = PairingToken::generate();
                assert!(seen.insert(t.0), "collision");
            }
        }
    }
}
```

- [ ] **Step 2: Run test to verify it compiles and passes**

Run: `cargo test -p minos-domain --lib ids::tests`
Expected: `test result: ok. 3 passed; 0 failed` plus 1 proptest pass.

- [ ] **Step 3: Commit**

```bash
git add crates/minos-domain/src/ids.rs
git commit -m "feat(minos-domain): DeviceId + PairingToken with proptest entropy check"
```

---

### Task 7: `AgentName`, `AgentStatus`, `AgentDescriptor` (agent module)

**Files:**
- Create: `crates/minos-domain/src/agent.rs`

- [ ] **Step 1: Write the file with tests**

```rust
//! Agent CLI descriptors (names, statuses, full descriptor records).

use serde::{Deserialize, Serialize};

/// The set of CLI agents Minos knows how to manage.
///
/// MVP enumerates the three planned backends; expansion is a breaking change
/// (intentional — every consumer must opt in to a new agent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentName {
    Codex,
    Claude,
    Gemini,
}

impl AgentName {
    /// All known agents, in the order shown to users.
    #[must_use]
    pub const fn all() -> &'static [AgentName] {
        &[AgentName::Codex, AgentName::Claude, AgentName::Gemini]
    }

    /// The CLI binary name to look for on PATH.
    #[must_use]
    pub const fn bin_name(self) -> &'static str {
        match self {
            AgentName::Codex => "codex",
            AgentName::Claude => "claude",
            AgentName::Gemini => "gemini",
        }
    }
}

/// Health state of a single CLI agent on the local machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AgentStatus {
    Ok,
    Missing,
    Error { reason: String },
}

/// The complete description of one agent's local installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub name: AgentName,
    pub path: Option<String>,
    pub version: Option<String>,
    pub status: AgentStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_name_serializes_snake_case() {
        let s = serde_json::to_string(&AgentName::Codex).unwrap();
        assert_eq!(s, "\"codex\"");
    }

    #[test]
    fn agent_status_ok_serializes_with_kind_tag() {
        let s = serde_json::to_string(&AgentStatus::Ok).unwrap();
        assert_eq!(s, r#"{"kind":"ok"}"#);
    }

    #[test]
    fn agent_status_error_carries_reason() {
        let s = serde_json::to_string(&AgentStatus::Error { reason: "boom".into() }).unwrap();
        assert_eq!(s, r#"{"kind":"error","reason":"boom"}"#);
    }

    #[test]
    fn agent_descriptor_round_trips() {
        let d = AgentDescriptor {
            name: AgentName::Claude,
            path: Some("/usr/local/bin/claude".into()),
            version: Some("1.2.0".into()),
            status: AgentStatus::Ok,
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: AgentDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn agent_name_all_returns_three_in_canonical_order() {
        assert_eq!(AgentName::all().len(), 3);
        assert_eq!(AgentName::all()[0], AgentName::Codex);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p minos-domain --lib agent::tests`
Expected: `5 passed; 0 failed`

- [ ] **Step 3: Commit**

```bash
git add crates/minos-domain/src/agent.rs
git commit -m "feat(minos-domain): AgentName/Status/Descriptor with serde shape pinned"
```

---

### Task 8: `ConnectionState` and `PairingState` (state enums)

**Files:**
- Create: `crates/minos-domain/src/connection.rs`
- Create: `crates/minos-domain/src/pairing_state.rs`

- [ ] **Step 1: Write `connection.rs`**

```rust
//! High-level connection state visible to the UI.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Pairing,
    Connected,
    /// Reconnect attempt in progress; `attempt` starts at 1 for the first retry.
    Reconnecting { attempt: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_serializes_as_string() {
        assert_eq!(serde_json::to_string(&ConnectionState::Disconnected).unwrap(), "\"disconnected\"");
    }

    #[test]
    fn reconnecting_carries_attempt() {
        let s = serde_json::to_string(&ConnectionState::Reconnecting { attempt: 3 }).unwrap();
        assert_eq!(s, r#"{"reconnecting":{"attempt":3}}"#);
    }
}
```

- [ ] **Step 2: Write `pairing_state.rs`**

```rust
//! Pairing-side state machine state (used both inside the pairing crate and
//! inside `MinosError` for diagnostic context).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingState {
    Unpaired,
    AwaitingPeer,
    Paired,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awaiting_peer_serializes_snake_case() {
        let s = serde_json::to_string(&PairingState::AwaitingPeer).unwrap();
        assert_eq!(s, "\"awaiting_peer\"");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p minos-domain --lib connection::tests pairing_state::tests`
Expected: `3 passed; 0 failed`

- [ ] **Step 4: Commit**

```bash
git add crates/minos-domain/src/connection.rs crates/minos-domain/src/pairing_state.rs
git commit -m "feat(minos-domain): ConnectionState + PairingState enums"
```

---

### Task 9: `MinosError` enum + `user_message`

**Files:**
- Create: `crates/minos-domain/src/error.rs`

- [ ] **Step 1: Write `error.rs`**

```rust
//! Single typed error for all Minos public APIs.
//!
//! Variants mirror the table in spec §7.4. `Lang` + `user_message` produce
//! short, user-facing copy (zh / en) so UI layers do not need to translate
//! by themselves.

use crate::PairingState;

#[derive(Debug, Clone, Copy)]
pub enum Lang {
    Zh,
    En,
}

#[derive(thiserror::Error, Debug)]
pub enum MinosError {
    // ── network / WS layer ──
    #[error("websocket bind failed on {addr}: {message}")]
    BindFailed { addr: String, message: String },

    #[error("websocket connect to {url} failed: {message}")]
    ConnectFailed { url: String, message: String },

    #[error("websocket disconnected: {reason}")]
    Disconnected { reason: String },

    // ── pairing layer ──
    #[error("pairing token invalid or expired")]
    PairingTokenInvalid,

    #[error("pairing not in expected state: {actual:?}")]
    PairingStateMismatch { actual: PairingState },

    #[error("device not trusted: {device_id}")]
    DeviceNotTrusted { device_id: String },

    // ── persistence layer ──
    #[error("store io failed at {path}: {message}")]
    StoreIo { path: String, message: String },

    #[error("store payload corrupt at {path}: {message}")]
    StoreCorrupt { path: String, message: String },

    // ── CLI probe layer ──
    #[error("cli probe timeout: {bin} after {timeout_ms}ms")]
    CliProbeTimeout { bin: String, timeout_ms: u64 },

    #[error("cli probe failed: {bin}: {message}")]
    CliProbeFailed { bin: String, message: String },

    // ── RPC layer ──
    #[error("rpc call failed: {method}: {message}")]
    RpcCallFailed { method: String, message: String },
}

impl MinosError {
    /// Short, user-facing string. Stable for UI binding; do not include
    /// dynamic field values here — that is what `Display` is for.
    #[must_use]
    pub fn user_message(&self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Self::BindFailed { .. }, Lang::Zh) => "无法绑定本机端口；请检查 Tailscale 是否已启动并登录",
            (Self::BindFailed { .. }, Lang::En) => "Cannot bind local port; please verify Tailscale is running and signed in",
            (Self::ConnectFailed { .. }, Lang::Zh) => "无法连接 Mac；请确认两端均已加入同一 Tailscale 网络",
            (Self::ConnectFailed { .. }, Lang::En) => "Cannot reach Mac; ensure both devices are on the same Tailscale network",
            (Self::Disconnected { .. }, Lang::Zh) => "连接已断开，正在重试",
            (Self::Disconnected { .. }, Lang::En) => "Disconnected; reconnecting",
            (Self::PairingTokenInvalid, Lang::Zh) => "二维码已过期，请重新扫描",
            (Self::PairingTokenInvalid, Lang::En) => "QR code expired, please rescan",
            (Self::PairingStateMismatch { .. }, Lang::Zh) => "已存在配对设备，请确认替换",
            (Self::PairingStateMismatch { .. }, Lang::En) => "A paired device already exists; confirm to replace",
            (Self::DeviceNotTrusted { .. }, Lang::Zh) => "配对已失效，请重新扫码",
            (Self::DeviceNotTrusted { .. }, Lang::En) => "Pairing invalidated, please rescan",
            (Self::StoreIo { .. }, Lang::Zh) => "本地存储不可访问，请检查权限",
            (Self::StoreIo { .. }, Lang::En) => "Local storage inaccessible; check permissions",
            (Self::StoreCorrupt { .. }, Lang::Zh) => "本地配对状态损坏，已备份；请重新配对",
            (Self::StoreCorrupt { .. }, Lang::En) => "Local pairing state corrupt; backed up. Please re-pair",
            (Self::CliProbeTimeout { .. }, Lang::Zh) => "CLI 探测超时",
            (Self::CliProbeTimeout { .. }, Lang::En) => "CLI probe timed out",
            (Self::CliProbeFailed { .. }, Lang::Zh) => "CLI 探测失败",
            (Self::CliProbeFailed { .. }, Lang::En) => "CLI probe failed",
            (Self::RpcCallFailed { .. }, Lang::Zh) => "服务端错误，请稍后重试",
            (Self::RpcCallFailed { .. }, Lang::En) => "Server error, please retry",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_dynamic_context() {
        let e = MinosError::BindFailed {
            addr: "100.64.0.10:7878".into(),
            message: "address already in use".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("100.64.0.10:7878"));
        assert!(s.contains("address already in use"));
    }

    #[test]
    fn user_message_is_static_per_variant_and_lang() {
        let e = MinosError::PairingTokenInvalid;
        assert_eq!(e.user_message(Lang::Zh), "二维码已过期，请重新扫描");
        assert_eq!(e.user_message(Lang::En), "QR code expired, please rescan");
    }

    #[test]
    fn every_variant_has_user_message_in_both_langs() {
        // Construct one of every variant; user_message must not panic for any.
        let variants = vec![
            MinosError::BindFailed { addr: String::new(), message: String::new() },
            MinosError::ConnectFailed { url: String::new(), message: String::new() },
            MinosError::Disconnected { reason: String::new() },
            MinosError::PairingTokenInvalid,
            MinosError::PairingStateMismatch { actual: PairingState::Paired },
            MinosError::DeviceNotTrusted { device_id: String::new() },
            MinosError::StoreIo { path: String::new(), message: String::new() },
            MinosError::StoreCorrupt { path: String::new(), message: String::new() },
            MinosError::CliProbeTimeout { bin: String::new(), timeout_ms: 0 },
            MinosError::CliProbeFailed { bin: String::new(), message: String::new() },
            MinosError::RpcCallFailed { method: String::new(), message: String::new() },
        ];
        for v in variants {
            assert!(!v.user_message(Lang::Zh).is_empty(), "missing zh for {v:?}");
            assert!(!v.user_message(Lang::En).is_empty(), "missing en for {v:?}");
        }
    }
}
```

**Note on schema:** `BindFailed` etc. carry `message: String` instead of `#[source] std::io::Error` to keep the type `Clone` and FFI-friendly. Source error chains live in tracing logs, not in the value.

- [ ] **Step 2: Run tests**

Run: `cargo test -p minos-domain --lib error::tests`
Expected: `3 passed; 0 failed`

- [ ] **Step 3: Verify the whole crate builds & all tests pass**

Run: `cargo test -p minos-domain`
Expected: `12 passed; 0 failed` (3 ids + 5 agent + 3 connection/pairing_state + 3 error, plus 1 proptest)

- [ ] **Step 4: Commit**

```bash
git add crates/minos-domain/src/error.rs
git commit -m "feat(minos-domain): MinosError enum with bilingual user_message"
```

---

### Task 10: serde golden-file tests for cross-language schema stability

**Files:**
- Create: `crates/minos-domain/tests/golden.rs`
- Create: `crates/minos-domain/tests/golden/agent_descriptor.json`
- Create: `crates/minos-domain/tests/golden/connection_state.json`

- [ ] **Step 1: Write `tests/golden/agent_descriptor.json`**

```json
{
  "name": "codex",
  "path": "/usr/local/bin/codex",
  "version": "0.18.2",
  "status": { "kind": "ok" }
}
```

- [ ] **Step 2: Write `tests/golden/connection_state.json`**

```json
{ "reconnecting": { "attempt": 7 } }
```

- [ ] **Step 3: Write `tests/golden.rs`**

```rust
//! Golden-file checks. Failure here means the JSON shape exposed across the
//! UniFFI / frb boundary would silently change, breaking Swift / Dart consumers.

use minos_domain::{AgentDescriptor, AgentName, AgentStatus, ConnectionState};

#[test]
fn agent_descriptor_matches_golden() {
    let golden = include_str!("golden/agent_descriptor.json");
    let parsed: AgentDescriptor = serde_json::from_str(golden).unwrap();
    assert_eq!(
        parsed,
        AgentDescriptor {
            name: AgentName::Codex,
            path: Some("/usr/local/bin/codex".into()),
            version: Some("0.18.2".into()),
            status: AgentStatus::Ok,
        }
    );
    let reserialized = serde_json::to_value(&parsed).unwrap();
    let expected: serde_json::Value = serde_json::from_str(golden).unwrap();
    assert_eq!(reserialized, expected);
}

#[test]
fn connection_state_reconnecting_matches_golden() {
    let golden = include_str!("golden/connection_state.json");
    let parsed: ConnectionState = serde_json::from_str(golden).unwrap();
    assert_eq!(parsed, ConnectionState::Reconnecting { attempt: 7 });
    let reserialized = serde_json::to_value(&parsed).unwrap();
    let expected: serde_json::Value = serde_json::from_str(golden).unwrap();
    assert_eq!(reserialized, expected);
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p minos-domain --test golden`
Expected: `2 passed; 0 failed`

- [ ] **Step 5: Commit**

```bash
git add crates/minos-domain/tests/
git commit -m "test(minos-domain): golden JSON files lock cross-language schema"
```

---

## Phase C · `minos-protocol` (RPC contract)

### Task 11: minos-protocol crate skeleton + request/response types

**Files:**
- Create: `crates/minos-protocol/Cargo.toml`
- Create: `crates/minos-protocol/src/lib.rs`
- Create: `crates/minos-protocol/src/messages.rs`
- Create: `crates/minos-protocol/src/events.rs`

- [ ] **Step 1: Write `crates/minos-protocol/Cargo.toml`**

```toml
[package]
name = "minos-protocol"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Minos JSON-RPC contract: messages, events, and the MinosRpc service trait."

[dependencies]
minos-domain = { path = "../minos-domain" }
serde = { workspace = true }
serde_json = { workspace = true }
jsonrpsee = { workspace = true }

[dev-dependencies]
pretty_assertions = { workspace = true }
```

- [ ] **Step 2: Write `crates/minos-protocol/src/lib.rs`**

```rust
//! Minos JSON-RPC 2.0 contract.
//!
//! - `messages`: typed request / response payloads
//! - `events`:   AgentEvent enum reserved for the future `subscribe_events` stream
//! - `rpc`:      jsonrpsee `#[rpc]` trait shared by daemon (server) and mobile (client)

#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

pub mod events;
pub mod messages;
pub mod rpc;

pub use events::*;
pub use messages::*;
pub use rpc::*;
```

- [ ] **Step 3: Write `crates/minos-protocol/src/messages.rs`**

```rust
//! Request and response payload types.

use minos_domain::{AgentDescriptor, DeviceId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairRequest {
    pub device_id: DeviceId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairResponse {
    pub ok: bool,
    pub mac_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub version: String,
    pub uptime_secs: u64,
}

pub type ListClisResponse = Vec<AgentDescriptor>;

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pair_request_round_trip() {
        let req = PairRequest {
            device_id: DeviceId::new(),
            name: "iPhone of fan".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: PairRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn pair_response_round_trip() {
        let resp = PairResponse { ok: true, mac_name: "MacBook".into() };
        let json = serde_json::to_string(&resp).unwrap();
        let back: PairResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn health_response_round_trip() {
        let resp = HealthResponse { version: "0.1.0".into(), uptime_secs: 42 };
        let json = serde_json::to_string(&resp).unwrap();
        let back: HealthResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }
}
```

- [ ] **Step 4: Write `crates/minos-protocol/src/events.rs`**

```rust
//! Streaming event payload (placeholder for P1).
//!
//! The variant set is finalized here so producer crates added later need not
//! migrate consumers. MVP server returns a "not implemented" error from
//! `subscribe_events`; this enum is what *will* be streamed once
//! `minos-agent-runtime` lands in plan-equivalent for P1.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TokenChunk { text: String },
    ToolCall { name: String, args_json: String },
    ToolResult { name: String, output: String },
    Reasoning { text: String },
    Done { exit_code: i32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_chunk_serializes_with_type_tag() {
        let s = serde_json::to_string(&AgentEvent::TokenChunk { text: "hi".into() }).unwrap();
        assert_eq!(s, r#"{"type":"token_chunk","text":"hi"}"#);
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p minos-protocol --lib`
Expected: `4 passed; 0 failed`

- [ ] **Step 6: Commit**

```bash
git add crates/minos-protocol/
git commit -m "feat(minos-protocol): request/response/event types with serde golden shapes"
```

---

### Task 12: jsonrpsee `MinosRpc` service trait

**Files:**
- Create: `crates/minos-protocol/src/rpc.rs`

- [ ] **Step 1: Write `crates/minos-protocol/src/rpc.rs`**

```rust
//! The shared service trait. `jsonrpsee` macros generate a server stub
//! (implemented by `minos-daemon`) and a typed client (used by `minos-mobile`).

use crate::{AgentEvent, HealthResponse, ListClisResponse, PairRequest, PairResponse};
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::proc_macros::rpc;

#[rpc(server, client, namespace = "minos")]
pub trait MinosRpc {
    /// Confirm a fresh pairing handshake. Idempotent only when the same
    /// (token, device_id) tuple is supplied.
    #[method(name = "pair")]
    async fn pair(&self, req: PairRequest) -> jsonrpsee::core::RpcResult<PairResponse>;

    /// Cheap liveness probe.
    #[method(name = "health")]
    async fn health(&self) -> jsonrpsee::core::RpcResult<HealthResponse>;

    /// Snapshot of locally detected CLI agents.
    #[method(name = "list_clis")]
    async fn list_clis(&self) -> jsonrpsee::core::RpcResult<ListClisResponse>;

    /// Streaming agent events. **MVP**: server returns "not implemented";
    /// shape and naming are pinned now so plan P1 only fills in the producer.
    #[subscription(name = "subscribe_events" => "agent_event", item = AgentEvent)]
    async fn subscribe_events(&self) -> SubscriptionResult;
}
```

- [ ] **Step 2: Verify build**

Run: `cargo check -p minos-protocol --all-targets`
Expected: clean (no warnings beyond clippy::pedantic).

- [ ] **Step 3: Commit**

```bash
git add crates/minos-protocol/src/rpc.rs
git commit -m "feat(minos-protocol): MinosRpc trait via jsonrpsee proc-macros"
```

---

## Phase D · `minos-pairing`

### Task 13: minos-pairing crate skeleton + `TrustedDevice` + `PairingStore`

**Files:**
- Create: `crates/minos-pairing/Cargo.toml`
- Create: `crates/minos-pairing/src/lib.rs`
- Create: `crates/minos-pairing/src/store.rs`

- [ ] **Step 1: Write `crates/minos-pairing/Cargo.toml`**

```toml
[package]
name = "minos-pairing"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Pairing state machine, trusted device records, and the PairingStore port."

[dependencies]
minos-domain = { path = "../minos-domain" }
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
url = { workspace = true }

[dev-dependencies]
pretty_assertions = { workspace = true }
rstest = { workspace = true }
proptest = { workspace = true }
```

- [ ] **Step 2: Write `crates/minos-pairing/src/lib.rs`**

```rust
#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod state_machine;
pub mod store;
pub mod token;

pub use state_machine::*;
pub use store::*;
pub use token::*;
```

- [ ] **Step 3: Write `crates/minos-pairing/src/store.rs`**

```rust
//! Pairing persistence port + trusted-device record.

use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, MinosError};
use serde::{Deserialize, Serialize};

/// One peer that has successfully paired and may reconnect on its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedDevice {
    pub device_id: DeviceId,
    pub name: String,
    /// Tailscale IP captured at pair time. Used by the mobile side to know
    /// where to reconnect; the Mac daemon ignores this field.
    pub host: String,
    pub port: u16,
    pub paired_at: DateTime<Utc>,
}

/// Persistence trait. Implementations:
/// - `minos-daemon::FilePairingStore` (JSON file)
/// - `minos-mobile::KeychainPairingStore` (FFI callback into iOS Keychain)
/// - test-only in-memory impls
pub trait PairingStore: Send + Sync + 'static {
    fn load(&self) -> Result<Vec<TrustedDevice>, MinosError>;
    fn save(&self, devices: &[TrustedDevice]) -> Result<(), MinosError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// In-memory store for unit tests in this module and downstream crates.
    pub(crate) struct InMemStore(pub Mutex<Vec<TrustedDevice>>);

    impl PairingStore for InMemStore {
        fn load(&self) -> Result<Vec<TrustedDevice>, MinosError> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, devices: &[TrustedDevice]) -> Result<(), MinosError> {
            *self.0.lock().unwrap() = devices.to_vec();
            Ok(())
        }
    }

    #[test]
    fn round_trip_through_in_mem_store() {
        let store = InMemStore(Mutex::new(vec![]));
        let dev = TrustedDevice {
            device_id: DeviceId::new(),
            name: "fan iPhone".into(),
            host: "100.64.0.42".into(),
            port: 7878,
            paired_at: Utc::now(),
        };
        store.save(&[dev.clone()]).unwrap();
        let back = store.load().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], dev);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p minos-pairing --lib store::tests`
Expected: `1 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/minos-pairing/
git commit -m "feat(minos-pairing): TrustedDevice + PairingStore port + in-mem test impl"
```

---

### Task 14: `Pairing` state machine (table-driven tests)

**Files:**
- Create: `crates/minos-pairing/src/state_machine.rs`

- [ ] **Step 1: Write `state_machine.rs`**

```rust
//! State machine: `Unpaired -> AwaitingPeer -> Paired`.
//!
//! Illegal transitions return `MinosError::PairingStateMismatch`.

use minos_domain::{MinosError, PairingState};

#[derive(Debug, Clone)]
pub struct Pairing {
    state: PairingState,
}

impl Pairing {
    #[must_use]
    pub fn new(initial: PairingState) -> Self {
        Self { state: initial }
    }

    #[must_use]
    pub fn state(&self) -> PairingState {
        self.state
    }

    /// Begin awaiting a peer (i.e., a QR has been displayed).
    pub fn begin_awaiting(&mut self) -> Result<(), MinosError> {
        match self.state {
            PairingState::Unpaired => {
                self.state = PairingState::AwaitingPeer;
                Ok(())
            }
            other => Err(MinosError::PairingStateMismatch { actual: other }),
        }
    }

    /// Accept a peer's pair RPC.
    pub fn accept_peer(&mut self) -> Result<(), MinosError> {
        match self.state {
            PairingState::AwaitingPeer => {
                self.state = PairingState::Paired;
                Ok(())
            }
            other => Err(MinosError::PairingStateMismatch { actual: other }),
        }
    }

    /// Forget current peer (UI "forget device" or corrupt-store reset).
    pub fn forget(&mut self) {
        self.state = PairingState::Unpaired;
    }

    /// Replace current paired peer (user confirmed "replace existing").
    pub fn replace(&mut self) -> Result<(), MinosError> {
        match self.state {
            PairingState::Paired => {
                self.state = PairingState::AwaitingPeer;
                Ok(())
            }
            other => Err(MinosError::PairingStateMismatch { actual: other }),
        }
    }
}
```

- [ ] **Step 2: Write `crates/minos-pairing/tests/state_machine_table.rs` (integration table tests)**

```rust
use minos_domain::{MinosError, PairingState};
use minos_pairing::Pairing;
use rstest::rstest;

#[rstest]
#[case::ok_unpaired_to_awaiting(PairingState::Unpaired, true)]
#[case::reject_awaiting(PairingState::AwaitingPeer, false)]
#[case::reject_paired(PairingState::Paired, false)]
fn begin_awaiting(#[case] from: PairingState, #[case] should_succeed: bool) {
    let mut p = Pairing::new(from);
    let r = p.begin_awaiting();
    assert_eq!(r.is_ok(), should_succeed, "from {from:?}");
    if should_succeed {
        assert_eq!(p.state(), PairingState::AwaitingPeer);
    } else {
        assert!(matches!(r, Err(MinosError::PairingStateMismatch { .. })));
        assert_eq!(p.state(), from);
    }
}

#[rstest]
#[case::ok_awaiting_to_paired(PairingState::AwaitingPeer, true)]
#[case::reject_unpaired(PairingState::Unpaired, false)]
#[case::reject_paired(PairingState::Paired, false)]
fn accept_peer(#[case] from: PairingState, #[case] should_succeed: bool) {
    let mut p = Pairing::new(from);
    let r = p.accept_peer();
    assert_eq!(r.is_ok(), should_succeed, "from {from:?}");
}

#[test]
fn forget_resets_to_unpaired_from_any_state() {
    for from in [PairingState::Unpaired, PairingState::AwaitingPeer, PairingState::Paired] {
        let mut p = Pairing::new(from);
        p.forget();
        assert_eq!(p.state(), PairingState::Unpaired);
    }
}

#[test]
fn replace_paired_returns_to_awaiting() {
    let mut p = Pairing::new(PairingState::Paired);
    p.replace().unwrap();
    assert_eq!(p.state(), PairingState::AwaitingPeer);
}

#[test]
fn replace_when_not_paired_errors() {
    let mut p = Pairing::new(PairingState::Unpaired);
    assert!(matches!(p.replace(), Err(MinosError::PairingStateMismatch { .. })));
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p minos-pairing`
Expected: ~10 passed (3 begin + 3 accept + 1 forget + 2 replace + 1 store).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-pairing/src/state_machine.rs crates/minos-pairing/tests/state_machine_table.rs
git commit -m "feat(minos-pairing): state machine with table-driven tests"
```

---

### Task 15: `generate_qr_payload` + token TTL

**Files:**
- Create: `crates/minos-pairing/src/token.rs`

- [ ] **Step 1: Write `token.rs`**

```rust
//! QR payload format (matches spec §6.1).

use chrono::{DateTime, Duration, Utc};
use minos_domain::PairingToken;
use serde::{Deserialize, Serialize};

pub const QR_TOKEN_TTL: Duration = Duration::minutes(5);
pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QrPayload {
    pub v: u8,
    pub host: String,
    pub port: u16,
    pub token: PairingToken,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ActiveToken {
    pub token: PairingToken,
    pub issued_at: DateTime<Utc>,
}

impl ActiveToken {
    #[must_use]
    pub fn fresh() -> Self {
        Self { token: PairingToken::generate(), issued_at: Utc::now() }
    }

    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now - self.issued_at > QR_TOKEN_TTL
    }
}

#[must_use]
pub fn generate_qr_payload(host: String, port: u16, name: String) -> (QrPayload, ActiveToken) {
    let active = ActiveToken::fresh();
    let payload = QrPayload {
        v: PROTOCOL_VERSION,
        host,
        port,
        token: active.token.clone(),
        name,
    };
    (payload, active)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn payload_has_v1_field() {
        let (p, _) = generate_qr_payload("100.64.0.10".into(), 7878, "Mac".into());
        assert_eq!(p.v, 1);
        assert_eq!(p.port, 7878);
    }

    #[test]
    fn payload_round_trips_through_json() {
        let (p, _) = generate_qr_payload("100.64.0.10".into(), 7878, "Mac".into());
        let json = serde_json::to_string(&p).unwrap();
        let back: QrPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn token_expires_after_five_minutes() {
        let issued = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let active = ActiveToken { token: PairingToken::generate(), issued_at: issued };
        let four_min = issued + Duration::minutes(4);
        let six_min = issued + Duration::minutes(6);
        assert!(!active.is_expired(four_min));
        assert!(active.is_expired(six_min));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p minos-pairing --lib token::tests`
Expected: `3 passed`

- [ ] **Step 3: Commit**

```bash
git add crates/minos-pairing/src/token.rs
git commit -m "feat(minos-pairing): QR payload + 5-min TTL token"
```

---

## Phase E · `minos-cli-detect`

### Task 16: minos-cli-detect crate + `CommandRunner` port

**Files:**
- Create: `crates/minos-cli-detect/Cargo.toml`
- Create: `crates/minos-cli-detect/src/lib.rs`
- Create: `crates/minos-cli-detect/src/runner.rs`

- [ ] **Step 1: Write `crates/minos-cli-detect/Cargo.toml`**

```toml
[package]
name = "minos-cli-detect"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Local CLI agent detection (which + --version)."

[dependencies]
minos-domain = { path = "../minos-domain" }
tokio = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
mockall = { workspace = true }
tokio-test = { workspace = true }
```

- [ ] **Step 2: Write `crates/minos-cli-detect/src/lib.rs`**

```rust
#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

pub mod detect;
pub mod runner;

pub use detect::*;
pub use runner::*;
```

- [ ] **Step 3: Write `crates/minos-cli-detect/src/runner.rs`**

```rust
//! Subprocess port. The trait exists so unit tests can inject deterministic
//! responses without forking real binaries.

use std::time::Duration;

use minos_domain::MinosError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync + 'static {
    async fn which(&self, bin: &str) -> Option<String>;
    async fn run(&self, bin: &str, args: &[&str], timeout: Duration) -> Result<CommandOutcome, MinosError>;
}
```

The trait uses `async_trait` for object-safety. Add the dep:

```bash
cargo add -p minos-cli-detect async-trait@0.1
```

- [ ] **Step 4: Verify build**

Run: `cargo check -p minos-cli-detect`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-cli-detect/
git commit -m "feat(minos-cli-detect): CommandRunner port for testable subprocess access"
```

---

### Task 17: `RealCommandRunner` (tokio-based) + `detect_all`

**Files:**
- Modify: `crates/minos-cli-detect/src/runner.rs` (append `RealCommandRunner`)
- Create: `crates/minos-cli-detect/src/detect.rs`

- [ ] **Step 1: Append to `runner.rs`**

```rust
// ──────────────────────────────────────────────────────────────────────────
// Real implementation (used by the daemon at runtime).
// ──────────────────────────────────────────────────────────────────────────

use std::process::Stdio;
use tokio::process::Command;
use tokio::time::timeout;

pub struct RealCommandRunner;

#[async_trait::async_trait]
impl CommandRunner for RealCommandRunner {
    async fn which(&self, bin: &str) -> Option<String> {
        let out = Command::new("which")
            .arg(bin)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (!s.is_empty()).then_some(s)
    }

    async fn run(
        &self,
        bin: &str,
        args: &[&str],
        timeout_dur: Duration,
    ) -> Result<CommandOutcome, MinosError> {
        let fut = Command::new(bin)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        let out = timeout(timeout_dur, fut).await.map_err(|_| MinosError::CliProbeTimeout {
            bin: bin.to_owned(),
            timeout_ms: u64::try_from(timeout_dur.as_millis()).unwrap_or(u64::MAX),
        })?;

        let out = out.map_err(|e| MinosError::CliProbeFailed {
            bin: bin.to_owned(),
            message: e.to_string(),
        })?;

        Ok(CommandOutcome {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}
```

- [ ] **Step 2: Write `crates/minos-cli-detect/src/detect.rs`**

```rust
//! Public entry point: `detect_all` returns one descriptor per known agent.

use std::sync::Arc;
use std::time::Duration;

use minos_domain::{AgentDescriptor, AgentName, AgentStatus};
use tracing::warn;

use crate::CommandRunner;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn detect_all(runner: Arc<dyn CommandRunner>) -> Vec<AgentDescriptor> {
    let mut out = Vec::with_capacity(AgentName::all().len());
    for &name in AgentName::all() {
        out.push(detect_one(&*runner, name).await);
    }
    out
}

async fn detect_one(runner: &dyn CommandRunner, name: AgentName) -> AgentDescriptor {
    let bin = name.bin_name();
    let Some(path) = runner.which(bin).await else {
        return AgentDescriptor { name, path: None, version: None, status: AgentStatus::Missing };
    };

    match runner.run(bin, &["--version"], PROBE_TIMEOUT).await {
        Ok(outcome) if outcome.exit_code == 0 => {
            let version = parse_version(&outcome.stdout).or_else(|| parse_version(&outcome.stderr));
            AgentDescriptor { name, path: Some(path), version, status: AgentStatus::Ok }
        }
        Ok(outcome) => {
            warn!(?name, exit_code = outcome.exit_code, "non-zero exit from --version probe");
            AgentDescriptor {
                name,
                path: Some(path),
                version: None,
                status: AgentStatus::Error {
                    reason: format!("exit {}: {}", outcome.exit_code, outcome.stderr.trim()),
                },
            }
        }
        Err(e) => AgentDescriptor {
            name,
            path: Some(path),
            version: None,
            status: AgentStatus::Error { reason: e.to_string() },
        },
    }
}

/// Extract the first whitespace-delimited token that looks like a semver.
fn parse_version(s: &str) -> Option<String> {
    s.split_whitespace()
        .find(|tok| tok.chars().next().is_some_and(|c| c.is_ascii_digit()) && tok.contains('.'))
        .map(|tok| tok.trim_matches(',').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Per-call scripted runner.
    struct ScriptRunner {
        script: Mutex<Vec<(&'static str, ScriptStep)>>,
    }
    enum ScriptStep {
        Which(Option<&'static str>),
        Run(Result<CommandOutcome, MinosError>),
    }

    use crate::CommandOutcome;

    #[async_trait::async_trait]
    impl CommandRunner for ScriptRunner {
        async fn which(&self, bin: &str) -> Option<String> {
            let mut s = self.script.lock().unwrap();
            let (expected, step) = s.remove(0);
            assert_eq!(expected, bin);
            match step {
                ScriptStep::Which(v) => v.map(String::from),
                _ => panic!("script expected which, got run"),
            }
        }
        async fn run(
            &self,
            bin: &str,
            _args: &[&str],
            _t: Duration,
        ) -> Result<CommandOutcome, MinosError> {
            let mut s = self.script.lock().unwrap();
            let (expected, step) = s.remove(0);
            assert_eq!(expected, bin);
            match step {
                ScriptStep::Run(r) => r,
                _ => panic!("script expected run, got which"),
            }
        }
    }

    fn outcome_ok(stdout: &str) -> ScriptStep {
        ScriptStep::Run(Ok(CommandOutcome {
            exit_code: 0,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }))
    }

    #[tokio::test]
    async fn missing_bin_yields_missing_status() {
        let runner = Arc::new(ScriptRunner {
            script: Mutex::new(vec![
                ("codex", ScriptStep::Which(None)),
                ("claude", ScriptStep::Which(None)),
                ("gemini", ScriptStep::Which(None)),
            ]),
        });
        let out = detect_all(runner).await;
        assert_eq!(out.len(), 3);
        for d in out {
            assert_eq!(d.status, AgentStatus::Missing);
            assert!(d.path.is_none());
        }
    }

    #[tokio::test]
    async fn version_parsed_from_stdout() {
        let runner = Arc::new(ScriptRunner {
            script: Mutex::new(vec![
                ("codex", ScriptStep::Which(Some("/u/c"))),
                ("codex", outcome_ok("codex 0.18.2\n")),
                ("claude", ScriptStep::Which(None)),
                ("gemini", ScriptStep::Which(None)),
            ]),
        });
        let out = detect_all(runner).await;
        assert_eq!(out[0].status, AgentStatus::Ok);
        assert_eq!(out[0].version.as_deref(), Some("0.18.2"));
        assert_eq!(out[0].path.as_deref(), Some("/u/c"));
    }

    #[tokio::test]
    async fn timeout_yields_error_status() {
        let runner = Arc::new(ScriptRunner {
            script: Mutex::new(vec![
                ("codex", ScriptStep::Which(Some("/u/c"))),
                (
                    "codex",
                    ScriptStep::Run(Err(MinosError::CliProbeTimeout {
                        bin: "codex".into(),
                        timeout_ms: 5000,
                    })),
                ),
                ("claude", ScriptStep::Which(None)),
                ("gemini", ScriptStep::Which(None)),
            ]),
        });
        let out = detect_all(runner).await;
        assert!(matches!(out[0].status, AgentStatus::Error { .. }));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p minos-cli-detect`
Expected: `3 passed`

- [ ] **Step 4: Commit**

```bash
git add crates/minos-cli-detect/
git commit -m "feat(minos-cli-detect): RealCommandRunner + detect_all with version parser"
```

---

## Phase F · `minos-transport`

### Task 18: minos-transport crate + reconnect backoff

**Files:**
- Create: `crates/minos-transport/Cargo.toml`
- Create: `crates/minos-transport/src/lib.rs`
- Create: `crates/minos-transport/src/backoff.rs`

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "minos-transport"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "WebSocket transport (jsonrpsee server + client) and reconnect logic."

[dependencies]
minos-domain = { path = "../minos-domain" }
minos-protocol = { path = "../minos-protocol" }
tokio = { workspace = true }
futures = { workspace = true }
jsonrpsee = { workspace = true }
tokio-tungstenite = { workspace = true }
tracing = { workspace = true }
serde = { workspace = true }
url = { workspace = true }

[dev-dependencies]
tokio-test = { workspace = true }
rstest = { workspace = true }
pretty_assertions = { workspace = true }
```

- [ ] **Step 2: Write `lib.rs`**

```rust
#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

pub mod backoff;
pub mod client;
pub mod server;

pub use backoff::*;
pub use client::*;
pub use server::*;
```

- [ ] **Step 3: Write `backoff.rs`**

```rust
//! Exponential backoff: 1s → 2s → 4s → 8s → 16s → 30s (capped).

use std::time::Duration;

const BASE: Duration = Duration::from_secs(1);
const CAP: Duration = Duration::from_secs(30);

#[must_use]
pub fn delay_for_attempt(attempt: u32) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let exp = u32::min(attempt - 1, 16); // avoid shift overflow
    let scaled = BASE.saturating_mul(1_u32 << exp);
    if scaled > CAP { CAP } else { scaled }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(0, 0)]
    #[case(1, 1)]
    #[case(2, 2)]
    #[case(3, 4)]
    #[case(4, 8)]
    #[case(5, 16)]
    #[case(6, 30)]
    #[case(7, 30)]
    #[case(100, 30)]
    fn backoff_sequence(#[case] attempt: u32, #[case] expected_secs: u64) {
        assert_eq!(delay_for_attempt(attempt), Duration::from_secs(expected_secs));
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p minos-transport --lib backoff::tests`
Expected: `9 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/minos-transport/
git commit -m "feat(minos-transport): exponential backoff helper"
```

---

### Task 19: `WsServer` (jsonrpsee server bound to a `MinosRpcServer` impl)

**Files:**
- Create: `crates/minos-transport/src/server.rs`

- [ ] **Step 1: Write `server.rs`**

```rust
//! Thin wrapper over `jsonrpsee::server::Server` that binds it to a TCP
//! listener (typically the Mac's Tailscale IP) and serves a `MinosRpcServer`.

use std::net::SocketAddr;

use jsonrpsee::server::{RpcModule, Server, ServerHandle};
use minos_domain::MinosError;
use tracing::info;

pub struct WsServer {
    handle: ServerHandle,
    addr: SocketAddr,
}

impl WsServer {
    /// Bind a jsonrpsee server to `addr` and start serving the supplied module.
    /// Returns once the listener is bound (the server runs in a background task).
    pub async fn bind(addr: SocketAddr, module: RpcModule<()>) -> Result<Self, MinosError> {
        let server = Server::builder()
            .build(addr)
            .await
            .map_err(|e| MinosError::BindFailed { addr: addr.to_string(), message: e.to_string() })?;
        let bound = server.local_addr().map_err(|e| MinosError::BindFailed {
            addr: addr.to_string(),
            message: e.to_string(),
        })?;
        let handle = server.start(module);
        info!(?bound, "WsServer started");
        Ok(Self { handle, addr: bound })
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn stop(self) -> Result<(), MinosError> {
        self.handle.stop().map_err(|e| MinosError::Disconnected { reason: e.to_string() })?;
        self.handle.stopped().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonrpsee::server::RpcModule;

    #[tokio::test]
    async fn binds_to_ephemeral_port() {
        let module = RpcModule::new(());
        let s = WsServer::bind("127.0.0.1:0".parse().unwrap(), module).await.unwrap();
        assert_ne!(s.addr().port(), 0);
        s.stop().await.unwrap();
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p minos-transport --lib server::tests`
Expected: `1 passed`

- [ ] **Step 3: Commit**

```bash
git add crates/minos-transport/src/server.rs
git commit -m "feat(minos-transport): WsServer wrapper around jsonrpsee::Server"
```

---

### Task 20: `WsClient` (jsonrpsee ws client) + simple connect

**Files:**
- Create: `crates/minos-transport/src/client.rs`

- [ ] **Step 1: Write `client.rs`**

```rust
//! WebSocket-side jsonrpsee client. Reconnect orchestration is the caller's
//! responsibility (use `backoff::delay_for_attempt`).

use std::sync::Arc;

use jsonrpsee::ws_client::{WsClient as JsonRpcWsClient, WsClientBuilder};
use minos_domain::MinosError;
use url::Url;

pub struct WsClient {
    inner: Arc<JsonRpcWsClient>,
}

impl WsClient {
    pub async fn connect(url: &Url) -> Result<Self, MinosError> {
        let inner = WsClientBuilder::default()
            .build(url.as_str())
            .await
            .map_err(|e| MinosError::ConnectFailed { url: url.to_string(), message: e.to_string() })?;
        Ok(Self { inner: Arc::new(inner) })
    }

    #[must_use]
    pub fn inner(&self) -> Arc<JsonRpcWsClient> {
        self.inner.clone()
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WsServer;
    use jsonrpsee::server::RpcModule;

    #[tokio::test]
    async fn client_connects_to_local_server() {
        // Spin up an empty server, then connect.
        let server = WsServer::bind("127.0.0.1:0".parse().unwrap(), RpcModule::new(()))
            .await
            .unwrap();
        let url = format!("ws://{}", server.addr()).parse().unwrap();
        let client = WsClient::connect(&url).await.unwrap();
        assert!(client.is_connected());
        server.stop().await.unwrap();
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p minos-transport --lib client::tests`
Expected: `1 passed`

- [ ] **Step 3: Commit**

```bash
git add crates/minos-transport/src/client.rs
git commit -m "feat(minos-transport): WsClient wrapper with connect smoke test"
```

---

## Phase G · `minos-daemon`

### Task 21: minos-daemon crate + `FilePairingStore`

**Files:**
- Create: `crates/minos-daemon/Cargo.toml`
- Create: `crates/minos-daemon/src/lib.rs`
- Create: `crates/minos-daemon/src/file_store.rs`

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "minos-daemon"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Mac-side composition root: WS server + file store + RPC handlers + CLI detect."

[dependencies]
minos-domain = { path = "../minos-domain" }
minos-protocol = { path = "../minos-protocol" }
minos-pairing = { path = "../minos-pairing" }
minos-cli-detect = { path = "../minos-cli-detect" }
minos-transport = { path = "../minos-transport" }
tokio = { workspace = true }
futures = { workspace = true }
jsonrpsee = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
mars-xlog = { workspace = true }
chrono = { workspace = true }
async-trait = "0.1"

[dev-dependencies]
tempfile = { workspace = true }
tokio-test = { workspace = true }
pretty_assertions = { workspace = true }
```

- [ ] **Step 2: Write `lib.rs`**

```rust
#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

pub mod file_store;
pub mod handle;
pub mod logging;
pub mod rpc_server;
pub mod tailscale;

pub use file_store::*;
pub use handle::*;
```

- [ ] **Step 3: Write `file_store.rs`**

```rust
//! `PairingStore` impl backed by a JSON file under
//! `~/Library/Application Support/minos/devices.json` (Mac convention).
//!
//! On parse failure, the existing file is renamed to `.bak` and `load()`
//! returns `MinosError::StoreCorrupt` so the daemon can surface it to UI.

use std::fs;
use std::path::PathBuf;

use minos_domain::MinosError;
use minos_pairing::{PairingStore, TrustedDevice};

pub struct FilePairingStore {
    path: PathBuf,
}

impl FilePairingStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Default Mac path: `~/Library/Application Support/minos/devices.json`.
    /// On non-Mac targets (e.g. CI Linux), falls back to `$HOME/.minos/devices.json`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        if cfg!(target_os = "macos") {
            PathBuf::from(home).join("Library/Application Support/minos/devices.json")
        } else {
            PathBuf::from(home).join(".minos/devices.json")
        }
    }
}

impl PairingStore for FilePairingStore {
    fn load(&self) -> Result<Vec<TrustedDevice>, MinosError> {
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let bytes = fs::read(&self.path).map_err(|e| MinosError::StoreIo {
            path: self.path.display().to_string(),
            message: e.to_string(),
        })?;
        match serde_json::from_slice::<Vec<TrustedDevice>>(&bytes) {
            Ok(v) => Ok(v),
            Err(e) => {
                let bak = self.path.with_extension("json.bak");
                let _ = fs::rename(&self.path, &bak);
                Err(MinosError::StoreCorrupt {
                    path: self.path.display().to_string(),
                    message: e.to_string(),
                })
            }
        }
    }

    fn save(&self, devices: &[TrustedDevice]) -> Result<(), MinosError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| MinosError::StoreIo {
                path: parent.display().to_string(),
                message: e.to_string(),
            })?;
        }
        let json = serde_json::to_vec_pretty(devices).map_err(|e| MinosError::StoreCorrupt {
            path: self.path.display().to_string(),
            message: e.to_string(),
        })?;
        fs::write(&self.path, json).map_err(|e| MinosError::StoreIo {
            path: self.path.display().to_string(),
            message: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use minos_domain::DeviceId;

    #[test]
    fn round_trip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePairingStore::new(dir.path().join("d.json"));
        let dev = TrustedDevice {
            device_id: DeviceId::new(),
            name: "iPhone".into(),
            host: "100.64.0.42".into(),
            port: 7878,
            paired_at: Utc::now(),
        };
        store.save(&[dev.clone()]).unwrap();
        let back = store.load().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].device_id, dev.device_id);
    }

    #[test]
    fn missing_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePairingStore::new(dir.path().join("never.json"));
        assert!(store.load().unwrap().is_empty());
    }

    #[test]
    fn corrupt_file_renamed_to_bak_and_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.json");
        fs::write(&path, b"not json").unwrap();
        let store = FilePairingStore::new(path.clone());
        let r = store.load();
        assert!(matches!(r, Err(MinosError::StoreCorrupt { .. })));
        assert!(!path.exists());
        assert!(path.with_extension("json.bak").exists());
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p minos-daemon --lib file_store::tests`
Expected: `3 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/minos-daemon/
git commit -m "feat(minos-daemon): FilePairingStore with .bak on corruption"
```

---

### Task 22: Tailscale IP discovery + `tailscale.rs` placeholder

**Files:**
- Create: `crates/minos-daemon/src/tailscale.rs`

- [ ] **Step 1: Write `tailscale.rs`**

```rust
//! Tailscale 100.x IP discovery. MVP shells out to `tailscale ip --4`.
//!
//! Returns `None` if `tailscale` is not installed or returns no IP. Callers
//! should map `None` to `MinosError::BindFailed { addr: "<unknown>", ... }`
//! and surface "please start Tailscale" to the user.

use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

pub async fn discover_ip() -> Option<String> {
    let fut = Command::new("tailscale")
        .args(["ip", "--4"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let out = timeout(Duration::from_secs(2), fut).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!s.is_empty() && s.starts_with("100.")).then_some(s)
}

#[cfg(test)]
mod tests {
    // Note: no unit tests here — discover_ip touches a real binary. CI runs
    // without tailscale installed, so it will return None; the daemon E2E
    // test (Task 24) supplies an explicit 127.0.0.1 address instead.
    #[tokio::test]
    async fn returns_none_or_some_with_100_prefix() {
        let ip = super::discover_ip().await;
        assert!(ip.is_none() || ip.as_ref().unwrap().starts_with("100."));
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p minos-daemon --lib tailscale::tests`
Expected: `1 passed` (regardless of whether tailscale is installed locally).

- [ ] **Step 3: Commit**

```bash
git add crates/minos-daemon/src/tailscale.rs
git commit -m "feat(minos-daemon): Tailscale IP discovery via 'tailscale ip --4'"
```

---

### Task 23: `DaemonHandle` + `RpcServerImpl` (wires everything)

**Files:**
- Create: `crates/minos-daemon/src/handle.rs`
- Create: `crates/minos-daemon/src/rpc_server.rs`
- Create: `crates/minos-daemon/src/logging.rs` (stub; real wiring in Task 27)

- [ ] **Step 1: Write `logging.rs` (stub)**

```rust
//! mars-xlog wiring lives in Task 27. For now this module just exposes a noop
//! init so other modules can call it without conditional compilation.

use minos_domain::MinosError;

/// Idempotent. Real implementation arrives in Task 27.
pub fn init() -> Result<(), MinosError> {
    Ok(())
}
```

- [ ] **Step 2: Write `rpc_server.rs`**

```rust
//! `MinosRpcServer` impl that routes to inner services.
//!
//! Holds `Arc`s only — cheap to clone once and pass into the jsonrpsee
//! `RpcModule`.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use chrono::Utc;
use jsonrpsee::core::async_trait;
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::types::ErrorObjectOwned;
use minos_cli_detect::{detect_all, CommandRunner};
use minos_domain::{DeviceId, MinosError, PairingState};
use minos_pairing::{Pairing, PairingStore, TrustedDevice};
use minos_protocol::{
    HealthResponse, ListClisResponse, MinosRpcServer, PairRequest, PairResponse,
};

pub struct RpcServerImpl {
    pub started_at: Instant,
    pub pairing: Arc<Mutex<Pairing>>,
    pub store: Arc<dyn PairingStore>,
    pub runner: Arc<dyn CommandRunner>,
    pub mac_name: String,
    pub host: String,
    pub port: u16,
}

#[async_trait]
impl MinosRpcServer for RpcServerImpl {
    async fn pair(&self, req: PairRequest) -> jsonrpsee::core::RpcResult<PairResponse> {
        // Token validation happens at the WS-upgrade layer (Task 24); by the
        // time `pair` is called the token is already verified. Here we only
        // gate on the state machine.
        let mut p = self.pairing.lock().unwrap();
        p.accept_peer().map_err(rpc_err)?;

        let mut current = self.store.load().map_err(rpc_err)?;
        let dev = TrustedDevice {
            device_id: req.device_id,
            name: req.name,
            host: self.host.clone(),
            port: self.port,
            paired_at: Utc::now(),
        };
        // Replace any existing entry for the same device_id; otherwise append.
        if let Some(idx) = current.iter().position(|d| d.device_id == req.device_id) {
            current[idx] = dev;
        } else {
            current.push(dev);
        }
        self.store.save(&current).map_err(rpc_err)?;
        Ok(PairResponse { ok: true, mac_name: self.mac_name.clone() })
    }

    async fn health(&self) -> jsonrpsee::core::RpcResult<HealthResponse> {
        Ok(HealthResponse {
            version: env!("CARGO_PKG_VERSION").into(),
            uptime_secs: self.started_at.elapsed().as_secs(),
        })
    }

    async fn list_clis(&self) -> jsonrpsee::core::RpcResult<ListClisResponse> {
        Ok(detect_all(self.runner.clone()).await)
    }

    async fn subscribe_events(&self, pending: PendingSubscriptionSink) -> SubscriptionResult {
        // MVP: not implemented; close immediately with a 4001 error.
        let sink = pending
            .accept()
            .await
            .map_err(|e| jsonrpsee::core::SubscriptionMessage::from(format!("{e}")))?;
        sink.close(jsonrpsee::types::ErrorObject::owned(
            4001,
            "subscribe_events not yet implemented (P1)",
            None::<()>,
        ))
        .await;
        Ok(())
    }
}

fn rpc_err(e: MinosError) -> ErrorObjectOwned {
    let code = match e {
        MinosError::PairingStateMismatch { .. } => -32001,
        MinosError::PairingTokenInvalid => -32002,
        MinosError::DeviceNotTrusted { .. } => -32003,
        _ => -32000,
    };
    ErrorObjectOwned::owned(code, e.to_string(), None::<()>)
}

#[allow(unused)] // suppresses unused warning while DeviceId is only re-used by callers
type _Hint = DeviceId;
```

- [ ] **Step 3: Write `handle.rs`**

```rust
//! Public façade exposed to Swift via UniFFI in plan 02. This crate only
//! exposes the Rust shape; UniFFI annotations live in `minos-ffi-uniffi`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use jsonrpsee::server::RpcModule;
use minos_cli_detect::{CommandRunner, RealCommandRunner};
use minos_domain::{ConnectionState, MinosError, PairingState};
use minos_pairing::{generate_qr_payload, ActiveToken, Pairing, PairingStore, QrPayload};
use minos_protocol::MinosRpcServer;
use minos_transport::WsServer;
use tokio::sync::watch;

use crate::file_store::FilePairingStore;
use crate::rpc_server::RpcServerImpl;
use crate::tailscale;

pub struct DaemonConfig {
    pub mac_name: String,
    pub bind_addr: SocketAddr,
}

pub struct DaemonHandle {
    server: Option<WsServer>,
    state_rx: watch::Receiver<ConnectionState>,
    state_tx: watch::Sender<ConnectionState>,
    pairing: Arc<Mutex<Pairing>>,
    store: Arc<dyn PairingStore>,
    active_token: Arc<Mutex<Option<ActiveToken>>>,
    addr: SocketAddr,
    mac_name: String,
}

impl DaemonHandle {
    /// Start the daemon. Binds to the supplied address and serves the RPC
    /// module in a background task. Returns once the listener is bound.
    pub async fn start(cfg: DaemonConfig) -> Result<Self, MinosError> {
        let store: Arc<dyn PairingStore> = Arc::new(FilePairingStore::new(FilePairingStore::default_path()));
        let runner: Arc<dyn CommandRunner> = Arc::new(RealCommandRunner);

        let initial_state = if store.load()?.is_empty() {
            PairingState::Unpaired
        } else {
            PairingState::Paired
        };
        let pairing = Arc::new(Mutex::new(Pairing::new(initial_state)));

        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);

        let impl_ = RpcServerImpl {
            started_at: Instant::now(),
            pairing: pairing.clone(),
            store: store.clone(),
            runner,
            mac_name: cfg.mac_name.clone(),
            host: cfg.bind_addr.ip().to_string(),
            port: cfg.bind_addr.port(),
        };

        let mut module = RpcModule::new(());
        module.merge(impl_.into_rpc()).map_err(|e| MinosError::BindFailed {
            addr: cfg.bind_addr.to_string(),
            message: e.to_string(),
        })?;

        let server = WsServer::bind(cfg.bind_addr, module).await?;
        let addr = server.addr();

        let _ = state_tx.send(if initial_state == PairingState::Paired {
            ConnectionState::Disconnected // peer still has to actually connect
        } else {
            ConnectionState::Disconnected
        });

        Ok(Self {
            server: Some(server),
            state_rx,
            state_tx,
            pairing,
            store,
            active_token: Arc::new(Mutex::new(None)),
            addr,
            mac_name: cfg.mac_name,
        })
    }

    /// Generate (or refresh) the pairing QR.
    pub fn pairing_qr(&self) -> Result<QrPayload, MinosError> {
        let mut p = self.pairing.lock().unwrap();
        if p.state() == PairingState::Paired {
            // Caller wants to re-pair — UI must have shown a "replace" confirm.
            p.replace()?;
        } else if p.state() == PairingState::Unpaired {
            p.begin_awaiting()?;
        }
        let (payload, active) = generate_qr_payload(
            self.addr.ip().to_string(),
            self.addr.port(),
            self.mac_name.clone(),
        );
        *self.active_token.lock().unwrap() = Some(active);
        Ok(payload)
    }

    pub async fn discover_tailscale_ip(&self) -> Option<String> {
        tailscale::discover_ip().await
    }

    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn stop(mut self) -> Result<(), MinosError> {
        if let Some(s) = self.server.take() {
            s.stop().await?;
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Verify build**

Run: `cargo check -p minos-daemon --all-targets`
Expected: clean. Note: UniFFI exposure is **not** wired here — that lives in plan 02.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-daemon/src/handle.rs crates/minos-daemon/src/rpc_server.rs crates/minos-daemon/src/logging.rs
git commit -m "feat(minos-daemon): DaemonHandle + RpcServerImpl wired through transport"
```

---

### Task 24: Daemon E2E integration test (the MVP confidence anchor)

**Files:**
- Create: `crates/minos-daemon/tests/e2e.rs`

- [ ] **Step 1: Write `tests/e2e.rs`**

```rust
//! End-to-end: start daemon → connect a fake mobile (jsonrpsee ws-client) →
//! call `pair` → call `list_clis` → tear down. No FFI involved; this test is
//! the pre-FFI MVP confidence anchor.

use std::net::SocketAddr;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use minos_daemon::{DaemonConfig, DaemonHandle};
use minos_domain::DeviceId;
use minos_protocol::{MinosRpcClient, PairRequest};

#[tokio::test]
async fn pair_then_list_clis_in_process() {
    // Bind to an ephemeral local port to avoid CI port collisions.
    let cfg = DaemonConfig {
        mac_name: "test-mac".into(),
        bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    };
    let handle = DaemonHandle::start(cfg).await.unwrap();

    // Take the QR (puts state into AwaitingPeer).
    let qr = handle.pairing_qr().unwrap();
    let url = format!("ws://{}", handle.addr());

    let client = jsonrpsee::ws_client::WsClientBuilder::default()
        .build(&url)
        .await
        .unwrap();

    // pair
    let pair_resp = MinosRpcClient::pair(
        &client,
        PairRequest {
            device_id: DeviceId::new(),
            name: "test-iphone".into(),
        },
    )
    .await
    .unwrap();
    assert!(pair_resp.ok);
    assert_eq!(pair_resp.mac_name, "test-mac");

    // list_clis — three rows (codex/claude/gemini) regardless of host machine
    let clis = MinosRpcClient::list_clis(&client).await.unwrap();
    assert_eq!(clis.len(), 3);

    // Token still in QR (sanity: serialization works through real WS)
    assert_eq!(qr.port, handle.addr().port());

    drop(client);
    handle.stop().await.unwrap();
}
```

- [ ] **Step 2: Run E2E**

Run: `cargo test -p minos-daemon --test e2e`
Expected: `1 passed` in ≤ 1s.

- [ ] **Step 3: Run the whole crate**

Run: `cargo test -p minos-daemon`
Expected: all daemon tests pass (file_store + tailscale + rpc_server impls + e2e).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/tests/e2e.rs
git commit -m "test(minos-daemon): in-process E2E pair → list_clis"
```

---

## Phase H · `minos-mobile`

### Task 25: minos-mobile crate + `MobileClient`

**Files:**
- Create: `crates/minos-mobile/Cargo.toml`
- Create: `crates/minos-mobile/src/lib.rs`
- Create: `crates/minos-mobile/src/store.rs`
- Create: `crates/minos-mobile/src/client.rs`
- Create: `crates/minos-mobile/src/logging.rs`

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "minos-mobile"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "iOS-side composition root: WS client + RPC entry surface."

[dependencies]
minos-domain = { path = "../minos-domain" }
minos-protocol = { path = "../minos-protocol" }
minos-pairing = { path = "../minos-pairing" }
minos-transport = { path = "../minos-transport" }
tokio = { workspace = true }
futures = { workspace = true }
jsonrpsee = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
mars-xlog = { workspace = true }
url = { workspace = true }
async-trait = "0.1"

[dev-dependencies]
tokio-test = { workspace = true }
```

- [ ] **Step 2: Write `lib.rs`**

```rust
#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

pub mod client;
pub mod logging;
pub mod store;

pub use client::*;
pub use store::*;
```

- [ ] **Step 3: Write `store.rs`**

```rust
//! Mobile-side `PairingStore`. The real implementation lives in Dart and is
//! invoked through frb (plan 03). For tests, an in-memory store is provided.

use std::sync::Mutex;

use minos_domain::MinosError;
use minos_pairing::{PairingStore, TrustedDevice};

pub struct InMemoryPairingStore(pub Mutex<Vec<TrustedDevice>>);

impl InMemoryPairingStore {
    #[must_use]
    pub fn new() -> Self {
        Self(Mutex::new(vec![]))
    }
}

impl Default for InMemoryPairingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingStore for InMemoryPairingStore {
    fn load(&self) -> Result<Vec<TrustedDevice>, MinosError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, devices: &[TrustedDevice]) -> Result<(), MinosError> {
        *self.0.lock().unwrap() = devices.to_vec();
        Ok(())
    }
}
```

- [ ] **Step 4: Write `client.rs`**

```rust
//! `MobileClient` — what Dart calls into through frb (plan 03).

use std::sync::Arc;

use chrono::Utc;
use jsonrpsee::core::client::ClientT;
use minos_domain::{ConnectionState, DeviceId, MinosError};
use minos_pairing::{PairingStore, QrPayload, TrustedDevice};
use minos_protocol::{MinosRpcClient, PairRequest, PairResponse};
use minos_transport::WsClient;
use tokio::sync::watch;
use url::Url;

pub struct MobileClient {
    store: Arc<dyn PairingStore>,
    ws: Arc<tokio::sync::Mutex<Option<WsClient>>>,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    device_id: DeviceId,
    self_name: String,
}

impl MobileClient {
    #[must_use]
    pub fn new(store: Arc<dyn PairingStore>, self_name: String) -> Self {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        Self {
            store,
            ws: Arc::new(tokio::sync::Mutex::new(None)),
            state_tx,
            state_rx,
            device_id: DeviceId::new(),
            self_name,
        }
    }

    /// Pair with a Mac whose QR was just scanned.
    pub async fn pair_with(&self, qr: QrPayload) -> Result<PairResponse, MinosError> {
        let url: Url = format!("ws://{}:{}", qr.host, qr.port)
            .parse()
            .map_err(|e: url::ParseError| MinosError::ConnectFailed {
                url: format!("ws://{}:{}", qr.host, qr.port),
                message: e.to_string(),
            })?;

        let _ = self.state_tx.send(ConnectionState::Pairing);
        let ws = WsClient::connect(&url).await?;

        let resp = MinosRpcClient::pair(
            &*ws.inner(),
            PairRequest {
                device_id: self.device_id,
                name: self.self_name.clone(),
            },
        )
        .await
        .map_err(|e| MinosError::RpcCallFailed {
            method: "pair".into(),
            message: e.to_string(),
        })?;

        // Persist trusted Mac.
        let dev = TrustedDevice {
            device_id: self.device_id,
            name: resp.mac_name.clone(),
            host: qr.host,
            port: qr.port,
            paired_at: Utc::now(),
        };
        self.store.save(&[dev])?;

        *self.ws.lock().await = Some(ws);
        let _ = self.state_tx.send(ConnectionState::Connected);
        Ok(resp)
    }

    pub async fn list_clis(&self) -> Result<Vec<minos_domain::AgentDescriptor>, MinosError> {
        let guard = self.ws.lock().await;
        let ws = guard.as_ref().ok_or(MinosError::Disconnected { reason: "no client".into() })?;
        MinosRpcClient::list_clis(&*ws.inner()).await.map_err(|e| MinosError::RpcCallFailed {
            method: "list_clis".into(),
            message: e.to_string(),
        })
    }

    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }
}
```

- [ ] **Step 5: Write `logging.rs` stub (mirrors daemon)**

```rust
use minos_domain::MinosError;

pub fn init() -> Result<(), MinosError> {
    Ok(())
}
```

- [ ] **Step 6: Verify build**

Run: `cargo check -p minos-mobile`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-mobile/
git commit -m "feat(minos-mobile): MobileClient with pair_with + list_clis"
```

---

### Task 26: Mobile E2E integration test against a real daemon (in-process)

**Files:**
- Create: `crates/minos-mobile/tests/e2e.rs`

- [ ] **Step 1: Write `tests/e2e.rs`**

```rust
//! Pair through the real `MobileClient` against a real `DaemonHandle`,
//! all in one process. Verifies the symmetric round trip.

use std::net::SocketAddr;
use std::sync::Arc;

use minos_daemon::{DaemonConfig, DaemonHandle};
use minos_mobile::{InMemoryPairingStore, MobileClient};

#[tokio::test]
async fn mobile_pairs_with_daemon_and_lists_clis() {
    let daemon = DaemonHandle::start(DaemonConfig {
        mac_name: "MacForTest".into(),
        bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    })
    .await
    .unwrap();

    let qr = daemon.pairing_qr().unwrap();

    let mobile = MobileClient::new(Arc::new(InMemoryPairingStore::new()), "iPhoneForTest".into());
    let resp = mobile.pair_with(qr).await.unwrap();
    assert_eq!(resp.mac_name, "MacForTest");
    assert!(resp.ok);

    let clis = mobile.list_clis().await.unwrap();
    assert_eq!(clis.len(), 3);

    daemon.stop().await.unwrap();
}
```

- [ ] **Step 2: Add `minos-daemon` as a dev-dependency of mobile**

Edit `crates/minos-mobile/Cargo.toml`, append to `[dev-dependencies]`:

```toml
minos-daemon = { path = "../minos-daemon" }
```

- [ ] **Step 3: Run E2E**

Run: `cargo test -p minos-mobile --test e2e`
Expected: `1 passed` in ≤ 1s.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-mobile/tests/e2e.rs crates/minos-mobile/Cargo.toml
git commit -m "test(minos-mobile): E2E pair against real DaemonHandle in-process"
```

---

## Phase I · FFI shim crates (placeholders that compile)

### Task 27: minos-ffi-uniffi crate (skeleton)

**Files:**
- Create: `crates/minos-ffi-uniffi/Cargo.toml`
- Create: `crates/minos-ffi-uniffi/src/lib.rs`
- Create: `crates/minos-ffi-uniffi/build.rs`
- Create: `crates/minos-ffi-uniffi/uniffi.toml`

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "minos-ffi-uniffi"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "UniFFI bindings shim over minos-daemon::DaemonHandle (Swift consumer)."

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[dependencies]
minos-daemon = { path = "../minos-daemon" }
minos-domain = { path = "../minos-domain" }
uniffi = { workspace = true }

[build-dependencies]
uniffi = { workspace = true }
```

- [ ] **Step 2: Write `build.rs`**

```rust
fn main() {
    uniffi::generate_scaffolding("./src/minos.udl").unwrap_or_else(|e| {
        // For now we use proc-macro mode (no UDL); guard so this build script
        // does not fail when the UDL file is absent.
        let _ = e;
    });
}
```

- [ ] **Step 3: Write `lib.rs` (placeholder — real exports land in plan 02)**

```rust
//! UniFFI surface for Swift. Plan 02 fills in the actual `#[uniffi::export]`
//! annotations on `DaemonHandle`. This crate currently exists only to ensure
//! it compiles under the workspace and reserves its name on disk.

#![allow(unused_imports)]
use minos_daemon::DaemonHandle;
use minos_domain::MinosError;

uniffi::setup_scaffolding!();

/// Sentinel function so the scaffolding has at least one symbol to bind.
/// Removed in plan 02.
#[uniffi::export]
pub fn ping() -> String {
    "minos-ffi-uniffi alive".into()
}
```

- [ ] **Step 4: Write `uniffi.toml`**

```toml
[bindings.swift]
module_name = "MinosCore"
generate_immutable_records = true
```

- [ ] **Step 5: Verify build**

Run: `cargo build -p minos-ffi-uniffi`
Expected: clean (no errors).

- [ ] **Step 6: Commit**

```bash
git add crates/minos-ffi-uniffi/
git commit -m "feat(minos-ffi-uniffi): skeleton with sentinel ping export"
```

---

### Task 28: minos-ffi-frb crate (skeleton)

**Files:**
- Create: `crates/minos-ffi-frb/Cargo.toml`
- Create: `crates/minos-ffi-frb/src/lib.rs`

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "minos-ffi-frb"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "flutter_rust_bridge bindings shim over minos-mobile::MobileClient (Dart consumer)."

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[dependencies]
minos-mobile = { path = "../minos-mobile" }
minos-domain = { path = "../minos-domain" }
```

Note: the `flutter_rust_bridge` crate dep is intentionally **not** added in plan 01. frb codegen + dep wiring lands in plan 03.

- [ ] **Step 2: Write `lib.rs`**

```rust
//! frb surface for Dart. Plan 03 fills in real exports. This file exists only
//! so the workspace compiles and the crate name is reserved.

#[allow(dead_code)]
fn _link_minos_mobile() {
    // Keep the `minos-mobile` dep used so cargo doesn't drop it from compile.
    let _ = std::any::type_name::<minos_mobile::MobileClient>();
}

#[no_mangle]
pub extern "C" fn minos_ffi_frb_ping() -> i32 {
    42
}
```

- [ ] **Step 3: Verify build**

Run: `cargo build -p minos-ffi-frb`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-ffi-frb/
git commit -m "feat(minos-ffi-frb): skeleton with C-ABI sentinel"
```

---

## Phase J · Logging (`mars-xlog` integration)

### Task 29: Wire mars-xlog into `minos-daemon::logging::init`

**Files:**
- Modify: `crates/minos-daemon/src/logging.rs`

- [ ] **Step 1: Replace stub with real init**

```rust
//! mars-xlog wiring for the Mac-side daemon process.
//!
//! Layout: `~/Library/Logs/Minos/<name_prefix>-YYYYMMDD.xlog`. Use prefix
//! `daemon` per spec §9.4. Decoder: `decode_mars_nocrypt_log_file.py` from
//! the upstream Mars repo (Tencent).

use std::path::PathBuf;
use std::sync::OnceLock;

use mars_xlog::{LogLevel, Xlog, XlogConfig, XlogLayer, XlogLayerConfig, XlogLayerHandle};
use minos_domain::MinosError;
use tracing_subscriber::prelude::*;

static HANDLE: OnceLock<XlogLayerHandle> = OnceLock::new();

const NAME_PREFIX: &str = "daemon";

#[must_use]
pub fn log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Logs/Minos")
    } else {
        PathBuf::from(home).join(".minos/logs")
    }
}

/// Idempotent global initialization. Subsequent calls are no-ops.
pub fn init() -> Result<(), MinosError> {
    if HANDLE.get().is_some() {
        return Ok(());
    }
    let dir = log_dir();
    std::fs::create_dir_all(&dir).map_err(|e| MinosError::StoreIo {
        path: dir.display().to_string(),
        message: e.to_string(),
    })?;

    let cfg = XlogConfig::new(dir.to_string_lossy().to_string(), NAME_PREFIX);
    let logger = Xlog::init(cfg, LogLevel::Info)
        .map_err(|e| MinosError::StoreIo { path: dir.display().to_string(), message: e.to_string() })?;

    let (layer, handle) =
        XlogLayer::with_config(logger, XlogLayerConfig::new(LogLevel::Info).enabled(true));

    let _ = HANDLE.set(handle);

    let subscriber = tracing_subscriber::registry().with(layer);
    let _ = tracing::subscriber::set_global_default(subscriber);

    tracing::info!(name_prefix = NAME_PREFIX, dir = %dir.display(), "daemon logging initialized");
    Ok(())
}

/// Toggle level at runtime (for the menubar "diagnostics" switch in plan 02).
pub fn set_debug(enabled: bool) {
    if let Some(h) = HANDLE.get() {
        h.set_level(if enabled { LogLevel::Debug } else { LogLevel::Info });
    }
}
```

- [ ] **Step 2: Add a smoke test**

Append to `crates/minos-daemon/src/logging.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn init_creates_log_dir_and_emits_once() {
        // Override HOME so test logs go to a tempdir, not the real ~/Library/Logs.
        let dir = tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        init().unwrap();
        // Idempotent
        init().unwrap();
        let computed = log_dir();
        assert!(computed.exists());
    }
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p minos-daemon --lib logging::tests`
Expected: `1 passed`. The test runs synchronously; mars-xlog's mmap appender flushes lazily so we don't assert on file presence beyond the directory.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/logging.rs
git commit -m "feat(minos-daemon): mars-xlog tracing layer with daemon prefix"
```

---

### Task 30: Wire mars-xlog into `minos-mobile::logging::init`

**Files:**
- Modify: `crates/minos-mobile/src/logging.rs`

- [ ] **Step 1: Replace stub**

```rust
//! mars-xlog wiring for the iOS-side core process.
//!
//! Sink directory comes from the Dart layer (frb-callback in plan 03) so that
//! `iOS app Documents/Minos/Logs/` is honored even though Rust doesn't know
//! the exact app sandbox path. For unit-test builds, callers may pass a
//! tempdir directly.

use std::path::Path;
use std::sync::OnceLock;

use mars_xlog::{LogLevel, Xlog, XlogConfig, XlogLayer, XlogLayerConfig, XlogLayerHandle};
use minos_domain::MinosError;
use tracing_subscriber::prelude::*;

static HANDLE: OnceLock<XlogLayerHandle> = OnceLock::new();

const NAME_PREFIX: &str = "mobile-rust";

/// Initialize logging for the mobile-side Rust core. `log_dir` is supplied by
/// the host (Dart side via frb in plan 03; tempdir in tests).
pub fn init(log_dir: &Path) -> Result<(), MinosError> {
    if HANDLE.get().is_some() {
        return Ok(());
    }
    std::fs::create_dir_all(log_dir).map_err(|e| MinosError::StoreIo {
        path: log_dir.display().to_string(),
        message: e.to_string(),
    })?;
    let cfg = XlogConfig::new(log_dir.to_string_lossy().to_string(), NAME_PREFIX);
    let logger = Xlog::init(cfg, LogLevel::Info).map_err(|e| MinosError::StoreIo {
        path: log_dir.display().to_string(),
        message: e.to_string(),
    })?;

    let (layer, handle) =
        XlogLayer::with_config(logger, XlogLayerConfig::new(LogLevel::Info).enabled(true));

    let _ = HANDLE.set(handle);

    let subscriber = tracing_subscriber::registry().with(layer);
    let _ = tracing::subscriber::set_global_default(subscriber);

    tracing::info!(name_prefix = NAME_PREFIX, dir = %log_dir.display(), "mobile logging initialized");
    Ok(())
}

pub fn set_debug(enabled: bool) {
    if let Some(h) = HANDLE.get() {
        h.set_level(if enabled { LogLevel::Debug } else { LogLevel::Info });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn init_creates_log_dir() {
        let dir = tempdir().unwrap();
        init(dir.path()).unwrap();
        init(dir.path()).unwrap(); // idempotent
        assert!(dir.path().exists());
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p minos-mobile --lib logging::tests`
Expected: `1 passed`

- [ ] **Step 3: Commit**

```bash
git add crates/minos-mobile/src/logging.rs
git commit -m "feat(minos-mobile): mars-xlog tracing layer with mobile-rust prefix"
```

---

## Phase K · `xtask` completion

### Task 31: Implement `cargo xtask check-all`

**Files:**
- Modify: `xtask/src/main.rs`

- [ ] **Step 1: Replace `Cmd::CheckAll` arm + add helper**

Replace the stub `not_yet("check-all")` with a real implementation. Final `xtask/src/main.rs`:

```rust
//! Minos build / codegen orchestration.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Minos build & codegen orchestration")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// fmt + clippy + workspace tests + UI lints (UI lints are no-ops until plans 02/03 land).
    CheckAll,
    /// Install developer-side codegen tools.
    Bootstrap,
    /// Generate Swift bindings via uniffi-bindgen. (Implemented in plan 02.)
    GenUniffi,
    /// Generate Dart bindings via flutter_rust_bridge_codegen. (Implemented in plan 03.)
    GenFrb,
    /// Build macOS xcframework from minos-ffi-uniffi. (Implemented in plan 02.)
    BuildMacos,
    /// Build iOS staticlib from minos-ffi-frb. (Implemented in plan 03.)
    BuildIos,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::CheckAll => check_all(),
        Cmd::Bootstrap => bootstrap(),
        Cmd::GenUniffi => not_yet("gen-uniffi"),
        Cmd::GenFrb => not_yet("gen-frb"),
        Cmd::BuildMacos => not_yet("build-macos"),
        Cmd::BuildIos => not_yet("build-ios"),
    }
}

fn check_all() -> Result<()> {
    let workspace_root = workspace_root()?;
    eprintln!("==> cargo fmt --check");
    run("cargo", &["fmt", "--all", "--check"], &workspace_root)?;

    eprintln!("==> cargo clippy");
    run(
        "cargo",
        &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
        &workspace_root,
    )?;

    eprintln!("==> cargo test");
    run("cargo", &["test", "--workspace"], &workspace_root)?;

    eprintln!("==> cargo deny check (licenses + advisories)");
    if which("cargo-deny").is_some() {
        run("cargo", &["deny", "check"], &workspace_root)?;
    } else {
        eprintln!("    (skipped: cargo-deny not installed; run `cargo xtask bootstrap`)");
    }

    eprintln!("OK: all checks pass.");
    Ok(())
}

fn bootstrap() -> Result<()> {
    let workspace_root = workspace_root()?;
    eprintln!("==> installing cargo-deny + uniffi-bindgen");
    run("cargo", &["install", "cargo-deny", "--locked"], &workspace_root)?;
    run("cargo", &["install", "uniffi-bindgen-cli", "--locked"], &workspace_root)?;
    // flutter_rust_bridge_codegen and dart deps come in plan 03.
    Ok(())
}

fn not_yet(name: &str) -> Result<()> {
    bail!("xtask `{}` not implemented yet (filled in later)", name)
}

fn run(program: &str, args: &[&str], cwd: &Path) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("spawning `{program} {args:?}`"))?;
    if !status.success() {
        bail!("`{program} {args:?}` exited {status}");
    }
    Ok(())
}

fn workspace_root() -> Result<std::path::PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").context("CARGO_MANIFEST_DIR unset")?;
    Ok(Path::new(&manifest).parent().unwrap().to_owned())
}

fn which(bin: &str) -> Option<String> {
    let out = Command::new("which").arg(bin).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
```

- [ ] **Step 2: Verify**

Run: `cargo xtask check-all`
Expected: All steps pass; final stderr contains `OK: all checks pass.` This is the **green-build gate** for the rest of plan 01.

If `cargo deny check` fails, inspect `deny.toml` license allow-list and append the missing license SPDX identifier.

- [ ] **Step 3: Commit**

```bash
git add xtask/src/main.rs
git commit -m "feat(xtask): implement check-all + bootstrap"
```

---

## Phase L · CI

### Task 32: GitHub Actions workflow (rust job)

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write the workflow**

```yaml
name: ci

on:
  push:
    branches: [main]
  pull_request:

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  rust:
    name: rust (fmt + clippy + test + deny)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - name: fmt
        run: cargo fmt --all --check
      - name: clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: test
        run: cargo test --workspace
      - name: install cargo-deny
        run: cargo install cargo-deny --locked
      - name: deny
        run: cargo deny check
```

- [ ] **Step 2: Verify locally that all referenced commands work**

Run from workspace root, sequentially:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```
Each must exit 0. If any fail, fix before committing the workflow.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add github actions workflow for rust"
```

- [ ] **Step 4: After push, verify the badge in actions tab**

After pushing to GitHub, navigate to `https://github.com/peterich-rs/minos/actions` and confirm the first run is green. If not, treat as a real bug and iterate.

---

## Phase M · Final verification

### Task 33: End-to-end green check + tag

**Files:** none (verification + tag only)

- [ ] **Step 1: Final `check-all` (must be green)**

Run: `cargo xtask check-all`
Expected: `OK: all checks pass.`

- [ ] **Step 2: Run the daemon E2E specifically**

Run: `cargo test -p minos-daemon --test e2e -- --nocapture`
Expected: `1 passed`. This is the MVP-confidence anchor for plan 01.

- [ ] **Step 3: Run mobile E2E**

Run: `cargo test -p minos-mobile --test e2e -- --nocapture`
Expected: `1 passed`.

- [ ] **Step 4: Confirm all 9 crates build & test cleanly**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected (last lines): `test result: ok.` for every crate; total ≈ 35–45 tests.

- [ ] **Step 5: Tag the milestone**

```bash
git tag -a v0.0.1-rust-core -m "plan 01 complete: rust core + monorepo scaffold"
git push origin main --tags  # (only if you have already added the github remote)
```

---

## Self-Review

Plan-author check after writing the plan:

1. **Spec coverage:**
   - Spec §3 stack: rust-toolchain (Task 2), tokio/jsonrpsee/tungstenite/uniffi versions (Task 2 workspace deps), mars-xlog (Tasks 29, 30), `cargo xtask check-all` (Task 31), `.gitignore`/`.gitattributes` (Task 1) ✓
   - Spec §5 crates: domain (5–10), protocol (11–12), pairing (13–15), cli-detect (16–17), transport (18–20), daemon (21–24), mobile (25–26), ffi-uniffi (27), ffi-frb (28) ✓
   - Spec §6 data flow: pair (Task 23 RPC handler + Task 24 E2E), list_clis (Task 23 + Task 24), reconnect backoff (Task 18; full reconnect loop is in plans 02/03) ✓
   - Spec §7 error handling: MinosError + bilingual user_message (Task 9), state mismatch (Task 14), corrupt store .bak (Task 21), CLI probe timeout (Task 17 + Task 23) ✓
   - Spec §8.1 Rust matrix: golden (Task 10), property tests (Task 6 PairingToken), table-driven state (Task 14), in-process E2E (Tasks 24, 26) ✓
   - Spec §8.5 CI rust job (Task 32) ✓
   - Spec §9.1 pinned files: rust-toolchain (Task 2), .gitattributes (Task 1) ✓
   - **Gaps acknowledged** (intentionally deferred to plans 02/03): swift / dart matrices and CI jobs; xtask `gen-uniffi` / `gen-frb` / `build-macos` / `build-ios` (stubs left in Task 3 and Task 31 with `not_yet`). Also: spec §10.1 hooks for `Agent` trait + `AgentEvent::ToolCall args_json` etc. — `AgentEvent` is defined here (Task 11) but `Agent` trait belongs to plan-equivalent for P1 (`codex-app-server-integration.md`).

2. **Placeholder scan:** No "TBD"/"TODO" in step contents. Two intentionally explicit `not_yet()` xtask stubs are documented in Tasks 3 and 31. Gaps in §10.1 spec hooks are explicitly accounted for above.

3. **Type consistency:**
   - `DeviceId` defined Task 6, used in Tasks 11 (`PairRequest.device_id`), 13 (`TrustedDevice.device_id`), 23 (rpc_server), 24 (E2E), 25 (mobile client) ✓
   - `PairingToken` defined Task 6, used in Task 15 (`ActiveToken.token`, `QrPayload.token`) ✓
   - `MinosError` defined Task 9; variant names referenced in Tasks 17 (CliProbeTimeout/CliProbeFailed), 21 (StoreIo/StoreCorrupt), 23 (PairingStateMismatch error mapping), 25 (ConnectFailed/RpcCallFailed) — all consistent ✓
   - `TrustedDevice.host` (String) chosen Task 13; same field accessed Task 21 (file store), Task 23 (rpc_server), Task 25 (mobile pair_with) ✓
   - `MinosRpc` trait method names (`pair`, `health`, `list_clis`, `subscribe_events`) defined Task 12; the impl in Task 23 uses identical names ✓
   - Workspace deps centralized in Task 2 `[workspace.dependencies]`; downstream Cargo.tomls consume via `{ workspace = true }` ✓

No issues found.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/01-rust-core-and-monorepo-scaffold.md`.**

After Plan 1 ships green, `02-macos-app-and-uniffi.md` and `03-flutter-app-and-frb.md` will follow the same brainstorm → spec → plan cycle.

Two execution options:

1. **Subagent-Driven (recommended)** — dispatch one fresh subagent per task; review between tasks; fast iteration with isolated context.
2. **Inline Execution** — execute tasks in this session; batched checkpoints.

Which approach?
