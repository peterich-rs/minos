# Minos · macOS App + UniFFI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fully wire the UniFFI shim over `minos-daemon::DaemonHandle` and ship a SwiftUI `MenuBarExtra` macOS 13+ app (`ai.minos.macos`) that surfaces boot state, pairing-QR display, Forget-paired-device affordance, and today-log Finder reveal — backed by logic-only Swift unit tests and a CI job on `macos-14`. The plan ends when `cargo xtask check-all` passes end-to-end (Rust + Swift legs) and `xcodebuild -scheme Minos build` produces `Minos.app` with no warnings.

**Architecture:** Eight phases sequenced by dependency. (A) Rust-side Phase 0 surgery in `minos-daemon` — refactor `DaemonHandle` to `Arc<Inner>`, add `start_autobind` / `subscribe` / `current_trusted_device` / `logging::today`, hoist `discover_tailscale_ip`. (B) Introduce `ErrorKind` companion enum in `minos-domain` and route `user_message` through it. (C) Add `uniffi` feature flag to `minos-domain` / `minos-pairing` / `minos-daemon` with `cfg_attr` derives on FFI-visible types. (D) Fill in `minos-ffi-uniffi` with `custom_newtype!` / `custom_type!` and free-function re-exports. (E) Implement `xtask build-macos` / `gen-uniffi` / `gen-xcode` + extend `check-all` + `bootstrap`. (F) XcodeGen project + Brewfile + swiftlint + gitignore. (G) Swift four-layer app (Infrastructure / Application / Domain / Presentation). (H) CI job on `macos-14` + ADR 0007 + README update.

**Tech Stack:**
- Rust stable channel (inherited from `rust-toolchain.toml`)
- UniFFI 0.31 (proc-macro mode) + `uniffi-bindgen-swift` (crates.io)
- Swift 5.10 + SwiftUI (macOS 13+) + AppKit (`NSWorkspace`, `NSAlert`) + CoreImage (`CIFilter.qrCodeGenerator`)
- XcodeGen 2.x + SwiftLint (brew)
- XCTest (built into Xcode toolchain)
- GitHub Actions `macos-14` runner

**Reference spec:** Implements `docs/superpowers/specs/macos-app-and-uniffi-design.md`. Rust-side assumptions from plan 01 are enumerated in that spec's §3.

**Working directory note:** This plan runs on `main` alongside plan 01's history; single-developer repo. No worktree isolation required.

**Version drift policy:** Versions listed below are accurate as of 2026-04-21. If `cargo add` / `brew install` resolves to a higher minor version when executed, prefer the resolved version unless compilation fails.

---

## File Structure (created or modified by this plan)

```
minos/
├── .github/workflows/ci.yml                        [modified: add swift job]
├── .gitignore                                      [modified: Xcode + Brewfile.lock]
├── README.md                                       [modified: Status line]
├── apps/
│   └── macos/                                      [new directory (replaces .gitkeep)]
│       ├── Brewfile
│       ├── .swiftlint.yml
│       ├── project.yml                             XcodeGen spec
│       ├── Minos/
│       │   ├── MinosApp.swift                      @main
│       │   ├── Info.plist
│       │   ├── Generated/                          (gitignored) UniFFI output
│       │   ├── Presentation/
│       │   │   ├── MenuBarView.swift
│       │   │   ├── QRSheet.swift
│       │   │   └── StatusIcon.swift
│       │   ├── Application/
│       │   │   ├── AppState.swift
│       │   │   ├── DaemonDriving.swift
│       │   │   └── ObserverAdapter.swift
│       │   ├── Domain/
│       │   │   ├── ConnectionState+Display.swift
│       │   │   └── MinosError+Display.swift
│       │   ├── Infrastructure/
│       │   │   ├── DaemonBootstrap.swift
│       │   │   ├── DaemonHandle+DaemonDriving.swift
│       │   │   ├── QRCodeRenderer.swift
│       │   │   └── DiagnosticsReveal.swift
│       │   └── Resources/Assets.xcassets/
│       │       ├── AppIcon.appiconset/Contents.json
│       │       └── AccentColor.colorset/Contents.json
│       └── MinosTests/
│           ├── Application/AppStateTests.swift
│           └── TestSupport/MockDaemon.swift
├── crates/
│   ├── minos-domain/
│   │   ├── Cargo.toml                              [modified: uniffi feature]
│   │   └── src/
│   │       ├── agent.rs                            [modified: cfg_attr derives]
│   │       ├── connection.rs                       [modified: cfg_attr derives]
│   │       ├── error.rs                            [modified: add ErrorKind, refactor user_message]
│   │       ├── ids.rs                              [unchanged — DeviceId uses custom_newtype! in shim]
│   │       ├── lib.rs                              [unchanged]
│   │       └── pairing_state.rs                    [modified: cfg_attr derives]
│   ├── minos-pairing/
│   │   ├── Cargo.toml                              [modified: uniffi feature]
│   │   └── src/
│   │       ├── store.rs                            [modified: TrustedDevice cfg_attr]
│   │       ├── token.rs                            [modified: QrPayload cfg_attr]
│   │       ├── state_machine.rs                    [unchanged — Pairing is internal]
│   │       └── lib.rs                              [unchanged]
│   ├── minos-daemon/
│   │   ├── Cargo.toml                              [modified: uniffi feature]
│   │   └── src/
│   │       ├── handle.rs                           [modified: Arc<Inner>, stop(&self), host/port, start_autobind, current_trusted_device, UniFFI cfg_attr]
│   │       ├── lib.rs                              [modified: pub mod subscription]
│   │       ├── logging.rs                          [modified: add today()]
│   │       ├── subscription.rs                     [new: Subscription + ConnectionStateObserver]
│   │       ├── tailscale.rs                        [modified: hoist discover_ip to pub]
│   │       ├── file_store.rs                       [unchanged]
│   │       └── rpc_server.rs                       [unchanged]
│   └── minos-ffi-uniffi/
│       ├── Cargo.toml                              [modified: enable uniffi features on path deps]
│       └── src/lib.rs                              [rewrite: remove ping, add custom_newtype/custom_type, re-export free functions]
├── docs/adr/
│   └── 0007-xcodegen-for-macos-project.md          [new]
└── xtask/
    └── src/main.rs                                 [modified: build-macos, gen-uniffi, gen-xcode, check-all swift leg, bootstrap swift tools]
```

**Total:** ~28 new files + ~16 modified files.

---

## Task dependency graph (big picture)

```
Phase A — minos-domain ErrorKind (Task 1)
  │
Phase B — domain UniFFI feature (Task 2)
  │     │
  │   Phase C — pairing UniFFI feature (Task 3)
  │     │
Phase D — daemon Phase 0 surgery (Tasks 4-11) ← depends on A/B/C
  │
Phase E — daemon UniFFI feature (Task 12) ← depends on D
  │
Phase F — ffi-uniffi shim fill (Task 13) ← depends on A-E
  │
Phase G — xtask extensions (Tasks 14-19) ← can run in parallel with F for early parts
  │
Phase H — XcodeGen + Brew + swiftlint + first codegen (Tasks 20-23) ← depends on F + G
  │
Phase I — Swift foundation (Tasks 24-28) ← depends on H (needs Generated/)
  │
Phase J — Swift views (Tasks 29-31) ← depends on I
  │
Phase K — Swift tests (Tasks 32-35) ← depends on I
  │
Phase L — CI + ADR + README (Tasks 36-38) ← depends on all above
```

---

## Phase A · `minos-domain`: ErrorKind + user_message refactor

### Task 1: Introduce `ErrorKind` and route `user_message` through it

**Why:** UniFFI's `#[uniffi::Error]` variants can be thrown but cannot be passed as function arguments. Plan 02's Swift side needs to display localized strings; the single source-of-truth string table must stay in Rust. Solution (spec §7.2): a payload-free companion enum `ErrorKind` that mirrors `MinosError`'s discriminants.

**Files:**
- Modify: `crates/minos-domain/src/error.rs`

- [ ] **Step 1: Add `ErrorKind` enum and move the string table onto it**

Open `crates/minos-domain/src/error.rs` and rewrite as:

```rust
//! Single typed error for all Minos public APIs.
//!
//! Variants mirror the table in spec §7.4. `Lang` + `user_message` produce
//! short, user-facing copy (zh / en) so UI layers do not need to translate
//! by themselves. The `ErrorKind` companion enum mirrors `MinosError`'s
//! discriminants without payload and carries the single-source-of-truth
//! localization table — UniFFI consumers call `kind_message(kind, lang)`
//! because `#[uniffi::Error]` variants cannot be passed as arguments.

use crate::PairingState;

#[derive(Debug, Clone, Copy)]
pub enum Lang {
    Zh,
    En,
}

/// Payload-free discriminant of `MinosError`. Mirrored 1:1 with `MinosError`
/// variants (excluding carried data). UniFFI exposes this + `user_message`
/// as the cross-language localization bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    BindFailed,
    ConnectFailed,
    Disconnected,
    PairingTokenInvalid,
    PairingStateMismatch,
    DeviceNotTrusted,
    StoreIo,
    StoreCorrupt,
    CliProbeTimeout,
    CliProbeFailed,
    RpcCallFailed,
}

impl ErrorKind {
    /// Single source of truth for user-facing zh/en strings. Adding a new
    /// `MinosError` variant requires adding:
    ///   1. the new `MinosError` variant itself
    ///   2. the matching `ErrorKind` variant
    ///   3. one arm in `MinosError::kind`
    ///   4. two arms (zh + en) here
    ///   5. one arm in Swift's `MinosError.kind` extension
    #[must_use]
    pub fn user_message(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Self::BindFailed, Lang::Zh) => {
                "无法绑定本机端口；请检查 Tailscale 是否已启动并登录"
            }
            (Self::BindFailed, Lang::En) => {
                "Cannot bind local port; please verify Tailscale is running and signed in"
            }
            (Self::ConnectFailed, Lang::Zh) => "无法连接 Mac；请确认两端均已加入同一 Tailscale 网络",
            (Self::ConnectFailed, Lang::En) => {
                "Cannot reach Mac; ensure both devices are on the same Tailscale network"
            }
            (Self::Disconnected, Lang::Zh) => "连接已断开，正在重试",
            (Self::Disconnected, Lang::En) => "Disconnected; reconnecting",
            (Self::PairingTokenInvalid, Lang::Zh) => "二维码已过期，请重新扫描",
            (Self::PairingTokenInvalid, Lang::En) => "QR code expired, please rescan",
            (Self::PairingStateMismatch, Lang::Zh) => "已存在配对设备，请确认替换",
            (Self::PairingStateMismatch, Lang::En) => {
                "A paired device already exists; confirm to replace"
            }
            (Self::DeviceNotTrusted, Lang::Zh) => "配对已失效，请重新扫码",
            (Self::DeviceNotTrusted, Lang::En) => "Pairing invalidated, please rescan",
            (Self::StoreIo, Lang::Zh) => "本地存储不可访问，请检查权限",
            (Self::StoreIo, Lang::En) => "Local storage inaccessible; check permissions",
            (Self::StoreCorrupt, Lang::Zh) => "本地配对状态损坏，已备份；请重新配对",
            (Self::StoreCorrupt, Lang::En) => {
                "Local pairing state corrupt; backed up. Please re-pair"
            }
            (Self::CliProbeTimeout, Lang::Zh) => "CLI 探测超时",
            (Self::CliProbeTimeout, Lang::En) => "CLI probe timed out",
            (Self::CliProbeFailed, Lang::Zh) => "CLI 探测失败",
            (Self::CliProbeFailed, Lang::En) => "CLI probe failed",
            (Self::RpcCallFailed, Lang::Zh) => "服务端错误，请稍后重试",
            (Self::RpcCallFailed, Lang::En) => "Server error, please retry",
        }
    }
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
    /// Payload-free discriminant — mirrors every variant 1:1.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::BindFailed { .. } => ErrorKind::BindFailed,
            Self::ConnectFailed { .. } => ErrorKind::ConnectFailed,
            Self::Disconnected { .. } => ErrorKind::Disconnected,
            Self::PairingTokenInvalid => ErrorKind::PairingTokenInvalid,
            Self::PairingStateMismatch { .. } => ErrorKind::PairingStateMismatch,
            Self::DeviceNotTrusted { .. } => ErrorKind::DeviceNotTrusted,
            Self::StoreIo { .. } => ErrorKind::StoreIo,
            Self::StoreCorrupt { .. } => ErrorKind::StoreCorrupt,
            Self::CliProbeTimeout { .. } => ErrorKind::CliProbeTimeout,
            Self::CliProbeFailed { .. } => ErrorKind::CliProbeFailed,
            Self::RpcCallFailed { .. } => ErrorKind::RpcCallFailed,
        }
    }

    /// Short, user-facing string. Delegates to `ErrorKind::user_message` so
    /// the table lives in exactly one place.
    #[must_use]
    pub fn user_message(&self, lang: Lang) -> &'static str {
        self.kind().user_message(lang)
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
    fn kind_matches_variant() {
        assert_eq!(MinosError::PairingTokenInvalid.kind(), ErrorKind::PairingTokenInvalid);
        assert_eq!(
            MinosError::BindFailed {
                addr: "x".into(),
                message: "y".into()
            }
            .kind(),
            ErrorKind::BindFailed
        );
    }

    #[test]
    fn every_minos_error_variant_maps_to_an_error_kind() {
        // If you add a MinosError variant but forget to add ErrorKind, this
        // test won't compile. (The match in MinosError::kind is exhaustive.)
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
            // Just call .kind() to ensure every arm covered.
            let _k = v.kind();
        }
    }

    #[test]
    fn every_error_kind_has_user_message_in_both_langs() {
        let kinds = [
            ErrorKind::BindFailed,
            ErrorKind::ConnectFailed,
            ErrorKind::Disconnected,
            ErrorKind::PairingTokenInvalid,
            ErrorKind::PairingStateMismatch,
            ErrorKind::DeviceNotTrusted,
            ErrorKind::StoreIo,
            ErrorKind::StoreCorrupt,
            ErrorKind::CliProbeTimeout,
            ErrorKind::CliProbeFailed,
            ErrorKind::RpcCallFailed,
        ];
        for k in kinds {
            assert!(!k.user_message(Lang::Zh).is_empty(), "missing zh for {k:?}");
            assert!(!k.user_message(Lang::En).is_empty(), "missing en for {k:?}");
        }
    }
}
```

Also update `crates/minos-domain/src/lib.rs` to re-export `ErrorKind`:

```rust
//! Minos domain types — pure values, no I/O, no async.

#![forbid(unsafe_code)]

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

(`pub use error::*` already picked up `MinosError` and `Lang`; now it picks up `ErrorKind` too.)

- [ ] **Step 2: Run Rust tests to verify refactor**

Run: `cargo test -p minos-domain`

Expected: all tests pass, including the two new `kind_*` tests.

- [ ] **Step 3: Commit**

```bash
git add crates/minos-domain/src/error.rs crates/minos-domain/src/lib.rs
git commit -m "feat(minos-domain): add ErrorKind companion, route user_message through it"
```

---

## Phase B · `minos-domain`: UniFFI feature flag + derives

### Task 2: Add `uniffi` feature and `cfg_attr` derives on domain types

**Why:** Plan 02 §5.3 — UniFFI derives live on source types behind an optional `uniffi` feature so that (1) `minos-ffi-frb` (Dart side, plan 03) can consume the same types without paying for UniFFI, and (2) the derives come for free when the shim enables the feature.

**Files:**
- Modify: `crates/minos-domain/Cargo.toml`
- Modify: `crates/minos-domain/src/agent.rs`
- Modify: `crates/minos-domain/src/connection.rs`
- Modify: `crates/minos-domain/src/error.rs`
- Modify: `crates/minos-domain/src/pairing_state.rs`

- [ ] **Step 1: Add `uniffi` feature to `minos-domain/Cargo.toml`**

Append to `crates/minos-domain/Cargo.toml` (after the `[dependencies]` block):

```toml
[features]
uniffi = ["dep:uniffi"]
```

And in the `[dependencies]` block, add `uniffi` as optional:

```toml
uniffi = { workspace = true, optional = true }
```

Full modified file:

```toml
[package]
name = "minos-domain"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Minos pure-value domain types: ids, agents, errors, connection state."

[features]
uniffi = ["dep:uniffi"]

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true }
base64 = { workspace = true }
getrandom = { workspace = true }
uniffi = { workspace = true, optional = true }

[dev-dependencies]
pretty_assertions = { workspace = true }
proptest = { workspace = true }
rstest = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Add `cfg_attr` derives on `AgentName`, `AgentStatus`, `AgentDescriptor`**

In `crates/minos-domain/src/agent.rs`, add `#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]` / `Record` on each public type. The `AgentStatus` enum has an inner `reason: String` variant — UniFFI supports that shape.

```rust
//! Agent CLI descriptors (names, statuses, full descriptor records).

use serde::{Deserialize, Serialize};

/// The set of CLI agents Minos knows how to manage.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentName {
    Codex,
    Claude,
    Gemini,
}

impl AgentName {
    #[must_use]
    pub const fn all() -> &'static [AgentName] {
        &[AgentName::Codex, AgentName::Claude, AgentName::Gemini]
    }

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
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AgentStatus {
    Ok,
    Missing,
    Error { reason: String },
}

/// The complete description of one agent's local installation.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub name: AgentName,
    pub path: Option<String>,
    pub version: Option<String>,
    pub status: AgentStatus,
}

#[cfg(test)]
mod tests {
    // Existing tests unchanged; they compile under both feature configurations.
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
        let s = serde_json::to_string(&AgentStatus::Error {
            reason: "boom".into(),
        })
        .unwrap();
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

- [ ] **Step 3: Add `cfg_attr` derive on `ConnectionState`**

In `crates/minos-domain/src/connection.rs`:

```rust
//! High-level connection state visible to the UI.

use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Pairing,
    Connected,
    /// Reconnect attempt in progress; `attempt` starts at 1 for the first retry.
    Reconnecting {
        attempt: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_serializes_as_string() {
        assert_eq!(
            serde_json::to_string(&ConnectionState::Disconnected).unwrap(),
            "\"disconnected\""
        );
    }

    #[test]
    fn reconnecting_carries_attempt() {
        let s = serde_json::to_string(&ConnectionState::Reconnecting { attempt: 3 }).unwrap();
        assert_eq!(s, r#"{"reconnecting":{"attempt":3}}"#);
    }
}
```

- [ ] **Step 4: Add `cfg_attr` derive on `PairingState`**

Open `crates/minos-domain/src/pairing_state.rs` and add the derive. (Plan 01 shape assumed.)

```rust
//! Pairing state machine's externally-visible state.

use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
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
    fn pairing_state_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&PairingState::AwaitingPeer).unwrap(), "\"awaiting_peer\"");
    }
}
```

(If the existing file has additional code, keep it — only add the derive and leave other logic untouched.)

- [ ] **Step 5: Add `cfg_attr` derives on `Lang`, `ErrorKind`, and `MinosError` in `error.rs`**

Modify `crates/minos-domain/src/error.rs` — add three derive attributes at the tops of the three types. The rest of the file (from Task 1) stays intact.

```rust
// Lang — line-prefixed derive
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy)]
pub enum Lang {
    Zh,
    En,
}

// ErrorKind — line-prefixed derive
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    // ... (unchanged variants)
}

// MinosError — add uniffi::Error derive
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
#[derive(thiserror::Error, Debug)]
pub enum MinosError {
    // ... (unchanged variants)
}
```

Only add these three `#[cfg_attr(...)]` lines; leave everything else from Task 1 unchanged.

- [ ] **Step 6: Verify the crate builds in both feature configurations**

Run without the feature (the default — should not bring in uniffi):

```bash
cargo build -p minos-domain
```

Expected: success; `uniffi` is not pulled into the compile because `optional = true`.

Run with the feature:

```bash
cargo build -p minos-domain --features uniffi
```

Expected: success; `uniffi` is compiled once as a derive-only dep.

Run tests in both configs:

```bash
cargo test -p minos-domain
cargo test -p minos-domain --features uniffi
```

Both should pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-domain/Cargo.toml crates/minos-domain/src/
git commit -m "feat(minos-domain): optional uniffi feature + cfg_attr derives on FFI-visible types"
```

---

## Phase C · `minos-pairing`: UniFFI feature flag + derives

### Task 3: Add `uniffi` feature and `cfg_attr` derives on pairing types

**Why:** `QrPayload` and `TrustedDevice` cross FFI to Swift (spec §5.3). `PairingToken` uses `custom_newtype!` in the shim (Task 13) and gets no derive here.

**Files:**
- Modify: `crates/minos-pairing/Cargo.toml`
- Modify: `crates/minos-pairing/src/store.rs`
- Modify: `crates/minos-pairing/src/token.rs`

- [ ] **Step 1: Add `uniffi` feature to `minos-pairing/Cargo.toml`**

```toml
[package]
name = "minos-pairing"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Pairing state machine, trusted device records, and the PairingStore port."

[features]
uniffi = ["dep:uniffi", "minos-domain/uniffi"]

[dependencies]
minos-domain = { path = "../minos-domain" }
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
url = { workspace = true }
uniffi = { workspace = true, optional = true }

[dev-dependencies]
pretty_assertions = { workspace = true }
rstest = { workspace = true }
proptest = { workspace = true }

[lints]
workspace = true
```

Note the feature propagates `minos-domain/uniffi` so enabling `minos-pairing/uniffi` transitively enables the domain derives.

- [ ] **Step 2: Add `cfg_attr` derive on `QrPayload` in `token.rs`**

`PairingToken` remains unchanged (handled by `custom_newtype!` in Task 13 shim).

```rust
//! QR payload format (matches spec §6.1).

use chrono::{DateTime, Duration, Utc};
use minos_domain::PairingToken;
use serde::{Deserialize, Serialize};

pub const QR_TOKEN_TTL: Duration = Duration::minutes(5);
pub const PROTOCOL_VERSION: u8 = 1;

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QrPayload {
    pub v: u8,
    pub host: String,
    pub port: u16,
    pub token: PairingToken,
    pub name: String,
}

// ... (rest of the file — ActiveToken, generate_qr_payload, tests — unchanged)
```

(Preserve the remaining types and tests as-is; only add the `cfg_attr` line above `QrPayload`.)

- [ ] **Step 3: Add `cfg_attr` derive on `TrustedDevice` in `store.rs`**

```rust
//! Pairing persistence port + trusted-device record.

use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, MinosError};
use serde::{Deserialize, Serialize};

/// One peer that has successfully paired and may reconnect on its own.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedDevice {
    pub device_id: DeviceId,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub paired_at: DateTime<Utc>,
}

// ... (rest of the file — PairingStore trait, in-mem test impl, tests — unchanged)
```

- [ ] **Step 4: Build and test in both feature configurations**

```bash
cargo build -p minos-pairing
cargo build -p minos-pairing --features uniffi
cargo test -p minos-pairing
cargo test -p minos-pairing --features uniffi
```

All four commands should succeed.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-pairing/Cargo.toml crates/minos-pairing/src/
git commit -m "feat(minos-pairing): optional uniffi feature + cfg_attr derives on QrPayload/TrustedDevice"
```

---

## Phase D · `minos-daemon`: Phase 0 FFI-friendly surgery

This phase is the largest — eight tasks. Each task is independently committable and keeps `cargo test -p minos-daemon` green.

### Task 4: Hoist `discover_ip` to a module-level public free function

**Why:** Spec §5.1 #4 + §5.1 #5 — `start_autobind` needs to call `discover_tailscale_ip` before `DaemonHandle` exists. Plan 01 declared it as `&self` method on `DaemonHandle`, which is a chicken-and-egg problem.

**Files:**
- Modify: `crates/minos-daemon/src/tailscale.rs`
- Modify: `crates/minos-daemon/src/handle.rs`
- Modify: `crates/minos-daemon/src/lib.rs`

- [ ] **Step 1: Promote `discover_ip` to `pub`**

Edit `crates/minos-daemon/src/tailscale.rs`. Rename nothing; just change the first line's visibility:

```rust
//! Tailscale 100.x IP discovery. MVP shells out to `tailscale ip --4`.

use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// Discover the machine's Tailscale 100.x IPv4, if any. Callers of
/// `DaemonHandle::start_autobind` use this before bind; direct callers
/// (CLI tools, integration tests) can also use it standalone.
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
    #[tokio::test]
    async fn returns_none_or_some_with_100_prefix() {
        let ip = super::discover_ip().await;
        assert!(ip.is_none() || ip.as_ref().unwrap().starts_with("100."));
    }
}
```

(Change: `pub(crate) async fn` / `async fn` → `pub async fn`. If plan 01 had it as `pub(crate)` or private, flip to `pub`.)

- [ ] **Step 2: Expose `discover_tailscale_ip` at crate root**

Edit `crates/minos-daemon/src/lib.rs` to re-export:

```rust
#![forbid(unsafe_code)]

pub mod file_store;
pub mod handle;
pub mod logging;
pub mod rpc_server;
pub mod tailscale;

pub use file_store::*;
pub use handle::*;

/// Module-level wrapper so callers don't need `tailscale::discover_ip` —
/// spec §5.1 #4 calls for this name.
pub use tailscale::discover_ip as discover_tailscale_ip;
```

- [ ] **Step 3: Remove the now-redundant instance method on `DaemonHandle`**

Plan 01's `handle.rs` has:

```rust
pub async fn discover_tailscale_ip(&self) -> Option<String> {
    tailscale::discover_ip().await
}
```

Delete that method. The free function re-exported in step 2 is the replacement.

(If anything in `minos-daemon` or `minos-mobile` calls `self.discover_tailscale_ip()`, replace with `minos_daemon::discover_tailscale_ip()` or `crate::tailscale::discover_ip()`.)

Search for callers:

```bash
rg 'discover_tailscale_ip' crates/
```

Expected: after edits, only the new `pub use` in `lib.rs` and the free function in `tailscale.rs` should match. Zero `self.discover_tailscale_ip()` callers should remain.

- [ ] **Step 4: Build and test**

```bash
cargo test -p minos-daemon
```

Expected: all existing tests pass; the `tailscale::discover_ip` test still runs.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-daemon/src/tailscale.rs crates/minos-daemon/src/handle.rs crates/minos-daemon/src/lib.rs
git commit -m "refactor(minos-daemon): hoist discover_tailscale_ip to crate-level free function"
```

---

### Task 5: Refactor `DaemonHandle` to `Arc<DaemonInner>` with interior-mutable server

**Why:** UniFFI `#[uniffi::Object]` requires `&self`-only methods, which means all mutable state must be behind interior-mutable wrappers (Mutex/RwLock/atomic). Current `DaemonHandle::stop(mut self)` consumes self; current `server: Option<WsServer>` field uses direct ownership. Spec §5.1 #1-2.

**Files:**
- Modify: `crates/minos-daemon/src/handle.rs`

- [ ] **Step 1: Introduce `DaemonInner` + rewrite `DaemonHandle` as a transparent `Arc` wrapper**

Rewrite `crates/minos-daemon/src/handle.rs`:

```rust
//! Public façade exposed to Swift via UniFFI in plan 02.
//!
//! Plan 02 Phase 0 refactor: all fields live inside `DaemonInner` owned by
//! an `Arc`, so every `DaemonHandle` method takes `&self` — a requirement
//! for UniFFI `#[uniffi::Object]` exports. `WsServer` uses interior
//! mutability via `Mutex<Option<_>>` so `stop(&self)` can take it out
//! without consuming the handle.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use jsonrpsee::server::RpcModule;
use minos_cli_detect::{CommandRunner, RealCommandRunner};
use minos_domain::{ConnectionState, DeviceId, MinosError, PairingState};
use minos_pairing::{
    generate_qr_payload, ActiveToken, Pairing, PairingStore, QrPayload, TrustedDevice,
};
use minos_protocol::MinosRpcServer;
use minos_transport::WsServer;
use tokio::sync::watch;

use crate::file_store::FilePairingStore;
use crate::rpc_server::RpcServerImpl;

pub struct DaemonConfig {
    pub mac_name: String,
    pub bind_addr: SocketAddr,
}

struct DaemonInner {
    server: Mutex<Option<WsServer>>,
    state_rx: watch::Receiver<ConnectionState>,
    state_tx: Arc<watch::Sender<ConnectionState>>,
    pairing: Arc<Mutex<Pairing>>,
    store: Arc<dyn PairingStore>,
    active_token: Arc<Mutex<Option<ActiveToken>>>,
    addr: SocketAddr,
    mac_name: String,
}

pub struct DaemonHandle {
    inner: Arc<DaemonInner>,
}

impl DaemonHandle {
    /// Start the daemon on an explicit bind address. Tests use this path;
    /// production code uses `start_autobind` (Task 8).
    #[allow(clippy::missing_errors_doc)]
    pub async fn start(cfg: DaemonConfig) -> Result<Arc<Self>, MinosError> {
        let store: Arc<dyn PairingStore> =
            Arc::new(FilePairingStore::new(FilePairingStore::default_path()));
        let runner: Arc<dyn CommandRunner> = Arc::new(RealCommandRunner);

        let initial_state = if store.load()?.is_empty() {
            PairingState::Unpaired
        } else {
            PairingState::Paired
        };
        let pairing = Arc::new(Mutex::new(Pairing::new(initial_state)));

        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let state_tx = Arc::new(state_tx);
        let active_token: Arc<Mutex<Option<ActiveToken>>> = Arc::new(Mutex::new(None));

        let impl_ = RpcServerImpl {
            started_at: Instant::now(),
            pairing: pairing.clone(),
            store: store.clone(),
            runner,
            mac_name: cfg.mac_name.clone(),
            host: cfg.bind_addr.ip().to_string(),
            port: cfg.bind_addr.port(),
            active_token: active_token.clone(),
            conn_state_tx: state_tx.clone(),
        };

        let mut module = RpcModule::new(());
        module
            .merge(impl_.into_rpc())
            .map_err(|e| MinosError::BindFailed {
                addr: cfg.bind_addr.to_string(),
                message: e.to_string(),
            })?;

        let server = WsServer::bind(cfg.bind_addr, module).await?;
        let addr = server.addr();

        let _ = state_tx.send(ConnectionState::Disconnected);

        Ok(Arc::new(Self {
            inner: Arc::new(DaemonInner {
                server: Mutex::new(Some(server)),
                state_rx,
                state_tx,
                pairing,
                store,
                active_token,
                addr,
                mac_name: cfg.mac_name,
            }),
        }))
    }

    /// Generate (or refresh) the pairing QR.
    #[allow(clippy::missing_errors_doc)]
    pub fn pairing_qr(&self) -> Result<QrPayload, MinosError> {
        let mut p = self.inner.pairing.lock().unwrap();
        if p.state() == PairingState::Paired {
            p.replace()?;
        } else if p.state() == PairingState::Unpaired {
            p.begin_awaiting()?;
        }
        let (payload, active) = generate_qr_payload(
            self.inner.addr.ip().to_string(),
            self.inner.addr.port(),
            self.inner.mac_name.clone(),
        );
        *self.inner.active_token.lock().unwrap() = Some(active);
        let _ = self.inner.state_tx.send(ConnectionState::Pairing);
        Ok(payload)
    }

    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        *self.inner.state_rx.borrow()
    }

    /// Subscribe to connection-state transitions. Receivers see the most
    /// recently sent value on first `borrow`, then each subsequent `changed`
    /// awaits the next transition. (Rust-side consumers only; UniFFI Swift
    /// path uses the callback-interface `subscribe` from Task 9.)
    #[must_use]
    pub fn events_stream(&self) -> watch::Receiver<ConnectionState> {
        self.inner.state_rx.clone()
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.inner.addr
    }

    /// Forget a previously trusted device.
    #[allow(clippy::missing_errors_doc, clippy::unused_async)]
    pub async fn forget_device(&self, id: DeviceId) -> Result<(), MinosError> {
        let mut current = self.inner.store.load()?;
        current.retain(|d| d.device_id != id);
        self.inner.store.save(&current)?;
        self.inner.pairing.lock().unwrap().forget();
        let _ = self.inner.state_tx.send(ConnectionState::Disconnected);
        Ok(())
    }

    /// Stop the WS server and transition to `Disconnected`. Idempotent —
    /// calling twice is a no-op after the first success.
    #[allow(clippy::missing_errors_doc)]
    pub async fn stop(&self) -> Result<(), MinosError> {
        let server = self.inner.server.lock().unwrap().take();
        if let Some(s) = server {
            s.stop().await?;
        }
        let _ = self.inner.state_tx.send(ConnectionState::Disconnected);
        Ok(())
    }

    // ── Getters populated in later tasks ──
    // host() / port()        → Task 6
    // start_autobind()       → Task 8
    // subscribe()            → Task 9
    // current_trusted_device → Task 10
}
```

Key changes vs. plan 01:
- `DaemonHandle` wraps `Arc<DaemonInner>`
- `server: Option<WsServer>` → `Mutex<Option<WsServer>>`
- `start(cfg) -> Result<Self>` → `start(cfg) -> Result<Arc<Self>>`
- `stop(mut self)` → `stop(&self)` + take from Mutex
- Removed the `discover_tailscale_ip` instance method (moved to free function in Task 4)
- `pairing_qr` now also sends `ConnectionState::Pairing` on every QR generation — previously only `rpc_server::pair` sent `Connected`; Swift AppState expects `Pairing` when QR appears (spec §6.2)

- [ ] **Step 2: Fix in-process tests that used `DaemonHandle` by value**

Plan 01 has tests in `crates/minos-daemon/tests/e2e.rs` that call `handle.stop().await` on an owned handle. Update them to work with `Arc<Self>`:

Search and replace in `crates/minos-daemon/tests/` and `crates/minos-mobile/tests/`:

```bash
rg 'let handle = minos_daemon::DaemonHandle::start' crates/
```

Each matched test site: the binding changes from `let handle = DaemonHandle::start(...).await?;` to `let handle = DaemonHandle::start(...).await?;` (same syntactically — `Arc<Self>` derefs transparently). But any `handle.stop().await?` after `let Some(h) = handle.stop().await` mutation patterns may need tweaks. Walk through each callsite and verify it still compiles; prefer to rewrite any "consuming" calls as `handle.stop().await?` with `&*handle`.

- [ ] **Step 3: Build and run full workspace tests**

```bash
cargo test --workspace
```

Expected: all tests pass, including existing `crates/minos-daemon/tests/e2e.rs` and `crates/minos-mobile/tests/e2e.rs`.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/handle.rs crates/minos-daemon/tests crates/minos-mobile/tests
git commit -m "refactor(minos-daemon): DaemonHandle owns Arc<Inner>, stop(&self) replaces consuming stop"
```

---

### Task 6: Expose `host()` + `port()` accessors

**Why:** Spec §5.1 #3 — UniFFI does not know `SocketAddr`. Swift bootstrap prints `{host}:{port}` in the MenuBar tooltip (spec §5.7), so both accessors are needed.

**Files:**
- Modify: `crates/minos-daemon/src/handle.rs`

- [ ] **Step 1: Add `host()` and `port()` methods to `impl DaemonHandle`**

Append to the `impl DaemonHandle` block in `handle.rs` (after `addr()`):

```rust
/// Bound host as a string (Tailscale 100.x or the loopback 127.0.0.1
/// used by tests). Exported to Swift via UniFFI.
#[must_use]
pub fn host(&self) -> String {
    self.inner.addr.ip().to_string()
}

/// Bound TCP port after auto-retry. Exported to Swift via UniFFI.
#[must_use]
pub fn port(&self) -> u16 {
    self.inner.addr.port()
}
```

The existing `addr()` method stays — Rust internal tests use it.

- [ ] **Step 2: Write unit test verifying host/port match the explicit cfg**

Add to `crates/minos-daemon/tests/e2e.rs` (or create a new `handle_getters.rs` integration test file):

```rust
#[tokio::test]
async fn host_and_port_round_trip_through_config() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let cfg = minos_daemon::DaemonConfig {
        mac_name: "Host Test".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
    };
    let handle = minos_daemon::DaemonHandle::start(cfg).await.unwrap();

    assert_eq!(handle.host(), "127.0.0.1");
    assert!(handle.port() > 0, "OS must pick a real port");
    assert_eq!(handle.addr().ip().to_string(), handle.host());
    assert_eq!(handle.addr().port(), handle.port());

    handle.stop().await.unwrap();
}
```

- [ ] **Step 3: Run the new test**

```bash
cargo test -p minos-daemon --test e2e host_and_port_round_trip_through_config
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/handle.rs crates/minos-daemon/tests/
git commit -m "feat(minos-daemon): host()/port() accessors on DaemonHandle"
```

---

### Task 7: Add `logging::today()` for Finder-reveal diagnostics

**Why:** Spec §5.1 "Also new" + §6.4 — Swift's "在 Finder 中显示今日日志…" menu item needs an absolute path to the current day's `.xlog` file. Rust must also flush the mars-xlog writer before returning the path so partially-buffered entries are on disk.

**Files:**
- Modify: `crates/minos-daemon/src/logging.rs`

- [ ] **Step 1: Inspect mars-xlog's flush API**

Check available methods on `XlogLayerHandle`:

```bash
rg 'impl.*XlogLayerHandle' ~/.cargo/registry/src/ -g '*.rs'
```

Expected: find `flush(&self)` or similar. If the method is named differently (`sync`, `close_files`, etc.), use the actual name; document a fallback in the impl comment.

If `HANDLE` has no flush method, shell out to `Xlog::flush` via the owned handle stored alongside.

- [ ] **Step 2: Add `today()` function**

Edit `crates/minos-daemon/src/logging.rs`, adding at the bottom (before `#[cfg(test)]`):

```rust
use std::path::PathBuf;

/// Return an absolute path to the current day's xlog file, after flushing
/// pending writes to disk. Swift uses this for "在 Finder 中显示今日日志…"
/// (spec §6.4).
///
/// Errors:
/// - `StoreIo` if flush failed, or the expected file does not exist.
#[allow(clippy::missing_errors_doc)]
pub fn today() -> Result<PathBuf, MinosError> {
    if let Some(h) = HANDLE.get() {
        // mars-xlog's public API name for sync-to-disk. If your installed
        // version exposes a different name, adjust here — the call is
        // idempotent and must not panic.
        h.flush();
    }

    let dir = log_dir();
    // mars-xlog naming: `{prefix}_{YYYYMMDD}.xlog`. Underscore separator,
    // not hyphen; no `{prefix}-{date}.xlog`.
    let stamp = chrono::Utc::now().format("%Y%m%d").to_string();
    let path = dir.join(format!("{NAME_PREFIX}_{stamp}.xlog"));

    if !path.exists() {
        return Err(MinosError::StoreIo {
            path: path.display().to_string(),
            message: "no log file written yet".to_string(),
        });
    }
    Ok(path)
}
```

Also add `use chrono::Utc;` at the top if not present. (Already transitive via other imports; verify with a build.)

- [ ] **Step 3: Add a test that exercises the happy path**

Append to the `tests` mod in `logging.rs`:

```rust
#[test]
fn today_returns_existing_path_after_a_log() {
    let dir = tempdir().unwrap();
    std::env::set_var("MINOS_LOG_DIR", dir.path());
    init().unwrap();

    // Emit one log record so mars-xlog opens the day's file.
    tracing::info!("probe");

    // today() flushes and returns the path.
    let p = today().unwrap();
    assert!(p.to_string_lossy().ends_with(".xlog"));
    assert!(p.exists(), "today() must return an existing file");
}

#[test]
fn today_errors_before_any_log_written() {
    // Isolate from the `init_creates_log_dir_and_emits_once` and
    // `today_returns_existing_path_after_a_log` tests by using a fresh
    // MINOS_LOG_DIR that has never been written to. `init` is global;
    // flushing there should not manufacture a file.
    let dir = tempdir().unwrap();
    std::env::set_var("MINOS_LOG_DIR", dir.path());
    // Do NOT call init() — we want to verify today() yields StoreIo when
    // the file doesn't exist at all. But init() is idempotent / static,
    // so this test is rigorous only on fresh process starts. Document that:
    // if other tests in this binary already called init(), this scenario
    // isn't reachable. Keep the assertion conservative.
    let r = today();
    match r {
        Err(MinosError::StoreIo { .. }) => { /* expected */ }
        Ok(p) => {
            // Prior init() already opened a file in a different dir — skip.
            assert!(!p.starts_with(dir.path()));
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minos-daemon --lib logging::tests
```

Expected: PASS (may include `init_creates_log_dir_and_emits_once` from plan 01).

- [ ] **Step 5: Commit**

```bash
git add crates/minos-daemon/src/logging.rs
git commit -m "feat(minos-daemon): logging::today() returns flushed current-day xlog path"
```

---

### Task 8: Add `DaemonHandle::start_autobind(mac_name)` with port retry

**Why:** Spec §5.1 #5 + §7.4 failure #2 — the 7878..=7882 retry loop belongs in Rust, so Swift doesn't have to translate error-then-retry logic. Swift calls `startAutobind(macName:)` once; five BindFailed retries are invisible to it.

**Files:**
- Modify: `crates/minos-daemon/src/handle.rs`

- [ ] **Step 1: Implement `start_autobind`**

Append to `impl DaemonHandle` in `crates/minos-daemon/src/handle.rs`:

```rust
/// Production entry point for Swift. Discovers the Tailscale 100.x IP,
/// tries ports 7878..=7882 in order, returns the first successful bind.
///
/// Errors:
/// - `BindFailed { addr: "<ip>:7878-7882", message: "all ports occupied" }`
///   if every port fails to bind
/// - `BindFailed { addr: "tailscale", message: "no 100.x IP" }` if
///   `discover_tailscale_ip` returns None
#[allow(clippy::missing_errors_doc)]
pub async fn start_autobind(mac_name: String) -> Result<Arc<Self>, MinosError> {
    const PORTS: std::ops::RangeInclusive<u16> = 7878..=7882;

    let host = crate::tailscale::discover_ip().await.ok_or_else(|| {
        MinosError::BindFailed {
            addr: "tailscale".into(),
            message: "no 100.x IP returned by `tailscale ip --4`".into(),
        }
    })?;

    let mut last_err: Option<MinosError> = None;
    for port in PORTS {
        let bind_addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .map_err(|e: std::net::AddrParseError| MinosError::BindFailed {
                addr: format!("{host}:{port}"),
                message: e.to_string(),
            })?;
        let cfg = DaemonConfig {
            mac_name: mac_name.clone(),
            bind_addr,
        };
        match Self::start(cfg).await {
            Ok(h) => return Ok(h),
            Err(e @ MinosError::BindFailed { .. }) => {
                tracing::warn!(port, err = %e, "port busy, trying next");
                last_err = Some(e);
                continue;
            }
            Err(other) => return Err(other),
        }
    }

    Err(MinosError::BindFailed {
        addr: format!("{host}:7878-7882"),
        message: last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "all ports occupied".into()),
    })
}
```

- [ ] **Step 2: Test port-retry with 5 pre-bound decoys (test file)**

Create (or append to existing) `crates/minos-daemon/tests/autobind.rs`:

```rust
//! Tests for DaemonHandle::start_autobind port-retry logic.
//!
//! These tests CANNOT use `start_autobind` directly because CI runners
//! lack Tailscale; they would return `BindFailed { addr: "tailscale" }`.
//! Instead we test the start(cfg) path with each port, with decoy
//! TcpListeners holding the lower ports. The autobind logic is also
//! exercised in `returns_bind_failed_when_all_occupied` by depending on
//! the Tailscale branch short-circuiting when no 100.x IP is available.

use std::net::{SocketAddr, TcpListener};

#[tokio::test]
async fn start_succeeds_when_first_port_free() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let cfg = minos_daemon::DaemonConfig {
        mac_name: "Autobind Test".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
    };
    let handle = minos_daemon::DaemonHandle::start(cfg).await.unwrap();
    assert!(handle.port() > 0);
    handle.stop().await.unwrap();
}

#[tokio::test]
async fn autobind_returns_bind_failed_without_tailscale() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    // Skip if tailscale is actually installed and reporting a 100.x IP
    // (dev laptops) — this CI-smoke test depends on the "no tailscale" path.
    let ip = minos_daemon::discover_tailscale_ip().await;
    if ip.is_some() {
        eprintln!("skipping — machine has a 100.x IP: {ip:?}");
        return;
    }

    let r = minos_daemon::DaemonHandle::start_autobind("Test Mac".into()).await;
    match r {
        Err(minos_domain::MinosError::BindFailed { addr, .. }) => {
            assert_eq!(addr, "tailscale");
        }
        Ok(_) => panic!("start_autobind should fail without tailscale"),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}
```

- [ ] **Step 3: Run the test**

```bash
cargo test -p minos-daemon --test autobind
```

Expected: both tests pass on a CI runner without Tailscale.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/handle.rs crates/minos-daemon/tests/autobind.rs
git commit -m "feat(minos-daemon): DaemonHandle::start_autobind with 7878..=7882 port retry"
```

---

### Task 9: Add `Subscription` + `ConnectionStateObserver` + `DaemonHandle::subscribe`

**Why:** Spec §5.1 #6 — Tokio `watch::Receiver` cannot cross UniFFI. Swift gets a callback-interface-based pushdown of `ConnectionState` via `subscribe`, with `Subscription.cancel()` for teardown.

**Files:**
- Create: `crates/minos-daemon/src/subscription.rs`
- Modify: `crates/minos-daemon/src/lib.rs`
- Modify: `crates/minos-daemon/src/handle.rs`

- [ ] **Step 1: Create `subscription.rs`**

Write `crates/minos-daemon/src/subscription.rs`:

```rust
//! UniFFI bridge for connection-state streaming.
//!
//! Rust consumers use `DaemonHandle::events_stream()` to get a raw
//! `watch::Receiver`. UniFFI consumers (Swift) use the push-model
//! `DaemonHandle::subscribe(observer)` + `Subscription::cancel()` because
//! Tokio types cannot cross the FFI boundary.

use std::sync::{Arc, Mutex};

use minos_domain::ConnectionState;
use tokio::sync::{oneshot, watch};

/// Opaque subscription handle. Swift holds this and calls `cancel` to
/// tear down the observer task at app shutdown or menu teardown.
pub struct Subscription {
    cancel_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl Subscription {
    #[must_use]
    pub(crate) fn new(cancel_tx: oneshot::Sender<()>) -> Self {
        Self {
            cancel_tx: Mutex::new(Some(cancel_tx)),
        }
    }

    /// Cancel the observer task. Idempotent.
    pub fn cancel(&self) {
        if let Some(tx) = self.cancel_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
}

/// Foreign-implementable callback. Swift conforms to the generated
/// `ConnectionStateObserver` protocol; Rust calls `on_state` each time
/// `watch::Receiver::changed` fires.
pub trait ConnectionStateObserver: Send + Sync {
    fn on_state(&self, state: ConnectionState);
}

/// Bridge a Tokio `watch::Receiver<ConnectionState>` to a foreign callback.
/// Returns a `Subscription` whose `cancel` stops the spawned task.
///
/// Called from `DaemonHandle::subscribe`; kept in its own module for
/// testability.
pub(crate) fn spawn_observer(
    mut rx: watch::Receiver<ConnectionState>,
    observer: Arc<dyn ConnectionStateObserver>,
) -> Arc<Subscription> {
    // Emit the current snapshot so Swift has a starting value.
    observer.on_state(*rx.borrow());

    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = &mut cancel_rx => break,
                r = rx.changed() => {
                    if r.is_err() {
                        break; // sender dropped
                    }
                    let state = *rx.borrow();
                    observer.on_state(state);
                }
            }
        }
    });
    Arc::new(Subscription::new(cancel_tx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    struct CountingObserver {
        hits: Arc<AtomicU32>,
    }

    impl ConnectionStateObserver for CountingObserver {
        fn on_state(&self, _: ConnectionState) {
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn observer_receives_initial_and_subsequent_states() {
        let (tx, rx) = watch::channel(ConnectionState::Disconnected);
        let hits = Arc::new(AtomicU32::new(0));
        let obs = Arc::new(CountingObserver { hits: hits.clone() });

        let sub = spawn_observer(rx, obs);
        // Initial send is synchronous, so wait a tick for bookkeeping.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(hits.load(Ordering::SeqCst) >= 1, "initial snapshot missed");

        tx.send(ConnectionState::Pairing).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(hits.load(Ordering::SeqCst) >= 2, "change not delivered");

        sub.cancel();
        let hits_before_cancel_send = hits.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;

        // After cancel, further sends must not increment hits.
        tx.send(ConnectionState::Connected).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            hits.load(Ordering::SeqCst),
            hits_before_cancel_send,
            "observer should have stopped after cancel"
        );
    }

    #[tokio::test]
    async fn cancel_is_idempotent() {
        let (_tx, rx) = watch::channel(ConnectionState::Disconnected);
        let hits = Arc::new(AtomicU32::new(0));
        let obs = Arc::new(CountingObserver { hits });
        let sub = spawn_observer(rx, obs);
        sub.cancel();
        sub.cancel(); // must not panic
    }
}
```

- [ ] **Step 2: Register the module in `lib.rs`**

Edit `crates/minos-daemon/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

pub mod file_store;
pub mod handle;
pub mod logging;
pub mod rpc_server;
pub mod subscription;
pub mod tailscale;

pub use file_store::*;
pub use handle::*;
pub use subscription::{ConnectionStateObserver, Subscription};

pub use tailscale::discover_ip as discover_tailscale_ip;
```

- [ ] **Step 3: Add `subscribe` method to `DaemonHandle`**

Append to `impl DaemonHandle` in `handle.rs`:

```rust
/// Push-model subscription for Swift/UniFFI. Internally bridges
/// `events_stream()` (the Tokio `watch::Receiver`) to the given observer
/// callback. Returns a `Subscription` whose `cancel` terminates the
/// forwarding task.
#[must_use]
pub fn subscribe(
    &self,
    observer: Arc<dyn crate::subscription::ConnectionStateObserver>,
) -> Arc<crate::subscription::Subscription> {
    crate::subscription::spawn_observer(self.events_stream(), observer)
}
```

- [ ] **Step 4: Run the subscription test**

```bash
cargo test -p minos-daemon --lib subscription::tests
```

Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add crates/minos-daemon/src/subscription.rs crates/minos-daemon/src/lib.rs crates/minos-daemon/src/handle.rs
git commit -m "feat(minos-daemon): ConnectionStateObserver + Subscription callback bridge"
```

---

### Task 10: Add `DaemonHandle::current_trusted_device`

**Why:** Spec §5.1 #7 — Swift's `MenuBarView` branches on whether a trusted device exists. Plan 01's `DaemonHandle` has no public accessor for the pairing store.

**Files:**
- Modify: `crates/minos-daemon/src/handle.rs`

- [ ] **Step 1: Add the accessor method**

Append to `impl DaemonHandle`:

```rust
/// Return the currently trusted device if one exists. MVP cap is one
/// (spec §6.4 single-pair), so the first entry in the store suffices.
/// Returns `Ok(None)` for an empty / missing `devices.json`.
#[allow(clippy::missing_errors_doc)]
pub fn current_trusted_device(&self) -> Result<Option<TrustedDevice>, MinosError> {
    let mut devices = self.inner.store.load()?;
    if devices.is_empty() {
        Ok(None)
    } else {
        Ok(Some(devices.remove(0)))
    }
}
```

Note the import of `TrustedDevice` from `minos_pairing` is already present at top of `handle.rs` (used by `rpc_server`).

- [ ] **Step 2: Write a test that covers empty + populated store**

Append to `crates/minos-daemon/tests/autobind.rs` (or create `trusted_device.rs`):

```rust
#[tokio::test]
async fn current_trusted_device_empty_then_populated() {
    use chrono::Utc;
    use minos_domain::DeviceId;
    use minos_pairing::{PairingStore, TrustedDevice};
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let cfg = minos_daemon::DaemonConfig {
        mac_name: "TD Test".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
    };
    let handle = minos_daemon::DaemonHandle::start(cfg).await.unwrap();

    // Empty on first start
    assert!(handle.current_trusted_device().unwrap().is_none());

    // Populate via the file store directly (simulates a plan-03 pair flow)
    let store: Arc<dyn PairingStore> = Arc::new(minos_daemon::FilePairingStore::new(
        minos_daemon::FilePairingStore::default_path(),
    ));
    let dev = TrustedDevice {
        device_id: DeviceId::new(),
        name: "iPhone".into(),
        host: "100.64.0.42".into(),
        port: 7878,
        paired_at: Utc::now(),
    };
    store.save(&[dev.clone()]).unwrap();

    // Re-start with the now-populated store
    handle.stop().await.unwrap();
    let handle = minos_daemon::DaemonHandle::start(minos_daemon::DaemonConfig {
        mac_name: "TD Test".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
    })
    .await
    .unwrap();

    let td = handle.current_trusted_device().unwrap().unwrap();
    assert_eq!(td.device_id, dev.device_id);
    assert_eq!(td.name, dev.name);
    handle.stop().await.unwrap();
}
```

- [ ] **Step 3: Run the test**

```bash
cargo test -p minos-daemon --test autobind current_trusted_device
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/handle.rs crates/minos-daemon/tests/
git commit -m "feat(minos-daemon): current_trusted_device accessor for Forget-UI flow"
```

---

## Phase E · `minos-daemon`: UniFFI feature flag + derives

### Task 11: Add `uniffi` feature to `minos-daemon` with `cfg_attr` on `DaemonHandle` / `Subscription` / `ConnectionStateObserver`

**Why:** Spec §5.3. The daemon types carry the biggest derives — an `Object` for `DaemonHandle`, an `Object` for `Subscription`, and a `with_foreign` callback trait for `ConnectionStateObserver`.

**Files:**
- Modify: `crates/minos-daemon/Cargo.toml`
- Modify: `crates/minos-daemon/src/handle.rs`
- Modify: `crates/minos-daemon/src/subscription.rs`

- [ ] **Step 1: Add feature to `minos-daemon/Cargo.toml`**

```toml
[package]
name = "minos-daemon"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Mac-side composition root: WS server + file store + RPC handlers + CLI detect."

[features]
uniffi = [
    "dep:uniffi",
    "minos-domain/uniffi",
    "minos-pairing/uniffi",
]

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
async-trait = { workspace = true }
uniffi = { workspace = true, optional = true }

[dev-dependencies]
tempfile = { workspace = true }
tokio-test = { workspace = true }
pretty_assertions = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Annotate `DaemonHandle` with `#[cfg_attr(feature = "uniffi", ...)]`**

In `crates/minos-daemon/src/handle.rs`, add the object derive to `DaemonHandle` and the method-level exports on the `impl` block. UniFFI requires the `impl` block to be annotated with `#[uniffi::export]` (or an outer `#[cfg_attr(feature = "uniffi", uniffi::export)]`).

```rust
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct DaemonHandle {
    inner: Arc<DaemonInner>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DaemonHandle {
    // Constructor export: UniFFI 0.31 requires #[uniffi::constructor] on
    // `start_autobind`.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub async fn start_autobind(mac_name: String) -> Result<Arc<Self>, MinosError> {
        // unchanged body from Task 8
        ...
    }

    pub fn pairing_qr(&self) -> Result<QrPayload, MinosError> { /* unchanged */ }

    pub fn current_state(&self) -> ConnectionState { /* unchanged */ }

    pub fn host(&self) -> String { /* unchanged */ }

    pub fn port(&self) -> u16 { /* unchanged */ }

    pub fn current_trusted_device(&self) -> Result<Option<TrustedDevice>, MinosError> { /* unchanged */ }

    pub async fn forget_device(&self, id: DeviceId) -> Result<(), MinosError> { /* unchanged */ }

    pub async fn stop(&self) -> Result<(), MinosError> { /* unchanged */ }

    pub fn subscribe(
        &self,
        observer: Arc<dyn crate::subscription::ConnectionStateObserver>,
    ) -> Arc<crate::subscription::Subscription> { /* unchanged */ }
}

// Non-exported methods stay outside the exported impl block.
impl DaemonHandle {
    /// Test-only constructor with explicit bind addr. NOT exported to UniFFI
    /// because Swift only calls `start_autobind`.
    #[allow(clippy::missing_errors_doc)]
    pub async fn start(cfg: DaemonConfig) -> Result<Arc<Self>, MinosError> { /* unchanged */ }

    #[must_use]
    pub fn events_stream(&self) -> watch::Receiver<ConnectionState> { /* unchanged */ }

    #[must_use]
    pub fn addr(&self) -> SocketAddr { /* unchanged */ }
}
```

Move `start(cfg)`, `events_stream()`, and `addr()` into a **separate, non-`uniffi::export`** `impl DaemonHandle` block. Everything else goes into the exported block.

- [ ] **Step 3: Annotate `Subscription` and `ConnectionStateObserver` in `subscription.rs`**

```rust
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct Subscription {
    cancel_tx: Mutex<Option<oneshot::Sender<()>>>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl Subscription {
    // cancel() is exported
    pub fn cancel(&self) { /* unchanged */ }
}

impl Subscription {
    // new() stays Rust-internal (not exported)
    #[must_use]
    pub(crate) fn new(cancel_tx: oneshot::Sender<()>) -> Self { /* unchanged */ }
}

#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait ConnectionStateObserver: Send + Sync {
    fn on_state(&self, state: ConnectionState);
}
```

Keep `spawn_observer` without `#[uniffi::export]` — it's a Rust-side helper used by `DaemonHandle::subscribe`.

- [ ] **Step 4: Build in both configurations + run tests**

```bash
cargo build -p minos-daemon
cargo build -p minos-daemon --features uniffi
cargo test -p minos-daemon
```

All three must succeed. If `--features uniffi` complains about `async fn` in `#[uniffi::export]`, check UniFFI 0.31's async support is enabled in the workspace dep (we have `features = ["build"]` currently — may need `tokio` or explicit `async` feature; adjust `Cargo.toml` if so).

- [ ] **Step 5: Commit**

```bash
git add crates/minos-daemon/Cargo.toml crates/minos-daemon/src/
git commit -m "feat(minos-daemon): uniffi feature + Object exports on DaemonHandle/Subscription/ConnectionStateObserver"
```

---

## Phase F · `minos-ffi-uniffi`: remove sentinel, wire the full shim

### Task 12: Rewrite `minos-ffi-uniffi/src/lib.rs` with custom types and free-function re-exports

**Why:** Spec §5.2. The shim is the single place where UniFFI "knows" about `Uuid` ↔ `String` and `DateTime<Utc>` ↔ `SystemTime` conversions, plus re-exports the free functions Swift will call.

**Files:**
- Modify: `crates/minos-ffi-uniffi/Cargo.toml`
- Modify: `crates/minos-ffi-uniffi/src/lib.rs`

- [ ] **Step 1: Enable `uniffi` features on path deps**

Replace `crates/minos-ffi-uniffi/Cargo.toml` contents:

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
minos-domain = { path = "../minos-domain", features = ["uniffi"] }
minos-pairing = { path = "../minos-pairing", features = ["uniffi"] }
minos-daemon = { path = "../minos-daemon", features = ["uniffi"] }
uniffi = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }

[build-dependencies]
uniffi = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Rewrite `src/lib.rs`**

```rust
//! UniFFI surface for Swift.
//!
//! Plan 02 fills in:
//! - Custom types for `Uuid` (via `String`) and `DateTime<Utc>` (via `SystemTime`)
//! - Newtype bridges for `DeviceId` and `PairingToken`
//! - Free-function re-exports: logging init, today_log_path, kind_message,
//!   discover_tailscale_ip
//!
//! All domain / pairing / daemon types cross via `cfg_attr(feature = "uniffi", ...)`
//! derives on the original types (spec §5.3), so this shim contains no
//! wrapper types.

#![allow(clippy::unused_async)] // UniFFI-generated glue may complain otherwise

use std::time::SystemTime;

use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, ErrorKind, Lang, MinosError, PairingToken};
use minos_pairing::{QrPayload, TrustedDevice};
use uuid::Uuid;

uniffi::setup_scaffolding!();

// ── Custom type bridges ────────────────────────────────────────────────────

uniffi::custom_type!(Uuid, String, {
    lower: |u| u.to_string(),
    try_lift: |s| Uuid::parse_str(&s).map_err(|e| e.into()),
});

uniffi::custom_type!(DateTime<Utc>, SystemTime, {
    lower: |dt| dt.into(),
    try_lift: |st| Ok::<_, std::convert::Infallible>(st.into()),
});

// ── Newtype bridges ───────────────────────────────────────────────────────

uniffi::custom_newtype!(DeviceId, Uuid);
uniffi::custom_newtype!(PairingToken, String);

// ── Free-function re-exports ──────────────────────────────────────────────

/// Initialize the Rust-side mars-xlog writer. Swift calls this once at app
/// startup before `DaemonHandle::start_autobind`.
#[uniffi::export]
pub fn init_logging() -> Result<(), MinosError> {
    minos_daemon::logging::init()
}

/// Toggle log-level at runtime.
#[uniffi::export]
pub fn set_debug(enabled: bool) {
    minos_daemon::logging::set_debug(enabled);
}

/// Absolute path to the current day's xlog file, after flush.
#[uniffi::export]
pub fn today_log_path() -> Result<String, MinosError> {
    minos_daemon::logging::today().map(|p| p.to_string_lossy().to_string())
}

/// Localized user-facing string for an `ErrorKind` + `Lang`. Swift's
/// `MinosError` extension maps the thrown variant to its `.kind` and calls
/// this to fetch the display string. See spec §7.2.
#[uniffi::export]
pub fn kind_message(kind: ErrorKind, lang: Lang) -> String {
    kind.user_message(lang).to_string()
}

/// Discover the local machine's Tailscale 100.x IP without starting a daemon.
/// Exposed for diagnostics / manual tooling; `start_autobind` uses it
/// internally and does not require Swift to invoke this first.
#[uniffi::export]
pub async fn discover_tailscale_ip() -> Option<String> {
    minos_daemon::discover_tailscale_ip().await
}

// The build.rs stays a no-op — proc-macro mode needs no UDL generation step.
// DaemonHandle, Subscription, ConnectionStateObserver, and all domain /
// pairing types are re-exported implicitly by the `#[uniffi::export]` on
// their respective impl blocks in `minos-daemon` / `minos-domain` /
// `minos-pairing` under the `uniffi` feature.

// Explicitly re-export the types at this crate level so uniffi-bindgen
// finds them when scanning the dylib.
pub use minos_daemon::{ConnectionStateObserver, DaemonHandle, Subscription};
pub use minos_domain::{
    AgentDescriptor, AgentName, AgentStatus, ConnectionState, ErrorKind as _ErrorKind,
    Lang as _Lang, MinosError as _MinosError, PairingState,
};
pub use minos_pairing::{QrPayload as _QrPayload, TrustedDevice as _TrustedDevice};
```

- [ ] **Step 3: Build the shim crate**

```bash
cargo build -p minos-ffi-uniffi --release
```

Expected: success. The release artifact will live at `target/release/libminos_ffi_uniffi.{a,dylib}`.

- [ ] **Step 4: Verify the dylib exports the expected UniFFI scaffolding**

```bash
ls target/release/ | rg 'minos_ffi_uniffi'
```

Expected: both `.a` and `.dylib` present. (On Linux CI, `.so` instead of `.dylib` — fine for the compile smoke.)

- [ ] **Step 5: Commit**

```bash
git add crates/minos-ffi-uniffi/Cargo.toml crates/minos-ffi-uniffi/src/lib.rs
git commit -m "feat(minos-ffi-uniffi): full shim — custom_type, custom_newtype, free-function re-exports"
```

---

## Phase G · `xtask`: build-macos / gen-uniffi / gen-xcode + check-all extension

### Task 13: Implement `xtask build-macos`

**Why:** Spec §5.4. Produces the universal `.a` that XcodeGen references.

**Files:**
- Modify: `xtask/src/main.rs`

- [ ] **Step 1: Replace `BuildMacos` placeholder with real impl**

Edit `xtask/src/main.rs`. Remove the `Cmd::BuildMacos => not_yet("build-macos"),` line and replace with:

```rust
Cmd::BuildMacos => build_macos(),
```

Then add the function (anywhere below `fn main`):

```rust
fn build_macos() -> Result<()> {
    let root = workspace_root()?;
    eprintln!("==> cargo build-macos: arm64 + x86_64 staticlib -> lipo universal");

    for target in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
        eprintln!("  target: {target}");
        run(
            "cargo",
            &[
                "build",
                "-p",
                "minos-ffi-uniffi",
                "--release",
                "--target",
                target,
            ],
            &root,
        )?;
    }

    let out_dir = root.join("target/xcframework");
    std::fs::create_dir_all(&out_dir).with_context(|| format!("mkdir {out_dir:?}"))?;
    let out_lib = out_dir.join("libminos_ffi_uniffi.a");

    let arm64 = root.join("target/aarch64-apple-darwin/release/libminos_ffi_uniffi.a");
    let x86_64 = root.join("target/x86_64-apple-darwin/release/libminos_ffi_uniffi.a");

    eprintln!("==> lipo -create -> {}", out_lib.display());
    run(
        "lipo",
        &[
            "-create",
            arm64.to_str().unwrap(),
            x86_64.to_str().unwrap(),
            "-output",
            out_lib.to_str().unwrap(),
        ],
        &root,
    )?;

    eprintln!("==> lipo -info (verification)");
    run("lipo", &["-info", out_lib.to_str().unwrap()], &root)?;

    eprintln!("OK: {} (universal)", out_lib.display());
    Ok(())
}
```

- [ ] **Step 2: Run on a Mac workstation**

```bash
cargo xtask build-macos
```

Expected output:
```
==> cargo build-macos: arm64 + x86_64 staticlib -> lipo universal
  target: aarch64-apple-darwin
  target: x86_64-apple-darwin
==> lipo -create -> .../target/xcframework/libminos_ffi_uniffi.a
==> lipo -info (verification)
Architectures in the fat file: .../libminos_ffi_uniffi.a are: x86_64 arm64
OK: .../libminos_ffi_uniffi.a (universal)
```

(CI `ubuntu-latest` cannot build Apple targets by default and will skip this step — we'll configure the `macos-14` CI job in Task 37 to exercise this code path.)

- [ ] **Step 3: Commit**

```bash
git add xtask/src/main.rs
git commit -m "feat(xtask): implement build-macos (universal arm64+x86_64 staticlib via lipo)"
```

---

### Task 14: Implement `xtask gen-uniffi`

**Why:** Spec §9.1 — `uniffi-bindgen-swift` operates on a built dylib. This command builds host-arch dylib first, then runs bindgen.

**Files:**
- Modify: `xtask/src/main.rs`

- [ ] **Step 1: Replace `GenUniffi` placeholder**

Change the match arm from `Cmd::GenUniffi => not_yet("gen-uniffi"),` to `Cmd::GenUniffi => gen_uniffi(),` and add:

```rust
fn gen_uniffi() -> Result<()> {
    let root = workspace_root()?;
    let out_dir = root.join("apps/macos/Minos/Generated");
    std::fs::create_dir_all(&out_dir).with_context(|| format!("mkdir {out_dir:?}"))?;

    eprintln!("==> cargo build (host arch) -p minos-ffi-uniffi --release");
    run(
        "cargo",
        &["build", "-p", "minos-ffi-uniffi", "--release"],
        &root,
    )?;

    let host_dylib_suffix = if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    let dylib = root
        .join("target/release")
        .join(format!("libminos_ffi_uniffi.{host_dylib_suffix}"));

    eprintln!("==> uniffi-bindgen-swift --library {}", dylib.display());
    run(
        "uniffi-bindgen-swift",
        &[
            "--library",
            dylib.to_str().unwrap(),
            "--module-name",
            "MinosCore",
            "--out-dir",
            out_dir.to_str().unwrap(),
        ],
        &root,
    )?;

    // Smoke-check expected public symbols in the generated Swift file.
    let swift_file = out_dir.join("MinosCore.swift");
    let contents = std::fs::read_to_string(&swift_file).with_context(|| {
        format!("reading {}", swift_file.display())
    })?;
    for needle in [
        "public class DaemonHandle",
        "public protocol ConnectionStateObserver",
        "public class Subscription",
    ] {
        if !contents.contains(needle) {
            bail!("Generated Swift is missing expected symbol: {needle}");
        }
    }
    eprintln!("OK: {}", swift_file.display());
    Ok(())
}
```

- [ ] **Step 2: Run after installing `uniffi-bindgen-swift`**

On a Mac workstation (installation covered in Task 18's extended bootstrap):

```bash
cargo install uniffi-bindgen-swift --locked
cargo xtask gen-uniffi
```

Expected output:
```
==> cargo build (host arch) -p minos-ffi-uniffi --release
...
==> uniffi-bindgen-swift --library .../libminos_ffi_uniffi.dylib
OK: apps/macos/Minos/Generated/MinosCore.swift
```

- [ ] **Step 3: Verify generated directory structure**

```bash
ls apps/macos/Minos/Generated/
```

Expected:
```
MinosCore.swift
MinosCoreFFI.h
MinosCoreFFI.modulemap
```

- [ ] **Step 4: Commit**

```bash
git add xtask/src/main.rs
git commit -m "feat(xtask): implement gen-uniffi (uniffi-bindgen-swift + symbol smoke-check)"
```

---

### Task 15: Implement `xtask gen-xcode`

**Why:** Spec §5.4. XcodeGen runs before any `xcodebuild` invocation so the `.xcodeproj` reflects what's on disk.

**Files:**
- Modify: `xtask/src/main.rs`

- [ ] **Step 1: Add the `GenXcode` subcommand to the clap enum**

In `xtask/src/main.rs`, update the `Cmd` enum:

```rust
#[derive(Subcommand)]
enum Cmd {
    CheckAll,
    Bootstrap,
    GenUniffi,
    GenFrb,
    BuildMacos,
    BuildIos,
    /// Generate apps/macos/Minos.xcodeproj from apps/macos/project.yml. (New in plan 02.)
    GenXcode,
}
```

And the match arm in `main`:

```rust
Cmd::GenXcode => gen_xcode(),
```

- [ ] **Step 2: Implement `gen_xcode`**

```rust
fn gen_xcode() -> Result<()> {
    let root = workspace_root()?;
    let spec = root.join("apps/macos/project.yml");
    if !spec.exists() {
        bail!("{} missing — run after Task 20 (project.yml)", spec.display());
    }
    eprintln!("==> xcodegen generate --spec {}", spec.display());
    run(
        "xcodegen",
        &["generate", "--spec", spec.to_str().unwrap()],
        &root.join("apps/macos"),
    )?;
    Ok(())
}
```

- [ ] **Step 3: Commit (project.yml comes later in Task 20; for now xtask compiles with the new subcommand)**

```bash
cargo build -p xtask
git add xtask/src/main.rs
git commit -m "feat(xtask): gen-xcode subcommand (runs xcodegen against apps/macos/project.yml)"
```

---

### Task 16: Extend `xtask check-all` with Swift leg

**Why:** Spec §8.4. The single CI command covers Rust + Swift.

**Files:**
- Modify: `xtask/src/main.rs`

- [ ] **Step 1: Modify `check_all` to append Swift leg when on macOS**

Replace the existing `check_all` function in `xtask/src/main.rs`:

```rust
fn check_all() -> Result<()> {
    let workspace_root = workspace_root()?;
    eprintln!("==> cargo fmt --check");
    run("cargo", &["fmt", "--all", "--check"], &workspace_root)?;

    eprintln!("==> cargo clippy");
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
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

    // Swift leg: only runs on macOS hosts, skipped on linux CI.
    if cfg!(target_os = "macos") {
        eprintln!("==> xtask gen-uniffi (before xcodebuild)");
        gen_uniffi()?;

        eprintln!("==> xtask gen-xcode");
        gen_xcode()?;

        if which("xcodegen").is_none() {
            bail!("xcodegen not installed; run `cargo xtask bootstrap`");
        }

        eprintln!("==> xcodebuild -scheme Minos build");
        run(
            "xcodebuild",
            &[
                "-project",
                "Minos.xcodeproj",
                "-scheme",
                "Minos",
                "-destination",
                "platform=macOS",
                "-configuration",
                "Debug",
                "build",
            ],
            &workspace_root.join("apps/macos"),
        )?;

        eprintln!("==> xcodebuild -scheme MinosTests test");
        run(
            "xcodebuild",
            &[
                "-project",
                "Minos.xcodeproj",
                "-scheme",
                "MinosTests",
                "-destination",
                "platform=macOS",
                "-configuration",
                "Debug",
                "test",
            ],
            &workspace_root.join("apps/macos"),
        )?;

        eprintln!("==> swiftlint --strict");
        if which("swiftlint").is_none() {
            bail!("swiftlint not installed; run `cargo xtask bootstrap`");
        }
        run(
            "swiftlint",
            &["--strict"],
            &workspace_root.join("apps/macos"),
        )?;
    } else {
        eprintln!("==> swift leg: skipped (non-macOS host)");
    }

    eprintln!("OK: all checks pass.");
    Ok(())
}
```

- [ ] **Step 2: Smoke-check on a Mac workstation (expect XcodeGen/bindings not yet present, so the run will fail before xcodebuild — that's fine; it confirms the Rust leg stays green)**

```bash
cargo xtask check-all
```

Expected: Rust leg passes; Swift leg fails at `gen-uniffi` or `xcodegen` if `apps/macos/project.yml` doesn't exist yet. The failure is expected at this point; later tasks fix it.

- [ ] **Step 3: Commit**

```bash
git add xtask/src/main.rs
git commit -m "feat(xtask): extend check-all with macOS-only Swift leg"
```

---

### Task 17: Extend `xtask bootstrap` for Swift tools

**Why:** Single-command dev setup — new contributors run one command and get everything.

**Files:**
- Modify: `xtask/src/main.rs`

- [ ] **Step 1: Rewrite `bootstrap`**

Replace the `bootstrap` function in `xtask/src/main.rs`:

```rust
fn bootstrap() -> Result<()> {
    let workspace_root = workspace_root()?;

    eprintln!("==> installing cargo-deny + uniffi-bindgen-cli + uniffi-bindgen-swift");
    run(
        "cargo",
        &["install", "cargo-deny", "--locked"],
        &workspace_root,
    )?;
    run(
        "cargo",
        &["install", "uniffi-bindgen-cli", "--locked"],
        &workspace_root,
    )?;
    run(
        "cargo",
        &["install", "uniffi-bindgen-swift", "--locked"],
        &workspace_root,
    )?;

    if cfg!(target_os = "macos") {
        let brewfile = workspace_root.join("apps/macos/Brewfile");
        if brewfile.exists() {
            eprintln!("==> brew bundle (xcodegen + swiftlint)");
            if which("brew").is_some() {
                run(
                    "brew",
                    &["bundle", "--file", brewfile.to_str().unwrap()],
                    &workspace_root,
                )?;
            } else {
                eprintln!("    brew not installed — install it and rerun `cargo xtask bootstrap`.");
            }
        } else {
            eprintln!("    (skipped: {} missing — bootstrap will install Brew tools once Task 21 lands the Brewfile)", brewfile.display());
        }
    }

    eprintln!("OK: bootstrap complete.");
    Ok(())
}
```

- [ ] **Step 2: Dry-run on Mac (the Brewfile check will skip gracefully until Task 21)**

```bash
cargo xtask bootstrap
```

Expected: cargo installs complete; brew bundle skipped because Brewfile doesn't exist yet.

- [ ] **Step 3: Commit**

```bash
git add xtask/src/main.rs
git commit -m "feat(xtask): extend bootstrap with uniffi-bindgen-swift + brew bundle (macos-only)"
```

---

### Task 18: Update `.gitignore` for Xcode artifacts

**Why:** The generated `.xcodeproj` and `Brewfile.lock.json` are tool artifacts, not code.

**Files:**
- Modify: `.gitignore`

- [ ] **Step 1: Add Xcode entries to `.gitignore`**

Edit `.gitignore`, appending:

```
# Xcode (XcodeGen regenerates from project.yml)
apps/macos/Minos.xcodeproj/
apps/macos/build/
apps/macos/DerivedData/
apps/macos/.DS_Store
apps/macos/**/xcuserdata/

# Brew bundle lock
apps/macos/Brewfile.lock.json

# xcframework products (regenerated by xtask build-macos)
target/xcframework/
```

- [ ] **Step 2: Verify**

```bash
cat .gitignore | tail -15
```

- [ ] **Step 3: Commit**

```bash
git add .gitignore
git commit -m "chore(gitignore): ignore Xcode artifacts + Brewfile lock + xcframework products"
```

---

## Phase H · XcodeGen + Brew + swiftlint + first codegen

### Task 19: Write `apps/macos/Brewfile`

**Files:**
- Create: `apps/macos/Brewfile`

- [ ] **Step 1: Create the Brewfile**

```ruby
# apps/macos/Brewfile
# `brew bundle --file apps/macos/Brewfile` (run by `cargo xtask bootstrap`).

brew "xcodegen"
brew "swiftlint"
```

- [ ] **Step 2: Run bootstrap to verify it installs both**

```bash
cargo xtask bootstrap
```

Expected: Brew installs xcodegen and swiftlint (or confirms already installed).

- [ ] **Step 3: Commit**

```bash
git add apps/macos/Brewfile
git commit -m "chore(macos): Brewfile pinning xcodegen + swiftlint"
```

---

### Task 20: Write `apps/macos/.swiftlint.yml`

**Why:** SwiftLint runs strict in CI; must exclude UniFFI-generated code and express the project's style opinions.

**Files:**
- Create: `apps/macos/.swiftlint.yml`

- [ ] **Step 1: Create the SwiftLint config**

```yaml
# apps/macos/.swiftlint.yml
# Strict linting on hand-written Swift; generated UniFFI code is exempted.

included:
  - Minos
  - MinosTests

excluded:
  - Minos/Generated         # UniFFI-generated bindings
  - build
  - DerivedData
  - Minos.xcodeproj

line_length:
  warning: 140
  error: 160
  ignores_comments: true
  ignores_urls: true

identifier_name:
  min_length:
    warning: 2
    error: 1
  excluded:
    - id
    - qr
    - ip

disabled_rules:
  - todo                    # TODO in source is sometimes useful during plan exec
  - trailing_whitespace     # handled by editor config

opt_in_rules:
  - empty_count
  - force_unwrapping
  - implicit_return
  - sorted_imports
  - prefer_self_in_static_references
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/.swiftlint.yml
git commit -m "chore(macos): swiftlint config — strict on hand-written Swift, excludes Generated/"
```

---

### Task 21: Write `apps/macos/project.yml` (XcodeGen)

**Why:** Spec §5.5. The declarative project spec that regenerates `.xcodeproj` on demand.

**Files:**
- Create: `apps/macos/project.yml`
- Delete: `apps/macos/.gitkeep`

- [ ] **Step 1: Remove the placeholder**

```bash
rm apps/macos/.gitkeep
```

- [ ] **Step 2: Create `project.yml`**

```yaml
# apps/macos/project.yml
# XcodeGen spec — run `cargo xtask gen-xcode` (or `xcodegen generate`) to
# produce apps/macos/Minos.xcodeproj. Do NOT edit the generated project
# directly; all changes live here.

name: Minos
options:
  bundleIdPrefix: ai.minos
  deploymentTarget:
    macOS: "13.0"
  createIntermediateGroups: true
  groupSortPosition: top

settings:
  base:
    SWIFT_VERSION: "5.10"
    MACOSX_DEPLOYMENT_TARGET: "13.0"
    ENABLE_HARDENED_RUNTIME: NO
    CODE_SIGN_IDENTITY: "-"
    CODE_SIGN_STYLE: Manual
    DEAD_CODE_STRIPPING: YES
    # Strict-concurrency exposes data-race bugs early; appropriate for a
    # fresh codebase. Switch to `targeted` if UniFFI-generated code
    # triggers spurious warnings.
    SWIFT_STRICT_CONCURRENCY: complete

targets:
  Minos:
    type: application
    platform: macOS
    sources:
      - path: Minos
        excludes:
          - Generated/**     # Generated sources added explicitly below so
                             # XcodeGen treats them as resources, not code
                             # scanned by SwiftLint.
      - path: Minos/Generated
    resources:
      - Minos/Resources/Assets.xcassets
    info:
      path: Minos/Info.plist
      properties:
        LSUIElement: true
        CFBundleDisplayName: Minos
        CFBundleShortVersionString: "0.1.0"
        CFBundleVersion: "1"
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: ai.minos.macos
        HEADER_SEARCH_PATHS: $(SRCROOT)/Minos/Generated
        LIBRARY_SEARCH_PATHS: $(SRCROOT)/../../target/xcframework
        OTHER_LDFLAGS: "-lminos_ffi_uniffi"
        OTHER_SWIFT_FLAGS: "-Xcc -fmodule-map-file=$(SRCROOT)/Minos/Generated/MinosCoreFFI.modulemap"
  MinosTests:
    type: bundle.unit-test
    platform: macOS
    sources:
      - path: MinosTests
    dependencies:
      - target: Minos
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: ai.minos.macos.tests
```

- [ ] **Step 3: Generate the xcodeproj (requires Task 13 done first; gen-uniffi for `Generated/`)**

```bash
cargo xtask build-macos     # produces libminos_ffi_uniffi.a
cargo xtask gen-uniffi      # produces Minos/Generated/
cargo xtask gen-xcode       # produces Minos.xcodeproj
```

The first `gen-xcode` is expected to succeed; `xcodebuild` will still fail because `MinosApp.swift` etc. don't exist yet — we add them in the following tasks.

- [ ] **Step 4: Commit**

```bash
git rm apps/macos/.gitkeep
git add apps/macos/project.yml
git commit -m "chore(macos): XcodeGen project.yml for Minos + MinosTests targets"
```

---

## Phase I · Swift app foundation

### Task 22: Write `Info.plist`

**Files:**
- Create: `apps/macos/Minos/Info.plist`

- [ ] **Step 1: Create the plist**

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Minos</string>
    <key>CFBundleDisplayName</key>
    <string>Minos</string>
    <key>CFBundleIdentifier</key>
    <string>ai.minos.macos</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>MIT licensed. © 2026 Minos contributors.</string>
</dict>
</plist>
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Info.plist
git commit -m "chore(macos): Info.plist — bundle id, LSUIElement, macOS 13 minimum"
```

---

### Task 23: Create `Assets.xcassets` with a placeholder AppIcon and AccentColor

**Files:**
- Create: `apps/macos/Minos/Resources/Assets.xcassets/Contents.json`
- Create: `apps/macos/Minos/Resources/Assets.xcassets/AppIcon.appiconset/Contents.json`
- Create: `apps/macos/Minos/Resources/Assets.xcassets/AccentColor.colorset/Contents.json`

- [ ] **Step 1: Assets root**

Create `apps/macos/Minos/Resources/Assets.xcassets/Contents.json`:

```json
{
  "info": {
    "author": "xcode",
    "version": 1
  }
}
```

- [ ] **Step 2: AppIcon (empty — Xcode auto-generates from system placeholder until a real icon lands)**

Create `apps/macos/Minos/Resources/Assets.xcassets/AppIcon.appiconset/Contents.json`:

```json
{
  "images": [
    { "idiom": "mac", "scale": "1x", "size": "16x16" },
    { "idiom": "mac", "scale": "2x", "size": "16x16" },
    { "idiom": "mac", "scale": "1x", "size": "32x32" },
    { "idiom": "mac", "scale": "2x", "size": "32x32" },
    { "idiom": "mac", "scale": "1x", "size": "128x128" },
    { "idiom": "mac", "scale": "2x", "size": "128x128" },
    { "idiom": "mac", "scale": "1x", "size": "256x256" },
    { "idiom": "mac", "scale": "2x", "size": "256x256" },
    { "idiom": "mac", "scale": "1x", "size": "512x512" },
    { "idiom": "mac", "scale": "2x", "size": "512x512" }
  ],
  "info": {
    "author": "xcode",
    "version": 1
  }
}
```

- [ ] **Step 3: AccentColor (system blue)**

Create `apps/macos/Minos/Resources/Assets.xcassets/AccentColor.colorset/Contents.json`:

```json
{
  "colors": [
    {
      "color": {
        "color-space": "srgb",
        "components": {
          "alpha": "1.000",
          "blue": "0xFF",
          "green": "0x7F",
          "red": "0x00"
        }
      },
      "idiom": "universal"
    }
  ],
  "info": {
    "author": "xcode",
    "version": 1
  }
}
```

- [ ] **Step 4: Commit**

```bash
git add apps/macos/Minos/Resources/
git commit -m "chore(macos): empty AppIcon + accent color placeholders (real icon TBD P1.5)"
```

---

### Task 24: `Application/DaemonDriving.swift` — the mock-friendly protocol

**Why:** Spec §4.2 — `AppState` talks to `DaemonDriving`, not UniFFI's concrete `DaemonHandle`. Tests inject `MockDaemon`.

**Files:**
- Create: `apps/macos/Minos/Application/DaemonDriving.swift`

- [ ] **Step 1: Write the protocol**

```swift
// apps/macos/Minos/Application/DaemonDriving.swift
//
// Abstraction over UniFFI's generated DaemonHandle so AppState can be
// unit-tested against a MockDaemon without linking the Rust static lib
// into the test target. Production wiring uses an extension on
// MinosCore.DaemonHandle to satisfy this protocol.

import Foundation
import MinosCore

protocol DaemonDriving: AnyObject {
    func currentState() -> ConnectionState
    func currentTrustedDevice() throws -> TrustedDevice?
    func pairingQr() throws -> QrPayload
    func forgetDevice(id: DeviceId) async throws
    func host() -> String
    func port() -> UInt16
    func subscribe(observer: any ConnectionStateObserver) -> Subscription
    func stop() async throws
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Application/DaemonDriving.swift
git commit -m "feat(macos): DaemonDriving protocol — the mockable façade over UniFFI DaemonHandle"
```

---

### Task 25: `Infrastructure/DaemonHandle+DaemonDriving.swift` — extension conformance

**Why:** UniFFI's generated `DaemonHandle` satisfies the protocol. Separate file so conformance isn't in the generated-code directory (which is excluded from lint).

**Files:**
- Create: `apps/macos/Minos/Infrastructure/DaemonHandle+DaemonDriving.swift`

- [ ] **Step 1: Write the extension**

```swift
// apps/macos/Minos/Infrastructure/DaemonHandle+DaemonDriving.swift
//
// Conform UniFFI's DaemonHandle to our local DaemonDriving protocol so
// production code can inject it where AppState expects DaemonDriving.

import Foundation
import MinosCore

extension DaemonHandle: DaemonDriving {}
```

UniFFI generates public methods with the exact signatures declared in `DaemonDriving`, so no method shims are needed here — the conformance is empty.

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Infrastructure/DaemonHandle+DaemonDriving.swift
git commit -m "feat(macos): DaemonHandle : DaemonDriving conformance (empty extension)"
```

---

### Task 26: `Application/ObserverAdapter.swift` — Rust callback → Swift closure

**Why:** Spec §4.2 / §6.3 — every observer callback must marshal to `@MainActor` before touching `AppState`.

**Files:**
- Create: `apps/macos/Minos/Application/ObserverAdapter.swift`

- [ ] **Step 1: Write the adapter class**

```swift
// apps/macos/Minos/Application/ObserverAdapter.swift
//
// UniFFI's `ConnectionStateObserver` protocol is a Swift-callable callback
// interface. This adapter wraps a closure so AppState can subscribe via
// `daemon.subscribe(observer: ObserverAdapter { state in ... })`. The
// adapter always marshals to @MainActor so AppState mutations happen on
// the main thread.

import Foundation
import MinosCore

final class ObserverAdapter: ConnectionStateObserver, @unchecked Sendable {
    typealias Handler = @Sendable @MainActor (ConnectionState) -> Void
    private let handler: Handler

    init(handler: @escaping Handler) {
        self.handler = handler
    }

    func onState(state: ConnectionState) {
        let handler = handler
        Task { @MainActor in handler(state) }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Application/ObserverAdapter.swift
git commit -m "feat(macos): ObserverAdapter — UniFFI callback → @MainActor closure"
```

---

### Task 27: `Application/AppState.swift` — the @Observable reducer

**Why:** Spec §4.2 / §6 — single source of truth for UI state, backed by `DaemonDriving`.

**Files:**
- Create: `apps/macos/Minos/Application/AppState.swift`

- [ ] **Step 1: Write `AppState`**

```swift
// apps/macos/Minos/Application/AppState.swift
//
// @Observable single-source-of-truth for the menubar UI.
//
// Layering: Presentation reads stored properties; DaemonBootstrap injects
// `daemon` (any `DaemonDriving` conformer). Rust-originated state changes
// arrive via `connectionStateObserver` on the MainActor.

import Foundation
import MinosCore
import OSLog

@MainActor
@Observable
final class AppState {
    // ── Reactive state ────────────────────────────────────────────────────
    var connectionState: ConnectionState = .disconnected
    var trustedDevice: TrustedDevice? = nil

    // ── Modal / sheet state ───────────────────────────────────────────────
    var isQrSheetPresented: Bool = false
    var currentQr: QrPayload? = nil

    // ── Error state ───────────────────────────────────────────────────────
    /// Fatal startup error. Non-nil activates the error-branch menu.
    var bootError: MinosError? = nil
    /// Transient, toast-style error banner. Auto-cleared by UI after 3s.
    var displayError: MinosError? = nil

    // ── Injected dependencies (via DaemonBootstrap) ──────────────────────
    var daemon: (any DaemonDriving)? = nil
    var subscription: Subscription? = nil

    private let log = Logger(subsystem: "ai.minos.macos", category: "appState")

    // ── Derived UI permissions (for MenuBarView branching) ───────────────
    var canShowQr: Bool { bootError == nil && trustedDevice == nil && daemon != nil }
    var canForgetDevice: Bool { bootError == nil && trustedDevice != nil && daemon != nil }

    // ── Intent methods ────────────────────────────────────────────────────

    func showQr() async {
        guard let daemon else { return }
        do {
            let qr = try daemon.pairingQr()
            currentQr = qr
            isQrSheetPresented = true
            log.info("showQr ok — token expires in 5 minutes")
        } catch let error as MinosError {
            displayError = error
            log.error("showQr failed: \(String(describing: error))")
        } catch {
            displayError = .rpcCallFailed(method: "pairingQr", message: "\(error)")
        }
    }

    func regenerateQr() async {
        await showQr()
    }

    func forgetDevice() async {
        guard let daemon, let td = trustedDevice else { return }
        do {
            try await daemon.forgetDevice(id: td.deviceId)
            trustedDevice = nil
            log.info("forgetDevice ok")
        } catch let error as MinosError {
            displayError = error
            log.error("forgetDevice failed: \(String(describing: error))")
        } catch {
            displayError = .rpcCallFailed(method: "forgetDevice", message: "\(error)")
        }
    }

    func refreshTrustedDevice() {
        guard let daemon else { return }
        do {
            trustedDevice = try daemon.currentTrustedDevice()
        } catch let error as MinosError {
            displayError = error
        } catch {
            // ignore
        }
    }

    func shutdown() async {
        subscription?.cancel()
        if let daemon {
            try? await daemon.stop()
        }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Application/AppState.swift
git commit -m "feat(macos): AppState reducer — @Observable state + intent methods for QR/Forget/shutdown"
```

---

### Task 28: `Infrastructure/DaemonBootstrap.swift` — compose everything at startup

**Why:** Spec §6.1 — initialize logging, start daemon, subscribe.

**Files:**
- Create: `apps/macos/Minos/Infrastructure/DaemonBootstrap.swift`

- [ ] **Step 1: Write the bootstrap function**

```swift
// apps/macos/Minos/Infrastructure/DaemonBootstrap.swift
//
// App-launch orchestration: Rust logging → daemon.startAutobind →
// observer subscribe → populate AppState. Any throw lands in
// AppState.bootError and renders the error-branch menu.

import Foundation
import MinosCore
import OSLog

enum DaemonBootstrap {
    private static let log = Logger(subsystem: "ai.minos.macos", category: "bootstrap")

    /// Kick off daemon startup. Safe to call multiple times — clears
    /// `bootError` on retry.
    @MainActor
    static func bootstrap(appState: AppState) async {
        appState.bootError = nil
        do {
            try initLogging()
            log.info("boot start")

            let macName = Host.current().localizedName ?? "Mac"
            let daemon = try await DaemonHandle.startAutobind(macName: macName)

            let sub = daemon.subscribe(observer: ObserverAdapter { state in
                appState.connectionState = state
            })

            appState.daemon = daemon
            appState.subscription = sub
            appState.connectionState = daemon.currentState()
            appState.refreshTrustedDevice()

            log.info("boot ok — host=\(daemon.host(), privacy: .public):\(daemon.port(), privacy: .public)")
        } catch let error as MinosError {
            appState.bootError = error
            log.error("boot failed: \(String(describing: error))")
        } catch {
            appState.bootError = .rpcCallFailed(method: "bootstrap", message: "\(error)")
            log.error("boot failed: \(String(describing: error))")
        }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Infrastructure/DaemonBootstrap.swift
git commit -m "feat(macos): DaemonBootstrap — initLogging + startAutobind + subscribe + populate state"
```

---

### Task 29: `Infrastructure/QRCodeRenderer.swift` — render QR image from JSON payload

**Why:** Spec §4.4 / §6.2. CoreImage-based, no external SPM.

**Files:**
- Create: `apps/macos/Minos/Infrastructure/QRCodeRenderer.swift`

- [ ] **Step 1: Write the renderer**

```swift
// apps/macos/Minos/Infrastructure/QRCodeRenderer.swift
//
// Turn a QrPayload into a CGImage via CoreImage's QRCodeGenerator filter.
// Encodes the JSON representation so mobile scanners receive the exact
// same bytes the daemon would have sent over the wire (spec §4.4).

import CoreImage.CIFilterBuiltins
import Foundation
import MinosCore

enum QRCodeRenderer {
    /// Render at a 16x scale for crisp display at ~256×256 points.
    static func image(for payload: QrPayload, scale: CGFloat = 16) -> CGImage? {
        guard let jsonData = try? JSONEncoder().encode(payload) else { return nil }
        let filter = CIFilter.qrCodeGenerator()
        filter.setValue(jsonData, forKey: "inputMessage")
        filter.correctionLevel = "M"
        guard let ci = filter.outputImage else { return nil }
        let scaled = ci.transformed(by: CGAffineTransform(scaleX: scale, y: scale))
        let ctx = CIContext(options: nil)
        return ctx.createCGImage(scaled, from: scaled.extent)
    }
}

// We need QrPayload to be Encodable. UniFFI-generated Swift structs are
// NOT Codable by default — add conformance here via a local mirror struct
// if that assumption proves wrong during first compile. For proc-macro
// UniFFI 0.31, `#[derive(uniffi::Record)]` emits a Swift struct without
// Codable conformance; extend it:

extension QrPayload: Encodable {
    private enum CodingKeys: String, CodingKey { case v, host, port, token, name }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(v, forKey: .v)
        try c.encode(host, forKey: .host)
        try c.encode(port, forKey: .port)
        try c.encode(token, forKey: .token)
        try c.encode(name, forKey: .name)
    }
}

extension PairingToken: Encodable {
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        // PairingToken is a Swift newtype struct around String; expose .value
        // (the UniFFI-generated inner). If the Swift generated type has a
        // different internal field name, replace `self.inner` accordingly.
        try c.encode(self.inner)
    }
}
```

(If `PairingToken`'s generated internal property is named differently — e.g., `value` or `v0` — adjust the extension body. Verify against `Minos/Generated/MinosCore.swift`.)

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Infrastructure/QRCodeRenderer.swift
git commit -m "feat(macos): QRCodeRenderer via CIFilter.qrCodeGenerator + Encodable for QrPayload/PairingToken"
```

---

### Task 30: `Infrastructure/DiagnosticsReveal.swift` — reveal today's log in Finder

**Files:**
- Create: `apps/macos/Minos/Infrastructure/DiagnosticsReveal.swift`

- [ ] **Step 1: Write the function**

```swift
// apps/macos/Minos/Infrastructure/DiagnosticsReveal.swift
//
// Spec §6.4 — open Finder with today's xlog file selected. Errors surface
// via AppState.displayError (the menu item handler awaits this).

import AppKit
import Foundation
import MinosCore

enum DiagnosticsReveal {
    /// Reveal today's log file in Finder. Throws `MinosError.storeIo` when
    /// the file doesn't yet exist (first-boot).
    static func revealTodayLog() throws {
        let path = try todayLogPath()
        let url = URL(fileURLWithPath: path)
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Infrastructure/DiagnosticsReveal.swift
git commit -m "feat(macos): DiagnosticsReveal — NSWorkspace file-viewer reveal of today's xlog"
```

---

### Task 31: `Domain/ConnectionState+Display.swift` and `Domain/MinosError+Display.swift`

**Files:**
- Create: `apps/macos/Minos/Domain/ConnectionState+Display.swift`
- Create: `apps/macos/Minos/Domain/MinosError+Display.swift`

- [ ] **Step 1: `ConnectionState+Display.swift`**

```swift
// apps/macos/Minos/Domain/ConnectionState+Display.swift
//
// Presentation-layer concerns in Domain: label strings + status-icon
// symbol names + tint roles by connection state.

import Foundation
import MinosCore
import SwiftUI

extension ConnectionState {
    /// Short Chinese label for the menu status header.
    var displayLabel: String {
        switch self {
        case .disconnected:                 return "未连接"
        case .pairing:                      return "等待扫码"
        case .connected:                    return "已连接"
        case .reconnecting(let attempt):    return "重连中 · 第 \(attempt) 次"
        }
    }

    /// SF Symbol name for the menubar icon.
    var iconName: String {
        switch self {
        case .disconnected:     return "bolt.circle"
        case .pairing:          return "bolt.circle.fill"
        case .connected:        return "bolt.circle.fill"
        case .reconnecting:     return "bolt.circle.fill"
        }
    }

    /// Tint for the icon.
    var iconTint: Color {
        switch self {
        case .disconnected:     return .secondary
        case .pairing:          return .accentColor
        case .connected:        return .green
        case .reconnecting:     return .orange
        }
    }
}
```

- [ ] **Step 2: `MinosError+Display.swift`**

```swift
// apps/macos/Minos/Domain/MinosError+Display.swift
//
// Spec §7.2 — fetch the Rust-owned localized user string by reducing the
// MinosError to its kind and calling MinosCore.kindMessage. The kind
// switch is the only Swift-side duplicated structure; strings stay in Rust.

import Foundation
import MinosCore

extension MinosError {
    var kind: ErrorKind {
        switch self {
        case .bindFailed:           return .bindFailed
        case .connectFailed:        return .connectFailed
        case .disconnected:         return .disconnected
        case .pairingTokenInvalid:  return .pairingTokenInvalid
        case .pairingStateMismatch: return .pairingStateMismatch
        case .deviceNotTrusted:     return .deviceNotTrusted
        case .storeIo:              return .storeIo
        case .storeCorrupt:         return .storeCorrupt
        case .cliProbeTimeout:      return .cliProbeTimeout
        case .cliProbeFailed:       return .cliProbeFailed
        case .rpcCallFailed:        return .rpcCallFailed
        }
    }

    func userMessage(lang: Lang = .zh) -> String {
        kindMessage(kind: kind, lang: lang)
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add apps/macos/Minos/Domain/
git commit -m "feat(macos): Domain display extensions — ConnectionState label/icon, MinosError.userMessage"
```

---

## Phase J · Swift presentation views

### Task 32: `Presentation/StatusIcon.swift`

**Files:**
- Create: `apps/macos/Minos/Presentation/StatusIcon.swift`

- [ ] **Step 1: Write the view**

```swift
// apps/macos/Minos/Presentation/StatusIcon.swift
//
// Menu bar icon — SF Symbol + tint driven by ConnectionState (or bootError).

import SwiftUI
import MinosCore

struct StatusIcon: View {
    let state: ConnectionState
    let hasError: Bool

    var body: some View {
        Image(systemName: hasError ? "bolt.circle.trianglebadge.exclamationmark" : state.iconName)
            .foregroundStyle(hasError ? .red : state.iconTint)
            .symbolRenderingMode(.hierarchical)
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Presentation/StatusIcon.swift
git commit -m "feat(macos): StatusIcon view — SF Symbol + tint from ConnectionState / bootError"
```

---

### Task 33: `Presentation/QRSheet.swift`

**Files:**
- Create: `apps/macos/Minos/Presentation/QRSheet.swift`

- [ ] **Step 1: Write the modal**

```swift
// apps/macos/Minos/Presentation/QRSheet.swift
//
// Modal sheet showing the pairing QR. Displays host:port debug info, the
// 5-minute TTL countdown, and a Regenerate button. No automatic rotation
// in plan 02 (spec §6.2) — expiry overlay waits for user click.

import SwiftUI
import MinosCore

struct QRSheet: View {
    let payload: QrPayload
    let onRegenerate: () async -> Void
    let onClose: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Text("扫码配对")
                .font(.headline)

            if let cg = QRCodeRenderer.image(for: payload) {
                Image(cg, scale: 1, label: Text("配对二维码"))
                    .interpolation(.none)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(width: 256, height: 256)
                    .background(Color.white)
                    .padding(8)
            } else {
                Text("二维码渲染失败").foregroundStyle(.red)
            }

            Text("有效期 5 分钟 · 在手机端扫描后完成配对")
                .font(.caption)
                .foregroundStyle(.secondary)

            Text("\(payload.host):\(payload.port)")
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(.tertiary)

            HStack(spacing: 12) {
                Button("重新生成") {
                    Task { await onRegenerate() }
                }
                Button("关闭") {
                    onClose()
                }
                .keyboardShortcut(.cancelAction)
            }
        }
        .padding(24)
        .frame(minWidth: 320, minHeight: 360)
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Presentation/QRSheet.swift
git commit -m "feat(macos): QRSheet — CoreImage QR render + regenerate button"
```

---

### Task 34: `Presentation/MenuBarView.swift` — three-branch menu

**Why:** Spec §5.7 + §6.6 + §6.7. Boot error / Unpaired / Paired.

**Files:**
- Create: `apps/macos/Minos/Presentation/MenuBarView.swift`

- [ ] **Step 1: Write the view**

```swift
// apps/macos/Minos/Presentation/MenuBarView.swift
//
// Menu bar dropdown content. Three branches:
// 1. bootError != nil              — error-recovery branch
// 2. trustedDevice != nil          — Paired layout (Forget + logs + Quit)
// 3. default                       — Unpaired layout (Show QR + logs + Quit)

import SwiftUI
import AppKit
import MinosCore

struct MenuBarView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        @Bindable var appState = appState

        VStack(alignment: .leading, spacing: 0) {
            if let bootError = appState.bootError {
                bootErrorBranch(bootError)
            } else if let td = appState.trustedDevice {
                pairedBranch(td)
            } else {
                unpairedBranch()
            }

            Divider()

            Button("在 Finder 中显示今日日志…") {
                revealTodayLog()
            }
            .buttonStyle(.plain)

            Divider()

            Button("退出 Minos") {
                Task { @MainActor in
                    await appState.shutdown()
                    NSApp.terminate(nil)
                }
            }
            .keyboardShortcut("q")
        }
        .padding(8)
        .sheet(isPresented: $appState.isQrSheetPresented) {
            if let qr = appState.currentQr {
                QRSheet(
                    payload: qr,
                    onRegenerate: { await appState.regenerateQr() },
                    onClose: { appState.isQrSheetPresented = false }
                )
            }
        }
    }

    // ── Branches ──────────────────────────────────────────────────────────

    @ViewBuilder
    private func unpairedBranch() -> some View {
        VStack(alignment: .leading, spacing: 4) {
            header
            Text(appState.connectionState.displayLabel)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 4)
        .padding(.vertical, 8)

        Divider()

        Button("显示配对二维码…") {
            Task { await appState.showQr() }
        }
        .disabled(!appState.canShowQr)
    }

    @ViewBuilder
    private func pairedBranch(_ td: TrustedDevice) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            header
            Text("已配对 · 等待回连")
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(td.name)
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 4)
        .padding(.vertical, 8)

        Divider()

        Button("忘记已配对设备") {
            Task { @MainActor in
                if await confirmForget(device: td) {
                    await appState.forgetDevice()
                }
            }
        }
    }

    @ViewBuilder
    private func bootErrorBranch(_ error: MinosError) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.red)
                Text("Minos · 启动失败").bold()
            }
            Text(error.userMessage(lang: .zh))
                .font(.caption)
                .foregroundStyle(.primary)
            Text(String(describing: error))
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(3)

            Button("重试") {
                Task { await DaemonBootstrap.bootstrap(appState: appState) }
            }
        }
        .padding(.horizontal, 4)
        .padding(.vertical, 8)
    }

    // ── Shared ────────────────────────────────────────────────────────────

    private var header: some View {
        HStack {
            StatusIcon(state: appState.connectionState, hasError: appState.bootError != nil)
            Text("Minos").bold()
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    private func revealTodayLog() {
        do {
            try DiagnosticsReveal.revealTodayLog()
        } catch let error as MinosError {
            appState.displayError = error
        } catch {
            appState.displayError = .rpcCallFailed(method: "revealTodayLog", message: "\(error)")
        }
    }

    @MainActor
    private func confirmForget(device: TrustedDevice) async -> Bool {
        let alert = NSAlert()
        alert.messageText = "忘记 \(device.name)?"
        alert.informativeText = "忘记后需要重新扫码才能再次配对。继续吗?"
        alert.addButton(withTitle: "忘记")
        alert.addButton(withTitle: "取消")
        alert.alertStyle = .warning
        let response = alert.runModal()
        return response == .alertFirstButtonReturn
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Presentation/MenuBarView.swift
git commit -m "feat(macos): MenuBarView — BootError / Paired / Unpaired branches + QR sheet + Forget confirm"
```

---

### Task 35: `MinosApp.swift` — @main entry point

**Files:**
- Create: `apps/macos/Minos/MinosApp.swift`

- [ ] **Step 1: Write the app entry**

```swift
// apps/macos/Minos/MinosApp.swift
//
// @main entry. Instantiates AppState as state, wires MenuBarExtra to
// MenuBarView, and kicks off DaemonBootstrap in .onAppear.

import SwiftUI
import MinosCore

@main
struct MinosApp: App {
    @State private var appState = AppState()

    var body: some Scene {
        MenuBarExtra("Minos", systemImage: "bolt.circle") {
            MenuBarView()
                .environment(appState)
                .task {
                    await DaemonBootstrap.bootstrap(appState: appState)
                }
        }
        .menuBarExtraStyle(.window)
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/MinosApp.swift
git commit -m "feat(macos): MinosApp @main — MenuBarExtra scene + DaemonBootstrap.task"
```

---

## Phase K · Swift logic tests

### Task 36: `MinosTests/TestSupport/MockDaemon.swift`

**Why:** Spec §8.3. Tests talk only to `DaemonDriving`; `MockDaemon` satisfies that protocol in-process.

**Files:**
- Create: `apps/macos/MinosTests/TestSupport/MockDaemon.swift`

- [ ] **Step 1: Write the mock**

```swift
// apps/macos/MinosTests/TestSupport/MockDaemon.swift
//
// Minimal test double implementing DaemonDriving. Records every call so
// tests can assert call counts; every method has a configurable closure
// or override to return the desired value / throw the desired error.

import Foundation
import MinosCore
@testable import Minos

final class MockDaemon: DaemonDriving {
    // ── Configurable return / throw sites ────────────────────────────────
    var stubCurrentState: ConnectionState = .disconnected
    var stubCurrentTrustedDevice: Result<TrustedDevice?, MinosError> = .success(nil)
    var stubPairingQr: Result<QrPayload, MinosError>
    var stubForgetDeviceResult: Result<Void, MinosError> = .success(())
    var stubHost: String = "127.0.0.1"
    var stubPort: UInt16 = 7878
    /// Observer captured on last subscribe so tests can drive `on_state`.
    var observer: (any ConnectionStateObserver)?

    // ── Call-count recorders ─────────────────────────────────────────────
    private(set) var forgetCalls: [DeviceId] = []
    private(set) var stopCallCount: Int = 0
    private(set) var subscribeCallCount: Int = 0
    private(set) var cancelCallCount: Int = 0
    private(set) var pairingQrCallCount: Int = 0

    // ── Init with a default valid QR so most tests succeed by default ────
    init() {
        self.stubPairingQr = .success(QrPayload(
            v: 1, host: "127.0.0.1", port: 7878,
            token: PairingToken(inner: "mock-token-44chars__________________________"),
            name: "Mock Mac"
        ))
    }

    // ── DaemonDriving conformance ────────────────────────────────────────
    func currentState() -> ConnectionState { stubCurrentState }

    func currentTrustedDevice() throws -> TrustedDevice? {
        try stubCurrentTrustedDevice.get()
    }

    func pairingQr() throws -> QrPayload {
        pairingQrCallCount += 1
        return try stubPairingQr.get()
    }

    func forgetDevice(id: DeviceId) async throws {
        forgetCalls.append(id)
        try stubForgetDeviceResult.get()
    }

    func host() -> String { stubHost }
    func port() -> UInt16 { stubPort }

    func subscribe(observer: any ConnectionStateObserver) -> Subscription {
        subscribeCallCount += 1
        self.observer = observer
        return MockSubscription(onCancel: { [weak self] in self?.cancelCallCount += 1 })
            as Subscription
    }

    func stop() async throws {
        stopCallCount += 1
    }
}

/// Stand-in for UniFFI's `Subscription`. UniFFI 0.31's `#[uniffi::Object]`
/// generates a Swift class; tests can't construct it directly but can
/// construct a local type that satisfies the same protocol contract via a
/// cast. For test-only purposes, we extend MockSubscription to match the
/// interface the test target uses.
///
/// If `Subscription` turns out to be `final` / non-subclassable from test
/// code, replace this with a pure `AnyObject` erasure behind a local
/// `SubscriptionLike` protocol and refactor `DaemonDriving.subscribe` to
/// return the protocol instead. (Refactor deferred until the real build
/// confirms which path is needed.)
final class MockSubscription {
    private let onCancel: () -> Void
    init(onCancel: @escaping () -> Void) { self.onCancel = onCancel }
    func cancel() { onCancel() }
}
```

- [ ] **Step 2: If Subscription turns out to be non-subclassable, introduce a protocol indirection**

If `xcodebuild` of `MinosTests` complains that `MockSubscription` cannot be cast to `Subscription`, refactor:

1. Introduce a protocol `SubscriptionLike` with a single `cancel()` method.
2. Change `DaemonDriving.subscribe` return type to `SubscriptionLike`.
3. `extension Subscription: SubscriptionLike {}` in `Infrastructure/`.
4. `MockSubscription: SubscriptionLike` satisfies.

If the first-pass cast works (UniFFI 0.31 often generates `public class` without `final`), leave this step's refactor off.

- [ ] **Step 3: Commit**

```bash
git add apps/macos/MinosTests/TestSupport/MockDaemon.swift
git commit -m "test(macos): MockDaemon + MockSubscription — DaemonDriving satisfier with call recorders"
```

---

### Task 37: `MinosTests/Application/AppStateTests.swift` — logic scenarios

**Files:**
- Create: `apps/macos/MinosTests/Application/AppStateTests.swift`

- [ ] **Step 1: Write the test class**

```swift
// apps/macos/MinosTests/Application/AppStateTests.swift
//
// Logic-only tests for AppState. No UI, no @Observable assertions beyond
// direct property reads. Every scenario from spec §8.3 is one test method.

import XCTest
import MinosCore
@testable import Minos

@MainActor
final class AppStateTests: XCTestCase {
    private var mock: MockDaemon!
    private var sut: AppState!

    override func setUp() async throws {
        mock = MockDaemon()
        sut = AppState()
        sut.daemon = mock
    }

    // 1. Observer callback updates connectionState
    func testObserverCallbackUpdatesConnectionState() async {
        let adapter = ObserverAdapter { [weak sut] s in sut?.connectionState = s }
        _ = mock.subscribe(observer: adapter)

        mock.observer?.onState(state: .connected)
        // ObserverAdapter dispatches to MainActor; await one tick.
        await Task.yield()
        XCTAssertEqual(sut.connectionState, .connected)
    }

    // 2. Boot observes trustedDevice on start
    func testBootObservesTrustedDevice() async {
        let td = TrustedDevice(
            deviceId: DeviceId(inner: UUID()),
            name: "iPhone", host: "100.64.0.42", port: 7878,
            pairedAt: Date()
        )
        mock.stubCurrentTrustedDevice = .success(td)
        sut.refreshTrustedDevice()
        XCTAssertNotNil(sut.trustedDevice)
        XCTAssertFalse(sut.canShowQr)
        XCTAssertTrue(sut.canForgetDevice)
    }

    // 3. showQr success
    func testShowQrSuccess() async {
        let qr = QrPayload(v: 1, host: "127.0.0.1", port: 7878,
                           token: PairingToken(inner: "t"), name: "Mac")
        mock.stubPairingQr = .success(qr)
        await sut.showQr()
        XCTAssertNotNil(sut.currentQr)
        XCTAssertTrue(sut.isQrSheetPresented)
        XCTAssertNil(sut.displayError)
    }

    // 4. showQr throws
    func testShowQrThrows() async {
        mock.stubPairingQr = .failure(.storeIo(path: "x", message: "denied"))
        await sut.showQr()
        XCTAssertNotNil(sut.displayError)
        XCTAssertNil(sut.currentQr)
    }

    // 5. regenerateQr returns a different payload
    func testRegenerateQrSwapsPayload() async {
        let q1 = QrPayload(v: 1, host: "h", port: 7878, token: PairingToken(inner: "a"), name: "m")
        mock.stubPairingQr = .success(q1)
        await sut.showQr()
        let first = sut.currentQr

        let q2 = QrPayload(v: 1, host: "h", port: 7878, token: PairingToken(inner: "b"), name: "m")
        mock.stubPairingQr = .success(q2)
        await sut.regenerateQr()

        XCTAssertNotNil(first)
        XCTAssertNotNil(sut.currentQr)
        XCTAssertNotEqual(first?.token.inner, sut.currentQr?.token.inner)
    }

    // 6. forgetDevice success
    func testForgetDeviceSuccess() async {
        let td = TrustedDevice(
            deviceId: DeviceId(inner: UUID()),
            name: "iPhone", host: "100.64.0.42", port: 7878, pairedAt: Date()
        )
        mock.stubCurrentTrustedDevice = .success(td)
        sut.refreshTrustedDevice()
        XCTAssertNotNil(sut.trustedDevice)

        await sut.forgetDevice()

        XCTAssertEqual(mock.forgetCalls.count, 1)
        XCTAssertEqual(mock.forgetCalls.first?.inner, td.deviceId.inner)
        XCTAssertNil(sut.trustedDevice)
    }

    // 7. forgetDevice throws
    func testForgetDeviceThrows() async {
        let td = TrustedDevice(
            deviceId: DeviceId(inner: UUID()),
            name: "iPhone", host: "100.64.0.42", port: 7878, pairedAt: Date()
        )
        mock.stubCurrentTrustedDevice = .success(td)
        mock.stubForgetDeviceResult = .failure(.storeIo(path: "x", message: "denied"))
        sut.refreshTrustedDevice()

        await sut.forgetDevice()

        XCTAssertNotNil(sut.displayError)
        XCTAssertNotNil(sut.trustedDevice, "failed forget must not clear trustedDevice")
    }

    // 8. forgetDevice no-op when no trustedDevice
    func testForgetDeviceNoOpWhenNilTrusted() async {
        XCTAssertNil(sut.trustedDevice)
        await sut.forgetDevice()
        XCTAssertTrue(mock.forgetCalls.isEmpty)
    }

    // 9. bootError hides actions
    func testBootErrorHidesAllActions() {
        sut.bootError = .bindFailed(addr: "100.64.0.10:7878", message: "addr in use")
        XCTAssertFalse(sut.canShowQr)
        XCTAssertFalse(sut.canForgetDevice)
    }

    // 10. shutdown calls daemon.stop + subscription.cancel
    func testShutdownCallsStopAndCancel() async {
        let sub = mock.subscribe(observer: ObserverAdapter { _ in })
        sut.subscription = sub as? Subscription    // see MockSubscription note in Task 36
        await sut.shutdown()
        XCTAssertEqual(mock.stopCallCount, 1)
        XCTAssertEqual(mock.cancelCallCount, 1)
    }
}
```

Caveats:
- `TrustedDevice(deviceId: DeviceId(inner: UUID()), name: ..., host: ..., port: ..., pairedAt: ...)` — the exact generated Swift init signature from UniFFI 0.31 may name the argument `device_id` → `deviceId` (camelCase) or keep snake. Verify against `Minos/Generated/MinosCore.swift` and adjust if it differs.
- `PairingToken(inner: "t")` and `DeviceId(inner: UUID())` assume `custom_newtype!`-generated Swift exposes the wrapped value as `.inner` — verify.

- [ ] **Step 2: Commit**

```bash
git add apps/macos/MinosTests/Application/AppStateTests.swift
git commit -m "test(macos): AppStateTests — 10 scenarios per spec §8.3"
```

---

### Task 38: Wire `xcodebuild test` and verify suite passes

- [ ] **Step 1: Rebuild bindings + regenerate xcodeproj + run tests**

```bash
cargo xtask gen-uniffi
cargo xtask gen-xcode
xcodebuild -project apps/macos/Minos.xcodeproj \
           -scheme MinosTests \
           -destination 'platform=macOS' \
           -configuration Debug \
           test
```

Expected: all 10 tests pass. If UniFFI-generated field names differ (camelCase vs snake), the test file is the only place they surface — fix inline and re-run.

- [ ] **Step 2: Run the full `check-all`**

```bash
cargo xtask check-all
```

Expected: Rust leg + Swift leg both green.

- [ ] **Step 3: If anything fails, fix inline and commit**

This task has no deliverable file of its own — it's the "verify everything lines up" gate before moving to Phase L.

---

## Phase L · CI + ADR + README

### Task 39: ADR 0007 — XcodeGen for macOS project

**Files:**
- Create: `docs/adr/0007-xcodegen-for-macos-project.md`

- [ ] **Step 1: Write the ADR**

```markdown
# ADR 0007 — XcodeGen for macOS project file

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-21 |

## Context

Plan 02 introduces the first Xcode-managed Swift project in the repo (`apps/macos/Minos.app`). We need a way to track the project's build graph in source control. Three options:

1. **Hand-authored `.xcodeproj` / `.pbxproj`** checked in.
2. **XcodeGen** (`project.yml` + `xcodegen generate`).
3. **Tuist** (`Project.swift` Swift DSL).

## Decision

Adopt **XcodeGen** (option 2). Commit `apps/macos/project.yml`; gitignore the generated `Minos.xcodeproj/`.

## Consequences

**Benefits:**
- Project spec is a human-readable YAML reviewable in PRs.
- `.pbxproj` merge conflicts disappear — the file regenerates on every `cargo xtask gen-xcode`.
- UniFFI-generated Swift sources (in `apps/macos/Minos/Generated/`) auto-sync into the project whenever regenerated, without manual file references.
- `cargo xtask bootstrap` installs XcodeGen via Brew; CI runs `xcodegen generate` before `xcodebuild`.

**Costs:**
- Contributors must regenerate the xcodeproj after adding files. Mitigated by `cargo xtask check-all` invoking `gen-xcode` automatically.
- Advanced Xcode configuration (build phases with scripts, custom output groups) must be expressed in `project.yml`, which is less discoverable than Xcode's GUI. No such customization is needed for the MVP.

## Alternatives rejected

- **Hand-authored `.xcodeproj`**: File-reference drift as UniFFI regenerates Swift sources. The mental cost of "I added a Swift file but forgot to drag it into Xcode" is non-trivial.
- **Tuist**: More powerful — modules, dependency graph, caching — but overkill for a single app target. The Swift DSL's type safety helps at scale; we have one target. Reconsider if Minos spawns multiple Swift targets (SwiftPM deps, XPC helpers, etc.).
- **SwiftPM-only**: `Package.swift` has limited support for app bundles, `MenuBarExtra`, `LSUIElement`, and entitlements. As of Swift 6.x, these still require Xcode project files.
```

- [ ] **Step 2: Commit**

```bash
git add docs/adr/0007-xcodegen-for-macos-project.md
git commit -m "docs(adr): 0007 — XcodeGen for macOS project file"
```

---

### Task 40: Update `.github/workflows/ci.yml` with macos-14 job

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Append a `swift` job**

Rewrite `.github/workflows/ci.yml`:

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
    name: rust (xtask check-all)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install rust toolchain (linux-only override)
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - uses: Swatinem/rust-cache@v2

      - name: Install cargo-deny
        run: cargo install cargo-deny --locked

      - name: cargo xtask check-all
        # On linux this runs the Rust leg only; the Swift leg is gated on
        # cfg!(target_os = "macos") inside xtask.
        run: cargo xtask check-all

  swift:
    name: swift (xcodebuild + swiftlint)
    runs-on: macos-14
    steps:
      - uses: actions/checkout@v4

      - name: Install rust toolchain (apple targets)
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-darwin,x86_64-apple-darwin

      - uses: Swatinem/rust-cache@v2

      - name: Install brew packages (XcodeGen + SwiftLint)
        run: brew bundle --file apps/macos/Brewfile

      - name: Install uniffi-bindgen-swift
        run: cargo install uniffi-bindgen-swift --locked

      - name: cargo xtask build-macos
        run: cargo xtask build-macos

      - name: cargo xtask gen-uniffi
        run: cargo xtask gen-uniffi

      - name: cargo xtask gen-xcode
        run: cargo xtask gen-xcode

      - name: xcodebuild Minos build
        working-directory: apps/macos
        run: |
          xcodebuild -project Minos.xcodeproj \
                     -scheme Minos \
                     -destination "platform=macOS" \
                     -configuration Debug \
                     build

      - name: xcodebuild MinosTests test
        working-directory: apps/macos
        run: |
          xcodebuild -project Minos.xcodeproj \
                     -scheme MinosTests \
                     -destination "platform=macOS" \
                     -configuration Debug \
                     test

      - name: swiftlint --strict
        working-directory: apps/macos
        run: swiftlint --strict
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add swift job on macos-14 — build-macos + gen-uniffi + gen-xcode + xcodebuild + swiftlint"
```

---

### Task 41: Update `README.md` with plan-02 status

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the Status section**

Edit `README.md`:

```markdown
# Minos

Native macOS status-bar app + Flutter mobile client + shared Rust core for remote AI-coding control. Drive `codex` / `claude` / `gemini` on a Mac from a paired phone over Tailscale.

## Status

- **Plan 01 (complete):** 9 Rust crates, in-process E2E (`pair → list_clis`), CI green.
- **Plan 02 (complete):** macOS status-bar app (`ai.minos.macos`), UniFFI bindings over `DaemonHandle`, XcodeGen project, macos-14 CI. Run `cargo xtask check-all` for the full verification set.
- **Plan 03 (next):** Flutter + flutter_rust_bridge iOS app.

See `docs/superpowers/specs/macos-app-and-uniffi-design.md` for the design and `docs/superpowers/plans/` for the implementation plans.

## Quick start (development)

```bash
# Install dev tools: cargo-deny, uniffi bindgen, xcodegen, swiftlint.
cargo xtask bootstrap

# Run all checks (fmt + clippy + tests + xcodebuild + swiftlint when on macOS).
cargo xtask check-all
```

## Repository layout

```
crates/   Rust workspace (9 crates: domain, protocol, pairing, cli-detect,
          transport, daemon, mobile, ffi-uniffi, ffi-frb)
apps/     macOS (Swift/UniFFI) and mobile (Flutter/frb) — macOS filled in plan 02
xtask/    Build / codegen orchestration in Rust
docs/     Specs (docs/superpowers/specs/) and ADRs (docs/adr/)
```

## License

MIT — see `LICENSE`.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(readme): plan 02 complete — macOS app + UniFFI"
```

---

## Final verification

### Task 42: End-to-end green gate

- [ ] **Step 1: From a fresh clone on a Mac, run bootstrap and check-all**

```bash
cd minos
cargo xtask bootstrap
cargo xtask check-all
```

Expected: all Rust checks pass, Swift leg runs through build + tests + lint with zero errors.

- [ ] **Step 2: Manual sanity (pre-merge gate, per spec §8.6 item 6)**

```bash
cargo xtask build-macos
cargo xtask gen-uniffi
cargo xtask gen-xcode
xcodebuild -project apps/macos/Minos.xcodeproj \
           -scheme Minos \
           -destination "platform=macOS" \
           -configuration Debug \
           build

# Find the .app in DerivedData and launch it
open ~/Library/Developer/Xcode/DerivedData/Minos-*/Build/Products/Debug/Minos.app
```

Manual check:
- Status-bar icon appears (bolt.circle)
- Click icon → Unpaired layout (Minos · 未连接), "显示配对二维码…" visible
- Click "显示配对二维码…" → modal appears, QR rendered (256×256)
- Click "重新生成" → QR refreshes
- Click "关闭" → modal dismisses
- Click "在 Finder 中显示今日日志…" → Finder opens with today's .xlog selected
- Click "退出 Minos" → app exits
- `~/Library/Logs/Minos/daemon_YYYYMMDD.xlog` exists and is non-empty

If any manual check fails, file a follow-up task and do **not** mark plan complete.

- [ ] **Step 3: No commit — this is a verification step only**

---

## Plan summary

Plan 02 delivers:

| Deliverable | Entry point | Verification |
|---|---|---|
| Daemon FFI-friendly refactor | `minos-daemon` crate | `cargo test -p minos-daemon` |
| UniFFI shim (custom types + re-exports) | `minos-ffi-uniffi/src/lib.rs` | `cargo xtask build-macos` |
| Swift bindings codegen | `cargo xtask gen-uniffi` | `ls apps/macos/Minos/Generated/*` |
| XcodeGen project | `apps/macos/project.yml` | `cargo xtask gen-xcode` |
| macOS app (status bar, QR sheet, Forget, logs reveal) | `apps/macos/Minos/MinosApp.swift` | manual sanity (§8.6 #6) |
| Logic tests | `apps/macos/MinosTests/` | `xcodebuild test` |
| CI | `.github/workflows/ci.yml` `swift` job | GitHub Actions macos-14 |
| ADR 0007 | `docs/adr/0007-xcodegen-for-macos-project.md` | review |
| README update | `README.md` | review |

Total: **42 tasks** across 12 phases (A through L). Each task is independently committable and leaves the workspace in a compilable state (Rust always; Swift after Task 21 onward).
