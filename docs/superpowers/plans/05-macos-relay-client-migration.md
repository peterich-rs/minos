# macOS Relay-Client Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the macOS app from Tailscale-P2P (Mac binds WS server on 100.x) to relay-client (Mac dials outbound WSS to the already-deployed `minos-relay` broker). Ship new onboarding UX, two-axis state model, Keychain persistence, and a `fake-peer` dev bin to smoke-test pairing without iOS.

**Architecture:** Six-step daemon surgery (add new domain types → shrink `DaemonInner` → envelope codec → wire relay-client task → implement LocalRpc senders → delete Tailscale). Swift layer follows once UniFFI surface is regenerated. See `docs/superpowers/specs/macos-relay-client-migration-design.md` for full design context.

**Tech Stack:** Rust 1.x stable, `axum`/`tokio-tungstenite`/`jsonrpsee` (client-only on Mac), `security-framework` (new Rust Keychain dep, macOS-target-gated), UniFFI 0.31, SwiftUI on macOS 14+, `Security.framework` on the Swift side.

---

## Divergence from the spec (read first)

The spec §5.1 table said to modify `minos-pairing` in place: delete `Pairing` state machine, drop `tailscale_ip` from `TrustedDevice`, change `QrPayload` schema. The survey during plan writing confirmed `minos-pairing` types are exported through `uniffi::Record` into both Swift and Dart (via frb). **Changing them in place would break iOS compile** — contradicting the Mac-only scope invariant locked in Q1.

**Resolution adopted by this plan:**
- `crates/minos-pairing/*` — untouched. Old `QrPayload`, `TrustedDevice`, `Pairing`, `ActiveToken`, `generate_qr_payload` all remain. iOS continues to compile and use them through its Tailscale path.
- New Mac-only types live in `crates/minos-daemon/src/relay_pairing.rs`: `RelayQrPayload` (relay-style schema) and `PeerRecord` (peer metadata without Tailscale IP).
- New Mac-only store `crates/minos-daemon/src/keychain_store.rs` implements a local Mac interface for peer + device_secret persistence (does **not** implement `minos_pairing::PairingStore` — different trait, distinct from iOS's usage).
- UniFFI re-exports switch: Swift sees `RelayQrPayload` and `PeerRecord` instead of `QrPayload`/`TrustedDevice`. iOS (frb) is unaffected because `minos-pairing` is untouched.
- When the iOS migration spec lands, it can delete the legacy types in `minos-pairing` and promote the Mac types to the shared crate.

Everything else in the spec applies as written.

## Commit and gate convention

Per project memory rule "Run cargo xtask check-all before every relay-plan commit":
- Within a phase, tasks may commit with a scoped gate: `cargo test -p <affected-crate>` for quick feedback during TDD.
- At the end of every phase, the last commit in the phase runs `cargo xtask check-all` as a hard gate. If it fails, add a fix commit within the same phase before advancing.
- Swift-touching phases (J–N) run `xcodebuild` via `check-all` as part of their terminal gate.

Commits in the examples below use the compact `cargo test -p <crate>` gate; replace with `cargo xtask check-all` at every phase-closing task (explicitly called out by a "☑ Phase N closer" step).

---

## Phase A — New domain types in `minos-domain`

Goal: introduce `RelayLinkState`, `PeerState`, `DeviceSecret`, and one new `MinosError` variant (`CfAuthFailed`). `ConnectionState` stays (for iOS).

### Task A.1: Scaffold `relay_state.rs` with `RelayLinkState`

**Files:**
- Create: `crates/minos-domain/src/relay_state.rs`
- Modify: `crates/minos-domain/src/lib.rs`
- Test: in-file `#[cfg(test)] mod tests` + `crates/minos-domain/tests/relay_state_golden.rs`

- [ ] **Step 1: Write the failing serde round-trip test**

Add `crates/minos-domain/tests/relay_state_golden.rs`:

```rust
use minos_domain::RelayLinkState;

#[test]
fn relay_link_state_disconnected_serde_round_trip() {
    let state = RelayLinkState::Disconnected;
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, r#""disconnected""#);
    let back: RelayLinkState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);
}

#[test]
fn relay_link_state_connecting_carries_attempt() {
    let state = RelayLinkState::Connecting { attempt: 3 };
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, r#"{"connecting":{"attempt":3}}"#);
    let back: RelayLinkState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);
}

#[test]
fn relay_link_state_connected_serde_round_trip() {
    let state = RelayLinkState::Connected;
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, r#""connected""#);
    let back: RelayLinkState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p minos-domain --test relay_state_golden
```

Expected: FAIL with "cannot find type `RelayLinkState` in this scope" (imports don't resolve).

- [ ] **Step 3: Create the module and type**

Create `crates/minos-domain/src/relay_state.rs`:

```rust
//! Relay client-side state axes. Two independent enums — link (to relay)
//! and peer (to paired iPhone). See spec §4.3.

use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RelayLinkState {
    Disconnected,
    Connecting { attempt: u32 },
    Connected,
}
```

- [ ] **Step 4: Register module and re-export in `lib.rs`**

Modify `crates/minos-domain/src/lib.rs` — add `pub mod relay_state;` next to existing `pub mod connection;`, and `pub use relay_state::RelayLinkState;` next to existing re-exports.

- [ ] **Step 5: Run tests to verify pass**

```bash
cargo test -p minos-domain --test relay_state_golden
```

Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
cd /Users/zhangfan/develop/github.com/minos-worktrees/macos-relay-migration
git add crates/minos-domain/src/relay_state.rs crates/minos-domain/src/lib.rs crates/minos-domain/tests/relay_state_golden.rs
git commit -m "feat(domain): add RelayLinkState enum with snake_case serde"
```

---

### Task A.2: Add `PeerState` in the same module

**Files:**
- Modify: `crates/minos-domain/src/relay_state.rs`
- Modify: `crates/minos-domain/src/lib.rs` (re-export)
- Modify: `crates/minos-domain/tests/relay_state_golden.rs` (additional cases)

- [ ] **Step 1: Add failing `PeerState` round-trip tests**

Append to `crates/minos-domain/tests/relay_state_golden.rs`:

```rust
use minos_domain::{DeviceId, PeerState};

#[test]
fn peer_state_unpaired_serde() {
    let s = PeerState::Unpaired;
    assert_eq!(serde_json::to_string(&s).unwrap(), r#""unpaired""#);
}

#[test]
fn peer_state_pairing_serde() {
    let s = PeerState::Pairing;
    assert_eq!(serde_json::to_string(&s).unwrap(), r#""pairing""#);
}

#[test]
fn peer_state_paired_carries_metadata() {
    let id = DeviceId::new();
    let s = PeerState::Paired {
        peer_id: id,
        peer_name: "fannnzhang's iPhone".into(),
        online: true,
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: PeerState = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}
```

- [ ] **Step 2: Verify failure**

```bash
cargo test -p minos-domain --test relay_state_golden peer_state
```

Expected: FAIL (`PeerState` not found).

- [ ] **Step 3: Add `PeerState` to relay_state.rs**

Append to `crates/minos-domain/src/relay_state.rs`:

```rust
use crate::DeviceId;

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerState {
    Unpaired,
    Pairing,
    Paired {
        peer_id: DeviceId,
        peer_name: String,
        online: bool,
    },
}
```

- [ ] **Step 4: Re-export from lib.rs**

Add `pub use relay_state::PeerState;` to `crates/minos-domain/src/lib.rs`.

- [ ] **Step 5: Run**

```bash
cargo test -p minos-domain --test relay_state_golden
```

Expected: PASS (6 tests).

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "feat(domain): add PeerState with peer metadata"
```

---

### Task A.3: Add `DeviceSecret` newtype

**Files:**
- Modify: `crates/minos-domain/src/ids.rs` (or wherever newtype-style ids live — verify)
- Modify: `crates/minos-domain/src/lib.rs`
- Test: in-file module tests

- [ ] **Step 1: Confirm file placement**

```bash
grep -n "pub struct DeviceId" crates/minos-domain/src/*.rs
```

Expected: one hit inside `crates/minos-domain/src/ids.rs`. `DeviceSecret` will live next to it.

- [ ] **Step 2: Write failing test**

Append to `crates/minos-domain/src/ids.rs` within the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn device_secret_round_trips_as_string() {
    let s = DeviceSecret("hunter2-the-32-byte-base64-secret".into());
    let j = serde_json::to_string(&s).unwrap();
    assert_eq!(j, r#""hunter2-the-32-byte-base64-secret""#);
    let back: DeviceSecret = serde_json::from_str(&j).unwrap();
    assert_eq!(s, back);
}

#[test]
fn device_secret_debug_redacts() {
    let s = DeviceSecret("super-secret".into());
    let d = format!("{:?}", s);
    assert!(!d.contains("super-secret"), "DeviceSecret Debug must not leak");
    assert!(d.contains("DeviceSecret"));
}
```

- [ ] **Step 3: Run, verify failure**

```bash
cargo test -p minos-domain device_secret
```

Expected: FAIL (`DeviceSecret` not defined).

- [ ] **Step 4: Add the newtype**

In `crates/minos-domain/src/ids.rs`, next to `DeviceId`:

```rust
/// 32-byte (post-base64) secret issued by the relay on successful pair.
/// `Debug` is explicitly redacting; do not log the inner String.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceSecret(pub String);

impl std::fmt::Debug for DeviceSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DeviceSecret").field(&"<redacted>").finish()
    }
}

#[cfg(feature = "uniffi")]
uniffi::custom_newtype!(DeviceSecret, String);
```

- [ ] **Step 5: Re-export from lib.rs**

Find the existing `pub use ids::{DeviceId, ...};` and add `DeviceSecret` to that list.

- [ ] **Step 6: Verify tests pass**

```bash
cargo test -p minos-domain device_secret
```

Expected: PASS (2 tests).

- [ ] **Step 7: Commit**

```bash
git add -u
git commit -m "feat(domain): add DeviceSecret newtype with redacting Debug"
```

---

### Task A.4: Add `CfAuthFailed` to `MinosError`

The survey confirmed `Unauthorized`, `ConnectionStateMismatch`, `EnvelopeVersionUnsupported`, `PeerOffline`, `RelayInternal` are already present from the relay PR. Only `CfAuthFailed` is missing.

**Files:**
- Modify: `crates/minos-domain/src/error.rs`
- Test: in-file

- [ ] **Step 1: Write failing test**

Add to `crates/minos-domain/src/error.rs` tests module:

```rust
#[test]
fn cf_auth_failed_display_and_kind() {
    let err = MinosError::CfAuthFailed {
        message: "Cloudflare denied".into(),
    };
    assert_eq!(err.kind(), ErrorKind::CfAuthFailed);
    let s = err.to_string();
    assert!(s.contains("cloudflare"));
    assert!(s.contains("Cloudflare denied"));
}

#[test]
fn cf_auth_failed_user_message_zh_no_tailscale_wording() {
    let m = ErrorKind::CfAuthFailed.user_message(Lang::Zh);
    assert!(m.contains("Cloudflare"));
    assert!(!m.to_lowercase().contains("tailscale"));
}
```

- [ ] **Step 2: Run, verify failure**

```bash
cargo test -p minos-domain cf_auth_failed
```

Expected: FAIL (variant missing).

- [ ] **Step 3: Add the variant**

In `crates/minos-domain/src/error.rs`, add to the `MinosError` enum (anywhere, conventional order is grouped by layer):

```rust
#[error("cloudflare access authentication failed: {message}")]
CfAuthFailed { message: String },
```

In the same file, add to `ErrorKind`:

```rust
CfAuthFailed,
```

In `MinosError::kind()` match, add the mapping arm:

```rust
MinosError::CfAuthFailed { .. } => ErrorKind::CfAuthFailed,
```

In `ErrorKind::user_message` match, add both language arms:

```rust
(Self::CfAuthFailed, Lang::Zh) => "Cloudflare Access 认证失败，请检查 Service Token",
(Self::CfAuthFailed, Lang::En) => "Cloudflare Access authentication failed; please check the Service Token",
```

- [ ] **Step 4: Run, verify pass**

```bash
cargo test -p minos-domain
```

Expected: all existing tests plus 2 new pass; `no_tailscale_strings_remain_in_user_messages` still passes.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(domain): add MinosError::CfAuthFailed variant"
```

---

### Task A.5: Phase A closer — run `check-all`

- [ ] **Step 1: Run workspace gate**

```bash
cd /Users/zhangfan/develop/github.com/minos-worktrees/macos-relay-migration
cargo xtask check-all
```

Expected: green. If red, add a fix commit before advancing to Phase B.

- [ ] **Step 2: Verify with git log**

```bash
git log --oneline feat/macos-relay-migration -n 5
```

Expected: 4 domain-layer commits (A.1 through A.4) plus the spec commit (`040708d`), plus the baseline main commit.

---

## Phase B — Mac-only types in `minos-daemon`

Goal: add `RelayQrPayload`, `PeerRecord`, `RelayConfig`, `BACKEND_URL` constant, and the local-state JSON read/write helper — all Mac-only, none touching `minos-pairing`.

### Task B.1: Add `config.rs` with `BACKEND_URL` and `RelayConfig`

**Files:**
- Create: `crates/minos-daemon/src/config.rs`
- Modify: `crates/minos-daemon/src/lib.rs`
- Test: in-file

- [ ] **Step 1: Write failing test**

Create `crates/minos-daemon/src/config.rs`:

```rust
//! Compile-time backend URL + runtime Relay configuration. See spec §10.1.

/// Compile-time backend URL. Overridable via `MINOS_BACKEND_URL` env var at build.
/// Fallback is the local dev relay (`cargo run -p minos-relay`).
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => "ws://127.0.0.1:8787/devices",
};

/// Runtime relay config (CF Service Token pair). Backend URL is BACKEND_URL (compile-time).
#[derive(Clone, Debug)]
pub struct RelayConfig {
    pub cf_client_id: String,
    pub cf_client_secret: String,
}

impl RelayConfig {
    pub fn new(cf_client_id: String, cf_client_secret: String) -> Self {
        Self { cf_client_id, cf_client_secret }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_url_has_a_sane_fallback() {
        // Not asserting the exact URL (CI may inject), only that it's non-empty
        // and looks like a ws URL.
        assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
    }

    #[test]
    fn relay_config_ctor_stores_fields() {
        let c = RelayConfig::new("id".into(), "secret".into());
        assert_eq!(c.cf_client_id, "id");
        assert_eq!(c.cf_client_secret, "secret");
    }
}
```

- [ ] **Step 2: Register in `lib.rs`**

Modify `crates/minos-daemon/src/lib.rs` — add `pub mod config;` and `pub use config::{RelayConfig, BACKEND_URL};`.

- [ ] **Step 3: Run**

```bash
cargo test -p minos-daemon --lib config
```

Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/config.rs crates/minos-daemon/src/lib.rs
git commit -m "feat(daemon): add config module with BACKEND_URL + RelayConfig"
```

---

### Task B.2: Add `relay_pairing.rs` with `RelayQrPayload` and `PeerRecord`

**Files:**
- Create: `crates/minos-daemon/src/relay_pairing.rs`
- Modify: `crates/minos-daemon/src/lib.rs`
- Test: in-file

- [ ] **Step 1: Write file**

Create `crates/minos-daemon/src/relay_pairing.rs`:

```rust
//! Mac-only pairing types for the relay flow. Kept in minos-daemon instead
//! of minos-pairing so iOS's frb bindings (which import the legacy types)
//! are untouched until iOS migrates to relay. See plan divergence note.

use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, PairingToken};
use serde::{Deserialize, Serialize};

/// QR payload emitted by the Mac when pairing. Encodes where and what —
/// the relay backend URL, a one-shot pairing token, and the Mac's display
/// name. No IP/port: the backend is Cloudflare-fronted, addresses are
/// invariant across deployments (baked at compile time).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RelayQrPayload {
    pub v: u8,
    pub backend_url: String,
    pub token: PairingToken,
    pub mac_display_name: String,
}

/// Mac-side peer record (formerly `minos_pairing::TrustedDevice` without
/// the Tailscale IP/port fields). Persisted in local-state.json.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PeerRecord {
    pub device_id: DeviceId,
    pub name: String,
    pub paired_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_qr_payload_round_trip() {
        let qr = RelayQrPayload {
            v: 1,
            backend_url: "wss://minos.fan-nn.top/devices".into(),
            token: PairingToken("example-32b".into()),
            mac_display_name: "fannnzhang's MacBook".into(),
        };
        let j = serde_json::to_string(&qr).unwrap();
        let back: RelayQrPayload = serde_json::from_str(&j).unwrap();
        assert_eq!(qr, back);
        assert!(!j.contains("host"));
        assert!(!j.contains("port"));
    }

    #[test]
    fn peer_record_round_trip() {
        let pr = PeerRecord {
            device_id: DeviceId::new(),
            name: "fannnzhang's iPhone".into(),
            paired_at: Utc::now(),
        };
        let j = serde_json::to_string(&pr).unwrap();
        let back: PeerRecord = serde_json::from_str(&j).unwrap();
        assert_eq!(pr.device_id, back.device_id);
        assert_eq!(pr.name, back.name);
    }
}
```

- [ ] **Step 2: Register in lib.rs**

Add to `crates/minos-daemon/src/lib.rs`: `pub mod relay_pairing;` and `pub use relay_pairing::{RelayQrPayload, PeerRecord};`.

- [ ] **Step 3: Run**

```bash
cargo test -p minos-daemon --lib relay_pairing
```

Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/relay_pairing.rs crates/minos-daemon/src/lib.rs
git commit -m "feat(daemon): add RelayQrPayload + PeerRecord (Mac-only types)"
```

---

### Task B.3: Add `local_state.rs` for JSON persistence (self_device_id + peer)

**Files:**
- Create: `crates/minos-daemon/src/local_state.rs`
- Modify: `crates/minos-daemon/src/lib.rs`
- Test: in-file

- [ ] **Step 1: Write failing test**

Create `crates/minos-daemon/src/local_state.rs`:

```rust
//! Plain-JSON persistence for the Mac-side non-secret state:
//! `self_device_id` (UUIDv4) + `peer` (nullable `PeerRecord`).
//! Secrets (CF tokens, device_secret) are NOT stored here — they go to
//! the Keychain via `keychain_store.rs`.

use crate::relay_pairing::PeerRecord;
use minos_domain::{DeviceId, MinosError};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalState {
    pub self_device_id: DeviceId,
    pub peer: Option<PeerRecord>,
}

impl LocalState {
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_default();
        let override_dir = std::env::var("MINOS_DATA_DIR").ok();
        let dir = match override_dir {
            Some(d) => PathBuf::from(d),
            None => PathBuf::from(&home).join("Library/Application Support/Minos"),
        };
        dir.join("local-state.json")
    }

    /// Load or initialize. If missing, create fresh with a new DeviceId.
    /// If present but unparseable, return `StoreCorrupt` — caller surfaces
    /// as a bootError; user deletes the file manually.
    pub fn load_or_init(path: &Path) -> Result<Self, MinosError> {
        if !path.exists() {
            let state = Self {
                self_device_id: DeviceId::new(),
                peer: None,
            };
            state.save(path)?;
            return Ok(state);
        }
        let bytes = fs::read(path).map_err(|e| MinosError::StoreIo {
            path: path.display().to_string(),
            source: e,
        })?;
        serde_json::from_slice(&bytes).map_err(|e| MinosError::StoreCorrupt {
            path: path.display().to_string(),
            source: e,
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), MinosError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| MinosError::StoreIo {
                path: parent.display().to_string(),
                source: e,
            })?;
        }
        let buf = serde_json::to_vec_pretty(self).map_err(|e| MinosError::StoreCorrupt {
            path: path.display().to_string(),
            source: e,
        })?;
        fs::write(path, buf).map_err(|e| MinosError::StoreIo {
            path: path.display().to_string(),
            source: e,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_or_init_creates_fresh_state_on_missing_file() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("local-state.json");
        let s = LocalState::load_or_init(&p).unwrap();
        assert!(s.peer.is_none());
        assert!(p.exists());
    }

    #[test]
    fn load_round_trips_peer() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("local-state.json");
        let original = LocalState {
            self_device_id: DeviceId::new(),
            peer: Some(PeerRecord {
                device_id: DeviceId::new(),
                name: "iPhone".into(),
                paired_at: chrono::Utc::now(),
            }),
        };
        original.save(&p).unwrap();
        let back = LocalState::load_or_init(&p).unwrap();
        assert_eq!(original.self_device_id, back.self_device_id);
        assert_eq!(original.peer.as_ref().unwrap().name, "iPhone");
    }

    #[test]
    fn load_on_corrupt_file_returns_store_corrupt() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("local-state.json");
        fs::write(&p, b"{this is not json").unwrap();
        let err = LocalState::load_or_init(&p).unwrap_err();
        assert!(matches!(err, MinosError::StoreCorrupt { .. }));
    }
}
```

- [ ] **Step 2: Register in lib.rs**

Add `pub mod local_state;` and `pub use local_state::LocalState;` to `crates/minos-daemon/src/lib.rs`.

- [ ] **Step 3: Run**

```bash
cargo test -p minos-daemon --lib local_state
```

Expected: PASS (3 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/local_state.rs crates/minos-daemon/src/lib.rs
git commit -m "feat(daemon): add LocalState JSON persistence for device_id + peer"
```

---

### Task B.4: Phase B closer

- [ ] **Step 1: Run `check-all`**

```bash
cargo xtask check-all
```

Expected: green.

- [ ] **Step 2: If red, fix and amend into B.3's commit (or add a B.5 fix commit).**

---

## Phase C — Keychain store for `device-secret` (Rust side)

Goal: add `security-framework` Cargo dep (macOS-target-gated), write `KeychainTrustedDeviceStore` reading/writing the `device-secret` account under service `ai.minos.macos`.

### Task C.1: Add `security-framework` to workspace + daemon manifests

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/minos-daemon/Cargo.toml`

- [ ] **Step 1: Edit workspace Cargo.toml**

In `Cargo.toml` `[workspace.dependencies]` append:

```toml
security-framework = { version = "3", default-features = false }
```

Use version 3.x (current major) with default-features off to avoid pulling OpenSSL. If clippy or cargo-deny flags a specific advisory, pin to the precise minor in the daemon manifest rather than workspace-wide.

- [ ] **Step 2: Edit crates/minos-daemon/Cargo.toml**

Append under `[target.'cfg(target_os = "macos")'.dependencies]` (create section if missing):

```toml
[target.'cfg(target_os = "macos")'.dependencies]
security-framework = { workspace = true }
```

- [ ] **Step 3: Verify the dep resolves**

```bash
cargo build -p minos-daemon
```

Expected: compiles, no new errors.

- [ ] **Step 4: Update cargo-deny if needed**

```bash
cargo xtask check-all 2>&1 | tail -40
```

If `cargo deny check` flags the new dep, add it to `deny.toml` exceptions with a comment referencing spec §7.3.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/minos-daemon/Cargo.toml
# plus deny.toml if modified
git commit -m "chore(daemon): add security-framework dep (macOS-target-gated)"
```

---

### Task C.2: Write `keychain_store.rs`

**Files:**
- Create: `crates/minos-daemon/src/keychain_store.rs`
- Modify: `crates/minos-daemon/src/lib.rs`
- Test: in-file, gated on `cfg(target_os = "macos")`

- [ ] **Step 1: Write failing test (macOS only)**

Create `crates/minos-daemon/src/keychain_store.rs`:

```rust
//! macOS Keychain adapter for the `device-secret` account under service
//! `ai.minos.macos`. CF Client ID/Secret are written by the Swift layer,
//! read by both; this module only owns `device-secret`.

#[cfg(target_os = "macos")]
pub mod imp {
    use minos_domain::{DeviceSecret, MinosError};
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    const SERVICE: &str = "ai.minos.macos";
    const ACCOUNT_DEVICE_SECRET: &str = "device-secret";

    pub struct KeychainTrustedDeviceStore;

    impl KeychainTrustedDeviceStore {
        /// Read the persisted `device-secret`. Returns `Ok(None)` when
        /// no entry is present.
        pub fn read(&self) -> Result<Option<DeviceSecret>, MinosError> {
            match get_generic_password(SERVICE, ACCOUNT_DEVICE_SECRET) {
                Ok(bytes) => {
                    let s = String::from_utf8(bytes).map_err(|e| MinosError::StoreCorrupt {
                        path: format!("Keychain {}/{}", SERVICE, ACCOUNT_DEVICE_SECRET),
                        source: serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            e,
                        )),
                    })?;
                    Ok(Some(DeviceSecret(s)))
                }
                Err(e) if e.code() == -25300 => Ok(None), // errSecItemNotFound
                Err(e) => Err(MinosError::StoreIo {
                    path: format!("Keychain {}/{}", SERVICE, ACCOUNT_DEVICE_SECRET),
                    source: std::io::Error::other(format!("keychain read: {e}")),
                }),
            }
        }

        pub fn write(&self, secret: &DeviceSecret) -> Result<(), MinosError> {
            set_generic_password(SERVICE, ACCOUNT_DEVICE_SECRET, secret.0.as_bytes()).map_err(
                |e| MinosError::StoreIo {
                    path: format!("Keychain {}/{}", SERVICE, ACCOUNT_DEVICE_SECRET),
                    source: std::io::Error::other(format!("keychain write: {e}")),
                },
            )
        }

        /// Delete the entry. Succeeds (Ok) if the entry doesn't exist.
        pub fn delete(&self) -> Result<(), MinosError> {
            match delete_generic_password(SERVICE, ACCOUNT_DEVICE_SECRET) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == -25300 => Ok(()),
                Err(e) => Err(MinosError::StoreIo {
                    path: format!("Keychain {}/{}", SERVICE, ACCOUNT_DEVICE_SECRET),
                    source: std::io::Error::other(format!("keychain delete: {e}")),
                }),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use minos_domain::DeviceSecret;

        /// Integration: writes then reads via the real login keychain.
        /// Runs on macOS dev + CI. If the keychain is locked, this test
        /// requires an interactive prompt; CI runner's keychain is unlocked.
        #[test]
        fn write_then_read_round_trips() {
            let store = KeychainTrustedDeviceStore;
            // Clean up first in case a prior test run left residue.
            let _ = store.delete();

            let secret = DeviceSecret("test-secret-xyz".into());
            store.write(&secret).unwrap();
            let got = store.read().unwrap().expect("just wrote");
            assert_eq!(got, secret);

            // Cleanup.
            store.delete().unwrap();
            assert!(store.read().unwrap().is_none());
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::KeychainTrustedDeviceStore;
```

- [ ] **Step 2: Register in lib.rs**

Add to `crates/minos-daemon/src/lib.rs`:

```rust
#[cfg(target_os = "macos")]
pub mod keychain_store;
#[cfg(target_os = "macos")]
pub use keychain_store::KeychainTrustedDeviceStore;
```

- [ ] **Step 3: Run**

```bash
cargo test -p minos-daemon --lib keychain_store
```

Expected on macOS dev box: PASS. If the test fails with a keychain access prompt in CI: investigate running the test inside the unlocked `login.keychain-db` (the GitHub macos-15 runner has this unlocked by default).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/keychain_store.rs crates/minos-daemon/src/lib.rs
git commit -m "feat(daemon): add KeychainTrustedDeviceStore for device-secret"
```

---

### Task C.3: Phase C closer

- [ ] **Step 1: Run `cargo xtask check-all`**

Expected: green. Note that the `linux` CI job does **not** compile the `#[cfg(target_os = "macos")]` module, so it trivially skips.

---

## Phase D — Transport refactor (`minos-transport`)

Goal: delete server code, add AuthHeaders, extend `WsClient::connect` with auth + close-code mapping.

### Task D.1: Delete `server.rs` and its tests

**Files:**
- Delete: `crates/minos-transport/src/server.rs`
- Delete: any `crates/minos-transport/tests/server_*.rs` (check)
- Modify: `crates/minos-transport/src/lib.rs` (remove `pub mod server;` + re-exports)
- Modify: `crates/minos-daemon/src/handle.rs` (remove `use minos_transport::WsServer` imports — anticipate)

- [ ] **Step 1: Check what references `WsServer`**

```bash
grep -rn "WsServer\|use minos_transport::server\|transport::server" crates apps
```

- [ ] **Step 2: Delete file**

```bash
rm crates/minos-transport/src/server.rs
# and any test file that references WsServer; inspect first
ls crates/minos-transport/tests/ 2>/dev/null
```

If tests exist that import server, delete them too.

- [ ] **Step 3: Update `lib.rs`**

Remove `pub mod server;` and any `pub use server::...;` line. Verify `crates/minos-transport/src/lib.rs` compiles.

- [ ] **Step 4: Build to find remaining references**

```bash
cargo build -p minos-transport
cargo build -p minos-daemon 2>&1 | grep WsServer
```

If `minos-daemon` still imports `WsServer`, stub it out temporarily — we'll rewrite properly in Phase G:

In `crates/minos-daemon/src/handle.rs`, remove `use minos_transport::WsServer;`. If fields use it, this phase leaves the crate non-compiling; acceptable for this intermediate commit — the next task repairs by adding `AuthHeaders` and laying the relay-client groundwork. However the commit must leave the workspace in a compilable state, so:

**Revised approach**: this task only deletes `server.rs` and its in-crate tests + `lib.rs` re-export. `minos-daemon`'s import of `WsServer` gets commented out (with a TODO pointing to Task G.1) and `start_autobind` is stubbed to `unimplemented!()` — this is the deliberate checkpoint break from spec §5.2 step 2.

Edit `crates/minos-daemon/src/handle.rs`:
- Delete the `use minos_transport::WsServer;` line (or related).
- Replace the body of `start_autobind` with `unimplemented!("migrated to relay-client in Phase G")`.
- Replace the body of `start_on_port_range` with `unimplemented!("deleted — replaced by relay-client start(..)")`.
- Replace the body of `start(cfg)` with `unimplemented!("...")` (retained signature; body gone).
- Comment out the `WsServer` field in `DaemonInner` and any code that uses it with a TODO.

This commit intentionally breaks `DaemonHandle::start*`. Tests that invoke `start_autobind` or `start` will panic if run.

- [ ] **Step 5: Disable affected tests with `#[ignore]`**

Find every test function that calls `start_autobind`, `start_on_port_range`, or `start(cfg)`:

```bash
grep -rn "start_autobind\|start_on_port_range" crates/minos-daemon/tests
```

In each test file, mark the function with `#[ignore = "Phase G: re-enable after DaemonHandle::start(RelayConfig, ...) lands"]`.

- [ ] **Step 6: Build + run**

```bash
cargo build -p minos-daemon
cargo test -p minos-transport
cargo test -p minos-daemon --lib  # lib tests should still pass; integration tests skipped
```

Expected: builds; lib tests pass; integration tests show "ignored".

- [ ] **Step 7: Commit**

```bash
git add -u
git commit -m "refactor(transport): delete server.rs; stub daemon start paths for Phase G"
```

---

### Task D.2: Add `AuthHeaders` to `auth.rs`

**Files:**
- Modify: `crates/minos-transport/src/auth.rs`
- Test: in-file

- [ ] **Step 1: Write failing test**

Append to `crates/minos-transport/src/auth.rs`:

```rust
#[cfg(test)]
mod auth_headers_tests {
    use super::AuthHeaders;
    use minos_domain::{DeviceId, DeviceRole, DeviceSecret};

    #[test]
    fn auth_headers_as_map_includes_cf_and_device_id() {
        let h = AuthHeaders {
            cf_client_id: "id".into(),
            cf_client_secret: "secret".into(),
            device_id: DeviceId::new(),
            device_secret: None,
            device_role: DeviceRole::MacHost,
        };
        let m = h.as_header_pairs();
        assert!(m.iter().any(|(k, v)| *k == "CF-Access-Client-Id" && v == "id"));
        assert!(m.iter().any(|(k, v)| *k == "CF-Access-Client-Secret" && v == "secret"));
        assert!(m.iter().any(|(k, _)| *k == "X-Device-Id"));
        assert!(m.iter().any(|(k, v)| *k == "X-Device-Role" && v == "mac-host"));
        assert!(m.iter().all(|(k, _)| *k != "X-Device-Secret"));
    }

    #[test]
    fn auth_headers_includes_secret_when_present() {
        let h = AuthHeaders {
            cf_client_id: "id".into(),
            cf_client_secret: "secret".into(),
            device_id: DeviceId::new(),
            device_secret: Some(DeviceSecret("mysecret".into())),
            device_role: DeviceRole::MacHost,
        };
        let m = h.as_header_pairs();
        assert!(m.iter().any(|(k, v)| *k == "X-Device-Secret" && v == "mysecret"));
    }
}
```

- [ ] **Step 2: Implement**

In `crates/minos-transport/src/auth.rs`:

```rust
use minos_domain::{DeviceId, DeviceRole, DeviceSecret};

#[derive(Clone, Debug)]
pub struct AuthHeaders {
    pub cf_client_id: String,
    pub cf_client_secret: String,
    pub device_id: DeviceId,
    pub device_secret: Option<DeviceSecret>,
    pub device_role: DeviceRole,
}

impl AuthHeaders {
    /// Build a list of (header name, value) pairs suitable for injection
    /// into a tungstenite request. Keeps ordering stable so tests can
    /// assert presence without flakiness.
    pub fn as_header_pairs(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![
            ("CF-Access-Client-Id", self.cf_client_id.clone()),
            ("CF-Access-Client-Secret", self.cf_client_secret.clone()),
            ("X-Device-Id", self.device_id.to_string()),
            ("X-Device-Role", self.device_role.to_string()),
        ];
        if let Some(s) = &self.device_secret {
            out.push(("X-Device-Secret", s.0.clone()));
        }
        out
    }
}
```

- [ ] **Step 3: Run**

```bash
cargo test -p minos-transport auth_headers
```

Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "feat(transport): add AuthHeaders with CF + device id/role/secret"
```

---

### Task D.3: Extend `WsClient::connect` to take `AuthHeaders`

**Files:**
- Modify: `crates/minos-transport/src/client.rs`
- Test: in-file using an in-process `axum` route that echoes headers back

- [ ] **Step 1: Write failing test with axum test server**

Append to `crates/minos-transport/src/client.rs`:

```rust
#[cfg(test)]
mod client_auth_tests {
    use super::*;
    use crate::auth::AuthHeaders;
    use minos_domain::{DeviceId, DeviceRole};
    // ... axum test fixture ...

    #[tokio::test]
    async fn client_connect_sends_cf_and_device_headers() {
        // spawn axum test server that captures headers on Upgrade request
        // and asserts CF-Access-Client-Id / X-Device-Id / X-Device-Role present
        // see tests/fixtures/axum_echo_ws.rs helper (create if needed)
        todo!("see Task D.4 for shared axum fixture")
    }
}
```

Since this needs a shared axum WS fixture, split: Task D.3 only introduces the `connect` signature change and asserts via a simpler test.

Replacement Step 1 (simpler): verify `WsClient::connect` now accepts `&AuthHeaders`:

```rust
#[cfg(test)]
#[test]
fn wsclient_connect_accepts_auth_headers_signature() {
    // Purely a compile-time check — if this compiles, the signature is right.
    fn _typecheck() {
        let _ = || async {
            use minos_domain::{DeviceId, DeviceRole};
            use url::Url;
            let auth = crate::auth::AuthHeaders {
                cf_client_id: "".into(),
                cf_client_secret: "".into(),
                device_id: DeviceId::new(),
                device_secret: None,
                device_role: DeviceRole::MacHost,
            };
            let u = Url::parse("ws://127.0.0.1:1/").unwrap();
            let _ = crate::WsClient::connect(&u, &auth).await;
        };
    }
}
```

- [ ] **Step 2: Run (expect fail — old signature)**

```bash
cargo test -p minos-transport wsclient_connect_accepts_auth
```

Expected: FAIL compile ("expected 1 argument, found 2").

- [ ] **Step 3: Change signature**

In `crates/minos-transport/src/client.rs`, change:

```rust
pub async fn connect(url: &Url) -> Result<Self, MinosError>
```

to:

```rust
pub async fn connect(url: &Url, auth: &crate::auth::AuthHeaders) -> Result<Self, MinosError>
```

Inside the body, build a tungstenite `Request` from the URL and inject `auth.as_header_pairs()` into `headers_mut()`. Sketch:

```rust
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderName};

let mut req = url.as_str().into_client_request().map_err(|e| MinosError::ConnectFailed {
    url: url.to_string(),
    source: tokio_tungstenite::tungstenite::Error::Http(
        tokio_tungstenite::tungstenite::http::Response::builder()
            .status(400).body(None).unwrap()
    ),
})?;
for (k, v) in auth.as_header_pairs() {
    req.headers_mut().insert(
        HeaderName::from_static(k),
        v.parse().unwrap(),
    );
}
// proceed with tokio_tungstenite::connect_async(req).await
```

Adjust the remainder of `connect` to handle `tokio_tungstenite` connect (move off `jsonrpsee::ws_client` if necessary; the envelope protocol is manual JSON frames, not jsonrpsee-managed, so we switch to `tokio_tungstenite` directly).

If the existing `WsClient` was wrapping `jsonrpsee::ws_client::WsClient`, this is a larger rewrite — expand Task D.3 into D.3a (rewrite to tokio-tungstenite) + D.3b (header injection). Branch per current state; the survey showed `jsonrpsee::ws_client::WsClientBuilder`, so the rewrite is needed.

- [ ] **Step 4: Update all callers of the old `WsClient::connect`**

```bash
grep -rn "WsClient::connect" crates
```

For each call site, add the second `&AuthHeaders` argument. Tests likely fabricate a trivial `AuthHeaders`; production code now passes the real one from `RelayConfig` (wired in Phase G).

- [ ] **Step 5: Run**

```bash
cargo test -p minos-transport
cargo build -p minos-daemon
```

Both should compile. Tests may be limited pending Task D.4 axum fixture.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "refactor(transport): WsClient::connect takes AuthHeaders, switch to tokio-tungstenite"
```

---

### Task D.4: Add axum-based fake-relay test fixture + close-code mapping tests

**Files:**
- Create: `crates/minos-transport/tests/fake_relay.rs` (shared fixture)
- Test: `crates/minos-transport/tests/client_close_codes.rs`

- [ ] **Step 1: Create fake-relay fixture**

`crates/minos-transport/tests/fake_relay.rs`:

```rust
//! Minimal axum WS endpoint that asserts incoming headers and responds
//! with a caller-specified close code. Used by client_close_codes tests.

use axum::{
    extract::{ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade}, State},
    response::Response,
    routing::get,
    Router,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::Mutex;

pub struct FakeRelay {
    pub addr: SocketAddr,
    pub captured_headers: Arc<Mutex<Vec<(String, String)>>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl FakeRelay {
    pub async fn start_reject_with_401() -> Self {
        // Binds to 127.0.0.1:0; returns immediately with a handle; any WS
        // attempt receives HTTP 401 (CF edge simulation).
        todo!("implement via axum Router + Upgrade that returns 401")
    }

    pub async fn start_close_with(close_code: u16) -> Self {
        // Accepts upgrade, immediately closes with the given code.
        todo!("")
    }

    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown_tx.take() { let _ = tx.send(()); }
    }
}
```

Implement the two fixtures (`start_reject_with_401`, `start_close_with`) in ~30 lines each using axum's `ws::WebSocketUpgrade` and `CloseFrame`. See `crates/minos-relay/tests/` for an existing example pattern.

- [ ] **Step 2: Write failing tests**

`crates/minos-transport/tests/client_close_codes.rs`:

```rust
mod fake_relay;

use fake_relay::FakeRelay;
use minos_domain::{DeviceId, DeviceRole, MinosError};
use minos_transport::auth::AuthHeaders;
use minos_transport::WsClient;
use url::Url;

fn auth() -> AuthHeaders {
    AuthHeaders {
        cf_client_id: "id".into(),
        cf_client_secret: "secret".into(),
        device_id: DeviceId::new(),
        device_secret: None,
        device_role: DeviceRole::MacHost,
    }
}

#[tokio::test]
async fn http_401_at_upgrade_maps_to_cf_auth_failed() {
    let relay = FakeRelay::start_reject_with_401().await;
    let url = Url::parse(&format!("ws://{}/devices", relay.addr)).unwrap();
    let err = WsClient::connect(&url, &auth()).await.unwrap_err();
    assert!(matches!(err, MinosError::CfAuthFailed { .. }));
    relay.stop().await;
}

#[tokio::test]
async fn ws_close_4401_maps_to_device_not_trusted() {
    let relay = FakeRelay::start_close_with(4401).await;
    let url = Url::parse(&format!("ws://{}/devices", relay.addr)).unwrap();
    let client = WsClient::connect(&url, &auth()).await.unwrap();
    let err = client.next_message().await.unwrap_err();
    assert!(matches!(err, MinosError::DeviceNotTrusted { .. }));
    relay.stop().await;
}

#[tokio::test]
async fn ws_close_4400_maps_to_envelope_version_unsupported() {
    let relay = FakeRelay::start_close_with(4400).await;
    let url = Url::parse(&format!("ws://{}/devices", relay.addr)).unwrap();
    let client = WsClient::connect(&url, &auth()).await.unwrap();
    let err = client.next_message().await.unwrap_err();
    assert!(matches!(err, MinosError::EnvelopeVersionUnsupported { .. }));
    relay.stop().await;
}
```

- [ ] **Step 3: Run, verify failure**

```bash
cargo test -p minos-transport --test client_close_codes
```

Expected: FAIL (either compile or runtime depending on current client impl).

- [ ] **Step 4: Implement close-code mapping in `WsClient`**

In `crates/minos-transport/src/client.rs`, in the receive loop / upgrade response handler, map:
- HTTP 401 at upgrade → `MinosError::CfAuthFailed { message }` with the body if present.
- `CloseFrame { code: 4401, .. }` → `MinosError::DeviceNotTrusted { .. }` (read device_id from auth).
- `CloseFrame { code: 4400, .. }` → `MinosError::EnvelopeVersionUnsupported { version: 1 }`.
- `CloseFrame { code: 4409, .. }` → `MinosError::ConnectionStateMismatch { expected, actual }` from the close reason.
- `CloseFrame { code: 1001, .. }` → surface as "server shutdown" via an event channel (see Phase E).
- `CloseFrame { code: 1000, .. }` → normal shutdown; no error.

Requires adding a `next_message` (or similar) method on `WsClient` for test-level introspection. For production, this is folded into the relay-client task (Phase E).

- [ ] **Step 5: Run tests**

```bash
cargo test -p minos-transport --test client_close_codes
```

Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "feat(transport): map HTTP 401 and WS close codes to MinosError"
```

---

### Task D.5: Phase D closer

- [ ] **Step 1: Run `cargo xtask check-all`**

Expected: green (except `minos-daemon` `start_*` is `unimplemented!()` — integration tests using them are `#[ignore]`d since D.1).

---

## Phase E — Envelope dispatcher + relay-client task

Goal: in `minos-daemon`, write the WS client task that dispatches incoming envelopes (LocalRpcResponse correlation, Event → state_tx, Forwarded → jsonrpsee server impl).

### Task E.1: Add `relay_client.rs` skeleton

**Files:**
- Create: `crates/minos-daemon/src/relay_client.rs`
- Modify: `crates/minos-daemon/src/lib.rs`

- [ ] **Step 1: Write skeleton**

`crates/minos-daemon/src/relay_client.rs`:

```rust
//! Outbound WSS client task. Owns the relay connection, dispatches
//! envelopes, manages reconnect backoff, and updates the two state
//! channels (RelayLinkState + PeerState). See spec §6.

use crate::{
    config::RelayConfig,
    local_state::LocalState,
    relay_pairing::{PeerRecord, RelayQrPayload},
};
use minos_domain::{
    DeviceId, DeviceRole, DeviceSecret, MinosError, PeerState, RelayLinkState,
};
use minos_protocol::envelope::{Envelope, EventKind, LocalRpcMethod};
use minos_transport::{auth::AuthHeaders, WsClient};
use std::{
    collections::HashMap,
    sync::{atomic::AtomicU64, Arc},
    time::Duration,
};
use tokio::sync::{oneshot, watch, Mutex};
use url::Url;

pub struct RelayClient {
    link_tx: watch::Sender<RelayLinkState>,
    peer_tx: watch::Sender<PeerState>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Envelope>>>>,
    next_id: AtomicU64,
    // handle to the running task so we can stop it
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl RelayClient {
    /// Spawn the client task. Returns immediately with the
    /// `RelayClient` handle; the task runs in the background.
    pub fn spawn(
        config: RelayConfig,
        self_device_id: DeviceId,
        peer: Option<PeerRecord>,
        secret: Option<DeviceSecret>,
        backend_url: &str,
    ) -> (Arc<Self>, watch::Receiver<RelayLinkState>, watch::Receiver<PeerState>) {
        let initial_link = RelayLinkState::Connecting { attempt: 0 };
        let initial_peer = match peer.as_ref() {
            None => PeerState::Unpaired,
            Some(p) => PeerState::Paired {
                peer_id: p.device_id,
                peer_name: p.name.clone(),
                online: false,
            },
        };
        let (link_tx, link_rx) = watch::channel(initial_link);
        let (peer_tx, peer_rx) = watch::channel(initial_peer);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let pending = Arc::new(Mutex::new(HashMap::new()));

        let client = Arc::new(Self {
            link_tx: link_tx.clone(),
            peer_tx: peer_tx.clone(),
            pending: pending.clone(),
            next_id: AtomicU64::new(1),
            shutdown_tx: Some(shutdown_tx),
        });

        let url = backend_url.to_string();
        let auth_template = AuthHeaders {
            cf_client_id: config.cf_client_id,
            cf_client_secret: config.cf_client_secret,
            device_id: self_device_id,
            device_secret: secret,
            device_role: DeviceRole::MacHost,
        };

        tokio::spawn(run_task(
            url,
            auth_template,
            link_tx,
            peer_tx,
            pending,
            shutdown_rx,
        ));

        (client, link_rx, peer_rx)
    }

    pub async fn request_pairing_token(
        &self,
        _mac_name: &str,
    ) -> Result<RelayQrPayload, MinosError> {
        // See Task E.4
        Err(MinosError::RelayInternal {
            message: "not yet implemented".into(),
        })
    }

    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        // See Task E.5
        Err(MinosError::RelayInternal {
            message: "not yet implemented".into(),
        })
    }

    pub async fn stop(&self) {
        // take + drop shutdown_tx to signal
        // in practice, move to an interior Mutex<Option<Sender>>
    }
}

async fn run_task(
    url: String,
    auth: AuthHeaders,
    link_tx: watch::Sender<RelayLinkState>,
    peer_tx: watch::Sender<PeerState>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Envelope>>>>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut attempt: u32 = 0;
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => { break; }
            res = connect_and_run(&url, &auth, &link_tx, &peer_tx, &pending) => {
                match res {
                    Ok(_) => { /* clean close; back to Connecting{0} and retry */ }
                    Err(e) => {
                        tracing::warn!(error = %e, "relay loop ended");
                    }
                }
            }
        }
        attempt = attempt.saturating_add(1);
        let _ = link_tx.send(RelayLinkState::Connecting { attempt });
        let delay = backoff_delay(attempt);
        tokio::time::sleep(delay).await;
    }
    let _ = link_tx.send(RelayLinkState::Disconnected);
}

fn backoff_delay(attempt: u32) -> Duration {
    let secs = match attempt {
        0 | 1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        5 => 16,
        _ => 30,
    };
    Duration::from_secs(secs)
}

async fn connect_and_run(
    url: &str,
    auth: &AuthHeaders,
    link_tx: &watch::Sender<RelayLinkState>,
    peer_tx: &watch::Sender<PeerState>,
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<Envelope>>>>,
) -> Result<(), MinosError> {
    let u = Url::parse(url).map_err(|e| MinosError::ConnectFailed {
        url: url.to_string(),
        source: tokio_tungstenite::tungstenite::Error::Url(e.to_string().into()),
    })?;
    let _client = WsClient::connect(&u, auth).await?;
    let _ = link_tx.send(RelayLinkState::Connected);
    // dispatch loop — see Task E.2
    Ok(())
}
```

- [ ] **Step 2: Register in lib.rs**

Add `pub mod relay_client;` to `crates/minos-daemon/src/lib.rs`.

- [ ] **Step 3: Compile-check**

```bash
cargo build -p minos-daemon
```

Expected: compiles (may produce unused-variable warnings, clippy will flag them — fix inline).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/relay_client.rs crates/minos-daemon/src/lib.rs
git commit -m "feat(daemon): scaffold relay_client with RelayClient handle + run_task"
```

---

### Task E.2: Implement envelope dispatch loop with correlation map

**Files:**
- Modify: `crates/minos-daemon/src/relay_client.rs`
- Test: `crates/minos-daemon/tests/relay_client_dispatch.rs`

- [ ] **Step 1: Write failing integration test using in-process relay**

`crates/minos-daemon/tests/relay_client_dispatch.rs`:

```rust
//! Runs a real minos-relay in-process on an ephemeral port; wires a
//! RelayClient at it; asserts state transitions and dispatch.

use minos_daemon::{
    config::RelayConfig,
    local_state::LocalState,
    relay_client::RelayClient,
};
use minos_domain::{DeviceRole, PeerState, RelayLinkState};
use std::time::Duration;

// Spawn an in-process relay. Crate minos-relay exposes a
// `tests::spawn_for_test()` helper — see crates/minos-relay/tests/*
// for the existing pattern.
async fn spawn_relay() -> (String, tokio::task::JoinHandle<()>) {
    todo!("wire via minos_relay::test_util::spawn_on_random_port")
}

#[tokio::test(flavor = "multi_thread")]
async fn connects_and_becomes_connected() {
    let (url, _relay) = spawn_relay().await;
    let state = LocalState {
        self_device_id: minos_domain::DeviceId::new(),
        peer: None,
    };
    let config = RelayConfig::new("dev".into(), "dev".into());
    let (_client, mut link_rx, _peer_rx) =
        RelayClient::spawn(config, state.self_device_id, None, None, &url);
    // wait up to 3s for Connected
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if matches!(*link_rx.borrow(), RelayLinkState::Connected) { break; }
        if std::time::Instant::now() > deadline { panic!("did not connect"); }
        link_rx.changed().await.unwrap();
    }
}
```

- [ ] **Step 2: Run, observe failure**

```bash
cargo test -p minos-daemon --test relay_client_dispatch
```

Expected: FAIL either at `todo!` or the connect-and-run loop not emitting `Connected` (scaffold only sends `link_tx.send(Connected)` but the inner dispatch loop is missing, and `WsClient::connect` may immediately return error).

- [ ] **Step 3: Implement the dispatch loop**

In `crates/minos-daemon/src/relay_client.rs` `connect_and_run`:

```rust
async fn connect_and_run(
    url: &str,
    auth: &AuthHeaders,
    link_tx: &watch::Sender<RelayLinkState>,
    peer_tx: &watch::Sender<PeerState>,
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<Envelope>>>>,
) -> Result<(), MinosError> {
    let u = url::Url::parse(url).map_err(/* ... */)?;
    let client = WsClient::connect(&u, auth).await?;
    let _ = link_tx.send(RelayLinkState::Connected);

    // Split into sink + stream
    let (mut sink, mut stream) = client.split();

    // TODO: send loop reads from an mpsc<Envelope> owned by RelayClient
    // for outbound LocalRpc frames.

    while let Some(msg) = stream.next().await {
        let text = match msg {
            Ok(tokio_tungstenite::tungstenite::Message::Text(t)) => t,
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => return Ok(()),
            Ok(_) => continue,
            Err(e) => return Err(MinosError::ConnectFailed {
                url: url.to_string(),
                source: e,
            }),
        };
        let env: Envelope = match serde_json::from_str(&text) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "malformed envelope; dropping");
                continue;
            }
        };
        dispatch(env, peer_tx, pending).await;
    }
    Ok(())
}

async fn dispatch(
    env: Envelope,
    peer_tx: &watch::Sender<PeerState>,
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<Envelope>>>>,
) {
    match env {
        Envelope::LocalRpcResponse { id, .. } => {
            let tx = pending.lock().await.remove(&id);
            if let Some(tx) = tx { let _ = tx.send(env); }
        }
        Envelope::Event { event, .. } => match event {
            EventKind::Paired { peer_device_id, peer_name, .. } => {
                let _ = peer_tx.send(PeerState::Paired {
                    peer_id: peer_device_id,
                    peer_name,
                    online: true,
                });
            }
            EventKind::PeerOnline { peer_device_id } => {
                peer_tx.send_if_modified(|s| {
                    if let PeerState::Paired { peer_id, online, .. } = s {
                        if *peer_id == peer_device_id { *online = true; return true; }
                    }
                    false
                });
            }
            EventKind::PeerOffline { peer_device_id } => {
                peer_tx.send_if_modified(|s| {
                    if let PeerState::Paired { peer_id, online, .. } = s {
                        if *peer_id == peer_device_id { *online = false; return true; }
                    }
                    false
                });
            }
            EventKind::Unpaired => {
                let _ = peer_tx.send(PeerState::Unpaired);
            }
            EventKind::ServerShutdown => {
                // Let caller re-enter Connecting{0}; just return Ok
            }
        },
        Envelope::Forwarded { payload, .. } => {
            // Phase H: dispatch to local jsonrpsee server impl
            tracing::info!(?payload, "forwarded payload (not yet handled)");
        }
        _ => {}
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minos-daemon --test relay_client_dispatch
```

Expected: PASS (1 test for now; more added in E.3, E.4, E.5).

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(daemon): relay-client dispatch loop with event → peer_tx mapping"
```

---

### Task E.3: Implement outbound sink for LocalRpc requests

**Files:**
- Modify: `crates/minos-daemon/src/relay_client.rs`
- Test: extend `relay_client_dispatch.rs`

- [ ] **Step 1: Add sink mpsc channel + `send_local_rpc` method**

In `RelayClient`, add field:

```rust
out_tx: tokio::sync::mpsc::Sender<Envelope>,
```

Initialize in `spawn`:

```rust
let (out_tx, out_rx) = tokio::sync::mpsc::channel::<Envelope>(32);
// pass out_rx into run_task / connect_and_run
```

Add to `RelayClient`:

```rust
/// Send a LocalRpc and await its correlated response.
pub async fn send_local_rpc(
    &self,
    method: LocalRpcMethod,
    params: serde_json::Value,
) -> Result<serde_json::Value, MinosError> {
    let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    self.pending.lock().await.insert(id, tx);
    let env = Envelope::LocalRpc { v: 1, id, method, params };
    self.out_tx.send(env).await.map_err(|_| MinosError::RelayInternal {
        message: "relay task is gone".into(),
    })?;
    let resp = tokio::time::timeout(Duration::from_secs(10), rx).await
        .map_err(|_| MinosError::RelayInternal { message: "local rpc timeout".into() })?
        .map_err(|_| MinosError::RelayInternal { message: "oneshot dropped".into() })?;
    // Extract result/error from resp
    todo!("unwrap LocalRpcResponse, map err variants to MinosError")
}
```

Update `connect_and_run` to drain `out_rx` into the sink via `tokio::select!` alongside the stream read.

- [ ] **Step 2: Test `send_local_rpc(Ping)` round trip**

Add to `relay_client_dispatch.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn ping_local_rpc_returns_ok_true() {
    let (url, _relay) = spawn_relay().await;
    let config = RelayConfig::new("dev".into(), "dev".into());
    let did = minos_domain::DeviceId::new();
    let (client, mut link_rx, _) = RelayClient::spawn(config, did, None, None, &url);
    // wait for Connected
    while !matches!(*link_rx.borrow(), RelayLinkState::Connected) {
        link_rx.changed().await.unwrap();
    }
    let result = client.send_local_rpc(
        minos_protocol::envelope::LocalRpcMethod::Ping,
        serde_json::json!({}),
    ).await.unwrap();
    assert_eq!(result["ok"], true);
}
```

- [ ] **Step 3: Run**

```bash
cargo test -p minos-daemon --test relay_client_dispatch ping_local_rpc
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "feat(daemon): add send_local_rpc with correlation + mpsc outbound"
```

---

### Task E.4: Implement `request_pairing_token` → `RelayQrPayload`

- [ ] **Step 1: Write failing test**

In `relay_client_dispatch.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn request_pairing_token_returns_qr_with_backend_url() {
    let (url, _relay) = spawn_relay().await;
    let config = RelayConfig::new("dev".into(), "dev".into());
    let did = minos_domain::DeviceId::new();
    let (client, mut link_rx, _) = RelayClient::spawn(config, did, None, None, &url);
    while !matches!(*link_rx.borrow(), RelayLinkState::Connected) {
        link_rx.changed().await.unwrap();
    }
    let qr = client.request_pairing_token("fannnzhang MacBook").await.unwrap();
    assert_eq!(qr.v, 1);
    assert_eq!(qr.backend_url, url);
    assert_eq!(qr.mac_display_name, "fannnzhang MacBook");
    assert!(!qr.token.0.is_empty());
}
```

- [ ] **Step 2: Implement**

In `RelayClient::request_pairing_token`:

```rust
pub async fn request_pairing_token(&self, mac_name: &str) -> Result<RelayQrPayload, MinosError> {
    let raw = self.send_local_rpc(
        LocalRpcMethod::RequestPairingToken,
        serde_json::json!({}),
    ).await?;
    let token_str = raw.get("token").and_then(|v| v.as_str())
        .ok_or_else(|| MinosError::RelayInternal {
            message: "pairing token response missing token".into(),
        })?;
    Ok(RelayQrPayload {
        v: 1,
        backend_url: self.backend_url.clone(),
        token: minos_domain::PairingToken(token_str.into()),
        mac_display_name: mac_name.into(),
    })
}
```

(Add `backend_url: String` field to `RelayClient`; pass it from `spawn`.)

- [ ] **Step 3: Run**

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "feat(daemon): implement request_pairing_token → RelayQrPayload"
```

---

### Task E.5: Implement `forget_peer`

- [ ] **Step 1: Write failing test**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn forget_peer_after_pair_unsets_peer_state() {
    // 1. Mac connects
    // 2. fake-peer (via a second WsClient in the test) consumes the token
    // 3. Mac receives Paired event
    // 4. Mac calls forget_peer
    // 5. Mac receives Unpaired event → peer_state == Unpaired
    todo!("compose via relay's test helpers; may be easier in E2E file")
}
```

Implementation detail: this test needs both Mac-side and iOS-side client connections; simplest path is to open two `WsClient`s in the test with different `DeviceRole`s.

- [ ] **Step 2: Implement `forget_peer`**

```rust
pub async fn forget_peer(&self) -> Result<(), MinosError> {
    let _ = self.send_local_rpc(
        LocalRpcMethod::ForgetPeer,
        serde_json::json!({}),
    ).await?;
    // The relay pushes Event{type:Unpaired} asynchronously; the dispatcher
    // already handles that in Task E.2. Return Ok here.
    Ok(())
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p minos-daemon --test relay_client_dispatch forget_peer
git add -u
git commit -m "feat(daemon): implement forget_peer LocalRpc"
```

---

### Task E.6: Phase E closer

- [ ] **Step 1: Run `cargo xtask check-all`**

Expected: green. Note the `linux` CI job runs this integration test; make sure the in-process relay fixture works on Linux (it should — `minos-relay` is cross-platform).

---

## Phase F — `DaemonHandle` surgery

Goal: rewire `DaemonHandle` to own `RelayClient`; replace `start_autobind` with `start(RelayConfig, self_device_id, peer, secret, mac_name)`; add `subscribe_relay_link` + `subscribe_peer`; delete Tailscale + file_store.

### Task F.1: Rewrite `DaemonInner` to own `RelayClient` + observers

**Files:**
- Modify: `crates/minos-daemon/src/handle.rs`
- Modify: `crates/minos-daemon/src/subscription.rs` (observer trait additions)
- Test: rewrite existing tests that were `#[ignore]`d in D.1

- [ ] **Step 1: Add two new observer traits**

In `crates/minos-daemon/src/subscription.rs`, next to existing `ConnectionStateObserver`:

```rust
#[cfg_attr(feature = "uniffi", uniffi::export(callback_interface))]
pub trait RelayLinkStateObserver: Send + Sync {
    fn on_state(&self, state: minos_domain::RelayLinkState);
}

#[cfg_attr(feature = "uniffi", uniffi::export(callback_interface))]
pub trait PeerStateObserver: Send + Sync {
    fn on_state(&self, state: minos_domain::PeerState);
}
```

Note: kept alongside `ConnectionStateObserver` for now — removed in F.5.

- [ ] **Step 2: Rewrite `DaemonInner`**

Replace `DaemonInner` fields in `crates/minos-daemon/src/handle.rs` with:

```rust
struct DaemonInner {
    relay: Arc<crate::relay_client::RelayClient>,
    link_rx: watch::Receiver<minos_domain::RelayLinkState>,
    peer_rx: watch::Receiver<minos_domain::PeerState>,
    self_device_id: DeviceId,
    peer: Arc<Mutex<Option<PeerRecord>>>,
    local_state_path: PathBuf,
    mac_name: String,
    agent_state: ... /* existing agent-runtime fields untouched */,
}
```

Implement `DaemonHandle`:

```rust
impl DaemonHandle {
    pub async fn start(
        config: crate::config::RelayConfig,
        self_device_id: DeviceId,
        peer: Option<PeerRecord>,
        secret: Option<DeviceSecret>,
        mac_name: String,
    ) -> Result<Arc<Self>, MinosError> {
        let path = LocalState::default_path();
        let (relay, link_rx, peer_rx) = crate::relay_client::RelayClient::spawn(
            config,
            self_device_id,
            peer.clone(),
            secret,
            crate::config::BACKEND_URL,
        );
        Ok(Arc::new(DaemonHandle {
            inner: Arc::new(DaemonInner {
                relay,
                link_rx,
                peer_rx,
                self_device_id,
                peer: Arc::new(Mutex::new(peer)),
                local_state_path: path,
                mac_name,
                agent_state: /* existing init */,
            }),
        }))
    }

    pub fn current_relay_link(&self) -> minos_domain::RelayLinkState {
        *self.inner.link_rx.borrow()
    }

    pub fn current_peer(&self) -> minos_domain::PeerState {
        self.inner.peer_rx.borrow().clone()
    }

    pub fn subscribe_relay_link(
        &self,
        observer: Arc<dyn crate::subscription::RelayLinkStateObserver>,
    ) -> Arc<Subscription> {
        // clone rx, spawn task, return cancel-able Subscription
        // similar to the existing `subscribe` implementation
        let mut rx = self.inner.link_rx.clone();
        let sub = Arc::new(Subscription::new());
        let canceled = sub.cancel_token();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = canceled.cancelled() => break,
                    r = rx.changed() => {
                        if r.is_err() { break; }
                        observer.on_state(*rx.borrow());
                    }
                }
            }
        });
        sub
    }

    pub fn subscribe_peer(
        &self,
        observer: Arc<dyn crate::subscription::PeerStateObserver>,
    ) -> Arc<Subscription> {
        /* symmetric */
    }

    pub async fn pairing_qr(&self) -> Result<RelayQrPayload, MinosError> {
        self.inner.relay.request_pairing_token(&self.inner.mac_name).await
    }

    pub fn current_trusted_device(&self) -> Result<Option<PeerRecord>, MinosError> {
        Ok(self.inner.peer.blocking_lock().clone())
    }

    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        self.inner.relay.forget_peer().await?;
        *self.inner.peer.lock().await = None;
        // Persist:
        let ls = LocalState {
            self_device_id: self.inner.self_device_id,
            peer: None,
        };
        ls.save(&self.inner.local_state_path)?;
        // Keychain delete:
        #[cfg(target_os = "macos")]
        {
            let _ = crate::KeychainTrustedDeviceStore.delete();
        }
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), MinosError> {
        self.inner.relay.stop().await;
        Ok(())
    }

    // Agent-runtime methods unchanged (keep existing impls).
}
```

- [ ] **Step 3: Update callers in main.rs**

Update `crates/minos-daemon/src/main.rs` doctor CLI:
- Remove the `tailscale:` line in the diagnostic output.
- Replace `start_autobind` call (if any) with `start(RelayConfig::new(...), ...)` — for CLI, read CF creds from env, local-state from disk.

- [ ] **Step 4: Run**

```bash
cargo build -p minos-daemon
cargo test -p minos-daemon --lib
```

Expected: builds; lib tests pass.

- [ ] **Step 5: Un-ignore and update integration tests**

```bash
grep -rn "#\[ignore = \"Phase G" crates/minos-daemon/tests
```

For each ignored test: either delete (if it was purely Tailscale-specific like `autobind.rs`) or rewrite to use the new `start(RelayConfig, ...)` flow.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "refactor(daemon): rewire DaemonHandle to own RelayClient + dual state subs"
```

---

### Task F.2: Delete `tailscale.rs`, `file_store.rs`, and related re-exports

**Files:**
- Delete: `crates/minos-daemon/src/tailscale.rs`
- Delete: `crates/minos-daemon/src/file_store.rs`
- Delete: `crates/minos-daemon/tests/autobind.rs`
- Modify: `crates/minos-daemon/src/lib.rs` (remove re-exports)

- [ ] **Step 1: Delete files**

```bash
rm crates/minos-daemon/src/tailscale.rs
rm crates/minos-daemon/src/file_store.rs
rm crates/minos-daemon/tests/autobind.rs
```

- [ ] **Step 2: Remove module declarations and re-exports from `lib.rs`**

In `crates/minos-daemon/src/lib.rs`, remove:
- `pub mod tailscale;`
- `pub use tailscale::discover_ip as discover_tailscale_ip;`
- `pub use tailscale::discover_ip_with_reason as discover_tailscale_ip_with_reason;`
- `pub mod file_store;`
- `pub use file_store::FilePairingStore;`

- [ ] **Step 3: Grep-verify no lingering references**

```bash
grep -rn "tailscale\|discover_tailscale_ip\|FilePairingStore\|file_store" crates/minos-daemon crates/minos-ffi-uniffi
```

Expected: zero hits outside of `.xlog` files or comments explicitly noting "removed" — remove any such comments too.

- [ ] **Step 4: Build + run full daemon tests**

```bash
cargo build -p minos-daemon
cargo test -p minos-daemon
```

Expected: green.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(daemon): delete tailscale.rs, file_store.rs, autobind test"
```

---

### Task F.3: Remove jsonrpsee server `pair` + `health` trait impls

**Files:**
- Modify: `crates/minos-daemon/src/rpc_server.rs`

- [ ] **Step 1: Read current trait**

```bash
grep -n "async fn " crates/minos-daemon/src/rpc_server.rs
```

Review which methods are implemented.

- [ ] **Step 2: Remove `pair` and `health` impl methods**

In `crates/minos-daemon/src/rpc_server.rs`, delete the `async fn pair(...)` and `async fn health(...)` method bodies inside the `impl MinosRpcServer for RpcServerImpl` block. These are now the relay's responsibility.

However, the `MinosRpcServer` trait (in `minos-protocol`) still declares them. Options:
- Split the trait: move `pair`/`health` into a relay-side-only trait.
- Keep the trait; Mac-side impl returns `MinosError::Unauthorized { reason: "handled by relay" }`.

For this plan, **keep the impls but error out cleanly** — fewer trait churn, and the relay-client dispatcher never forwards these methods to the Mac anyway:

```rust
async fn pair(&self, _req: PairRequest) -> RpcResult<PairResponse> {
    Err(rpc_err(MinosError::Unauthorized {
        reason: "pair handled by relay, not host".into(),
    }))
}

async fn health(&self) -> RpcResult<HealthResponse> {
    // Health is still useful — it's how peers ask "can you hear me?".
    // Keep returning a thin Ok.
    Ok(HealthResponse { ok: true, mac_name: /* from RpcServerImpl state */ })
}
```

Actually `health` stays — it's a peer-invoked check that the Mac is responsive, and `list_clis` / agent methods need to remain callable via `Forwarded` dispatch. This task just removes `pair` because pairing now goes through LocalRpc-to-relay, not peer-to-peer.

- [ ] **Step 3: Build**

```bash
cargo build -p minos-daemon
```

If `MinosRpcServer` requires `pair` to be implemented, stub it as above. If not, delete it.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "refactor(daemon): pair() RPC now returns Unauthorized (relay owns pairing)"
```

---

### Task F.4: Wire `RpcServerImpl` to be invoked from envelope `Forwarded` dispatch

**Files:**
- Modify: `crates/minos-daemon/src/relay_client.rs` (`dispatch` fn)
- Modify: `crates/minos-daemon/src/rpc_server.rs` (expose a call-by-method-name entry)

- [ ] **Step 1: Add `invoke_forwarded` helper**

In `crates/minos-daemon/src/rpc_server.rs`, add a free function that parses a JSON-RPC 2.0 payload, dispatches to the matching `MinosRpcServer` method, and returns the JSON-RPC response. Reuse `jsonrpsee`'s `RpcModule::raw_json_request()` if available, otherwise write a small match.

- [ ] **Step 2: Update `dispatch` in `relay_client.rs`**

In the `Envelope::Forwarded { payload, .. }` arm:

```rust
let response = crate::rpc_server::invoke_forwarded(&payload, &self.rpc_server_impl).await;
let out = Envelope::Forward { v: 1, payload: response };
// push out via out_tx
```

This requires `RelayClient` to hold a reference to `RpcServerImpl`; add it at `spawn` time (Task F.1 adjustment).

- [ ] **Step 3: Integration test: Forwarded `list_clis` round-trip**

Extend `relay_client_dispatch.rs` with a test that has a second fake-peer `WsClient` send a `Forward{ jsonrpc, method: list_clis }` and asserts the Mac's response arrives back as `Forwarded`.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "feat(daemon): dispatch Forwarded JSON-RPC to local MinosRpcServer impl"
```

---

### Task F.5: Remove old `ConnectionStateObserver` and `subscribe()` + related dead state

**Files:**
- Modify: `crates/minos-daemon/src/subscription.rs` (keep `Subscription`, remove old observer trait)
- Modify: `crates/minos-daemon/src/handle.rs` (remove old `subscribe`, `events_stream`, `current_state`, `host`, `port`, `addr`)
- Check: `crates/minos-mobile` references — if `ConnectionStateObserver` is shared, see divergence note (keep if iOS imports it)

- [ ] **Step 1: Grep iOS-side usage**

```bash
grep -rn "ConnectionStateObserver\|events_stream\|current_state" crates/minos-mobile crates/minos-ffi-frb apps/mobile
```

If hits exist, **keep** `ConnectionStateObserver` in `minos-daemon::subscription` for now (iOS still uses it via `MobileClient`'s similar-shaped subscribe). Only `DaemonHandle` methods are removed; the trait stays.

- [ ] **Step 2: Remove `DaemonHandle` methods**

In `crates/minos-daemon/src/handle.rs`, remove:
- `pub fn host(&self) -> String`
- `pub fn port(&self) -> u16`
- `pub fn addr(&self) -> SocketAddr` (if public)
- `pub fn current_state(&self) -> ConnectionState`
- `pub fn events_stream(&self) -> watch::Receiver<ConnectionState>`
- `pub fn subscribe(&self, observer: Arc<dyn ConnectionStateObserver>) -> Arc<Subscription>`

- [ ] **Step 3: Build + update callers**

```bash
cargo build 2>&1 | grep error
```

Check every error for callers of the removed methods. Most will be in `apps/macos` Swift layer — we'll fix those in Phase K.

- [ ] **Step 4: For now, if Rust callers break, stub-ignore integration tests until Phase K wires Swift**

Add `#[ignore = "Phase K: Swift not yet rewired"]` to any integration test that exercised the old API and has no Rust-side fix path.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "refactor(daemon): drop host/port/addr/current_state/subscribe from DaemonHandle"
```

---

### Task F.6: Phase F closer

- [ ] **Step 1: Run `cargo xtask check-all`**

Swift leg will fail because UniFFI-generated Swift still references `DaemonHandle.currentState`, `DaemonHandle.host`, etc. That's expected — Phase I regenerates the bindings. Gate is "Rust green, Swift broken" at this point.

- [ ] **Step 2: Gate: run Rust subset**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all three green. If any red, fix before Phase G.

---

## Phase G — UniFFI shim regen

### Task G.1: Update `minos-ffi-uniffi/src/lib.rs`

**Files:**
- Modify: `crates/minos-ffi-uniffi/src/lib.rs`

- [ ] **Step 1: Remove `discover_tailscale_ip` free function**

Delete:

```rust
#[uniffi::export]
pub async fn discover_tailscale_ip() -> Option<String> { ... }
```

- [ ] **Step 2: Update re-exports**

Remove `QrPayload`, `TrustedDevice`, `ConnectionState` from the `pub use` line.

Add `RelayLinkState`, `PeerState`, `DeviceSecret`, `RelayQrPayload`, `PeerRecord`, `RelayLinkStateObserver`, `PeerStateObserver`:

```rust
pub use minos_daemon::{
    AgentState, AgentStateObserver, DaemonHandle, Subscription,
    PeerRecord, RelayQrPayload,
    subscription::{RelayLinkStateObserver, PeerStateObserver},
};
pub use minos_domain::{
    AgentDescriptor, AgentName, AgentStatus, DeviceId, DeviceSecret,
    ErrorKind, Lang, MinosError, RelayLinkState, PeerState,
    SendUserMessageRequest, StartAgentRequest, StartAgentResponse,
};
```

(Verify `minos_domain::DeviceRole` visibility; add if Swift ever sees role values.)

- [ ] **Step 3: Run gen**

```bash
cargo xtask gen-uniffi
```

Inspect:

```bash
ls apps/macos/Minos/Generated/
grep -l "discover_tailscale_ip" apps/macos/Minos/Generated/*.swift
```

First command: Generated directory listing changed (new file for relay types?). Second: no hits.

- [ ] **Step 4: Commit regenerated Swift**

Generated files are gitignored per plan 02 convention? Check `.gitignore`:

```bash
grep -n "Generated" .gitignore
```

If generated Swift is tracked, commit it here. If gitignored, don't — `check-all` regenerates.

- [ ] **Step 5: Commit Rust shim**

```bash
git add crates/minos-ffi-uniffi/src/lib.rs
# plus apps/macos/Minos/Generated/ if tracked
git commit -m "feat(ffi-uniffi): export new observer traits + Mac-only relay types"
```

---

### Task G.2: Phase G closer

- [ ] **Step 1: Verify bindings contain new symbols**

```bash
grep -l "RelayLinkStateObserver" apps/macos/Minos/Generated/*.swift
grep -l "PeerStateObserver" apps/macos/Minos/Generated/*.swift
grep -l "RelayQrPayload" apps/macos/Minos/Generated/*.swift
grep -l "discover_tailscale_ip" apps/macos/Minos/Generated/*.swift
```

Expected: first three find hits, fourth finds nothing.

- [ ] **Step 2: Rust build succeeds**

```bash
cargo build --workspace --all-targets
```

Expected: green.

---

## Phase H — Swift infrastructure

### Task H.1: Add `KeychainRelayConfig.swift`

**Files:**
- Create: `apps/macos/Minos/Infrastructure/KeychainRelayConfig.swift`
- Test: `apps/macos/MinosTests/Infrastructure/KeychainRelayConfigTests.swift`

- [ ] **Step 1: Implement (no TDD for Swift Keychain — exercising requires real keychain)**

`apps/macos/Minos/Infrastructure/KeychainRelayConfig.swift`:

```swift
import Foundation
import Security

enum KeychainRelayConfig {
    static let service = "ai.minos.macos"
    static let accountClientId = "cf-client-id"
    static let accountClientSecret = "cf-client-secret"

    struct Creds: Equatable {
        let clientId: String
        let clientSecret: String
    }

    static func read() -> Creds? {
        guard
            let id = readItem(account: accountClientId),
            let secret = readItem(account: accountClientSecret)
        else { return nil }
        return Creds(clientId: id, clientSecret: secret)
    }

    static func write(_ creds: Creds) throws {
        try writeItem(account: accountClientId, value: creds.clientId)
        try writeItem(account: accountClientSecret, value: creds.clientSecret)
    }

    static func clear() throws {
        try deleteItem(account: accountClientId)
        try deleteItem(account: accountClientSecret)
    }

    // MARK: - Internal

    private static func readItem(account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess,
              let data = item as? Data,
              let s = String(data: data, encoding: .utf8)
        else { return nil }
        return s
    }

    private static func writeItem(account: String, value: String) throws {
        let attrs: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let valueData: [String: Any] = [
            kSecValueData as String: Data(value.utf8),
        ]
        let status = SecItemAdd(attrs.merging(valueData, uniquingKeysWith: { $1 }) as CFDictionary, nil)
        if status == errSecDuplicateItem {
            let updateStatus = SecItemUpdate(attrs as CFDictionary, valueData as CFDictionary)
            if updateStatus != errSecSuccess {
                throw KeychainError.unexpected(status: updateStatus)
            }
        } else if status != errSecSuccess {
            throw KeychainError.unexpected(status: status)
        }
    }

    private static func deleteItem(account: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            throw KeychainError.unexpected(status: status)
        }
    }

    enum KeychainError: Error {
        case unexpected(status: OSStatus)
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/macos/Minos/Infrastructure/KeychainRelayConfig.swift
git commit -m "feat(macos): add KeychainRelayConfig helper (Security.framework)"
```

---

## Phase I — Swift Domain + Application layers

### Task I.1: Add `RelayLinkState+Display.swift` and `PeerState+Display.swift`

**Files:**
- Create: `apps/macos/Minos/Domain/RelayLinkState+Display.swift`
- Create: `apps/macos/Minos/Domain/PeerState+Display.swift`
- Delete: `apps/macos/Minos/Domain/ConnectionState+Display.swift`

- [ ] **Step 1: Create RelayLinkState+Display**

```swift
import SwiftUI

extension RelayLinkState {
    var displayLabel: String {
        switch self {
        case .disconnected:
            return "未连接后端"
        case .connecting(let attempt):
            return attempt == 0 ? "正在连接后端…" : "正在重连后端 · 第 \(attempt) 次"
        case .connected:
            return "已连接后端"
        }
    }

    var iconName: String {
        switch self {
        case .disconnected: return "bolt.slash"
        case .connecting: return "bolt.circle"
        case .connected: return "bolt.circle.fill"
        }
    }

    var tint: Color {
        switch self {
        case .disconnected: return .secondary
        case .connecting: return .orange
        case .connected: return .green
        }
    }
}
```

- [ ] **Step 2: Create PeerState+Display**

```swift
import SwiftUI

extension PeerState {
    var displayLabel: String {
        switch self {
        case .unpaired: return "未配对"
        case .pairing: return "等待扫码"
        case .paired(_, let name, let online):
            return online ? "手机在线 · \(name)" : "手机离线 · \(name)"
        }
    }

    var peerName: String? {
        if case .paired(_, let name, _) = self { return name }
        return nil
    }

    var isOnline: Bool {
        if case .paired(_, _, let online) = self { return online }
        return false
    }
}
```

- [ ] **Step 3: Delete `ConnectionState+Display.swift`**

```bash
rm apps/macos/Minos/Domain/ConnectionState+Display.swift
```

- [ ] **Step 4: Commit**

```bash
git add -u apps/macos
git commit -m "feat(macos): add RelayLinkState + PeerState display extensions"
```

---

### Task I.2: Update `MinosError+Display.swift` for `CfAuthFailed`

**Files:**
- Modify: `apps/macos/Minos/Domain/MinosError+Display.swift`

- [ ] **Step 1: Add `.cfAuthFailed` arm to the `kind` switch**

```swift
case .cfAuthFailed: return .cfAuthFailed
```

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "feat(macos): map MinosError.cfAuthFailed in Display"
```

---

### Task I.3: Add `RelayLinkObserver` + `PeerObserver`, delete `ObserverAdapter.swift`

**Files:**
- Create: `apps/macos/Minos/Application/RelayLinkObserver.swift`
- Create: `apps/macos/Minos/Application/PeerObserver.swift`
- Delete: `apps/macos/Minos/Application/ObserverAdapter.swift`

- [ ] **Step 1: Write `RelayLinkObserver.swift`**

```swift
import Foundation

final class RelayLinkObserver: RelayLinkStateObserver, @unchecked Sendable {
    private let onStateChange: @Sendable (RelayLinkState) -> Void

    init(onStateChange: @escaping @Sendable (RelayLinkState) -> Void) {
        self.onStateChange = onStateChange
    }

    func onState(state: RelayLinkState) {
        onStateChange(state)
    }
}
```

- [ ] **Step 2: Write `PeerObserver.swift`**

```swift
import Foundation

final class PeerObserver: PeerStateObserver, @unchecked Sendable {
    private let onStateChange: @Sendable (PeerState) -> Void

    init(onStateChange: @escaping @Sendable (PeerState) -> Void) {
        self.onStateChange = onStateChange
    }

    func onState(state: PeerState) {
        onStateChange(state)
    }
}
```

- [ ] **Step 3: Delete old adapter**

```bash
rm apps/macos/Minos/Application/ObserverAdapter.swift
```

- [ ] **Step 4: Commit**

```bash
git add -u apps/macos
git commit -m "feat(macos): split ObserverAdapter into RelayLinkObserver + PeerObserver"
```

---

### Task I.4: Update `DaemonDriving` protocol

**Files:**
- Modify: `apps/macos/Minos/Application/DaemonDriving.swift`

- [ ] **Step 1: Rewrite protocol**

```swift
import Foundation

protocol DaemonDriving: AnyObject, Sendable {
    func currentRelayLink() -> RelayLinkState
    func currentPeer() -> PeerState
    func currentTrustedDevice() throws -> PeerRecord?
    func pairingQr() async throws -> RelayQrPayload
    func forgetPeer() async throws
    func stop() async throws

    // Agent-runtime methods unchanged
    func currentAgentState() -> AgentState
    func startAgent(_ req: StartAgentRequest) async throws -> StartAgentResponse
    func sendUserMessage(_ req: SendUserMessageRequest) async throws
    func stopAgent() async throws

    func subscribeRelayLink(_ observer: RelayLinkStateObserver) -> any SubscriptionHandle
    func subscribePeer(_ observer: PeerStateObserver) -> any SubscriptionHandle
    func subscribeAgentState(_ observer: AgentStateObserver) -> any SubscriptionHandle
}

protocol SubscriptionHandle: AnyObject, Sendable {
    func cancel()
}
```

- [ ] **Step 2: Update `DaemonHandle+DaemonDriving.swift`**

Rewrite the extension so `DaemonHandle` conforms to the new protocol. Uses UniFFI-generated method names:

```swift
extension DaemonHandle: DaemonDriving {
    func currentRelayLink() -> RelayLinkState { currentRelayLink() } // UniFFI-generated
    // ... etc
    func subscribeRelayLink(_ observer: RelayLinkStateObserver) -> any SubscriptionHandle {
        subscribeRelayLink(observer: observer)
    }
    // ...
}
```

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "feat(macos): DaemonDriving protocol rewritten for dual-axis state"
```

---

### Task I.5: Update `AppState` for phase + relay link + peer fields

**Files:**
- Modify: `apps/macos/Minos/Application/AppState.swift`

- [ ] **Step 1: Update stored properties**

Add fields:

```swift
enum Phase { case awaitingConfig, running, bootFailed }

@Observable
final class AppState: @unchecked Sendable {
    var daemon: (any DaemonDriving)?
    var relayLinkSubscription: (any SubscriptionHandle)?
    var peerSubscription: (any SubscriptionHandle)?
    var agentSubscription: (any SubscriptionHandle)?

    var phase: Phase = .awaitingConfig
    var relayLink: RelayLinkState = .disconnected
    var peer: PeerState = .unpaired
    var trustedDevice: PeerRecord?
    var currentQr: RelayQrPayload?
    var currentQrGeneratedAt: Date?
    var onboardingVisible: Bool = false
    var settingsVisible: Bool = false

    var isShowingQr: Bool = false
    var currentSession: StartAgentResponse?
    var agentState: AgentState = .idle
    var agentError: MinosError?
    var bootError: MinosError?
    var displayError: MinosError?

    @ObservationIgnored
    var logger: Logger
    @ObservationIgnored
    var displayErrorTask: Task<Void, Never>?
    @ObservationIgnored
    var agentErrorTask: Task<Void, Never>?
    @ObservationIgnored
    var forgetConfirmation: @MainActor @Sendable (PeerRecord) -> Bool
    @ObservationIgnored
    var terminator: @MainActor @Sendable () -> Void

    // Computed helpers for MenuBarView
    var canShowQr: Bool {
        phase == .running && peer == .unpaired && relayLink == .connected
    }
    var canForgetPeer: Bool {
        guard phase == .running, case .paired = peer, case .connected = relayLink else { return false }
        return true
    }

    init(/* ... */) { /* ... */ }
}
```

Remove `connectionState: ConnectionState?` field and all references.

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "feat(macos): AppState uses phase + relayLink + peer + onboarding flags"
```

---

### Task I.6: Rewrite `DaemonBootstrap.swift`

**Files:**
- Modify: `apps/macos/Minos/Infrastructure/DaemonBootstrap.swift`

- [ ] **Step 1: Replace body**

```swift
import Foundation

enum DaemonBootstrap {
    @MainActor
    static func bootstrap(_ appState: AppState) async {
        try? initLogging()
        appState.logger.info("boot start")

        // 1. CF creds: env → Keychain → onboarding
        let creds = readEnvCreds() ?? KeychainRelayConfig.read()
        guard let creds = creds else {
            appState.phase = .awaitingConfig
            appState.onboardingVisible = true
            return
        }

        do {
            let daemon = try await DaemonHandle.start(
                cfClientId: creds.clientId,
                cfClientSecret: creds.clientSecret,
                macName: Host.current().localizedName ?? "Mac"
            )
            let relayObs = RelayLinkObserver { state in
                Task { @MainActor in appState.relayLink = state }
            }
            let peerObs = PeerObserver { state in
                Task { @MainActor in
                    appState.peer = state
                    if case .paired(let id, let name, _) = state {
                        appState.trustedDevice = PeerRecord(
                            deviceId: id,
                            name: name,
                            pairedAt: Date()
                        )
                    } else if case .unpaired = state {
                        appState.trustedDevice = nil
                    }
                }
            }
            appState.daemon = daemon
            appState.relayLinkSubscription = daemon.subscribeRelayLink(relayObs)
            appState.peerSubscription = daemon.subscribePeer(peerObs)
            appState.relayLink = daemon.currentRelayLink()
            appState.peer = daemon.currentPeer()
            appState.trustedDevice = try? daemon.currentTrustedDevice()
            appState.phase = .running
        } catch let e as MinosError {
            appState.bootError = e
            appState.phase = .bootFailed
        } catch {
            appState.bootError = .relayInternal(message: "\(error)")
            appState.phase = .bootFailed
        }
    }

    private static func readEnvCreds() -> KeychainRelayConfig.Creds? {
        guard
            let id = ProcessInfo.processInfo.environment["CF_ACCESS_CLIENT_ID"],
            let secret = ProcessInfo.processInfo.environment["CF_ACCESS_CLIENT_SECRET"]
        else { return nil }
        return KeychainRelayConfig.Creds(clientId: id, clientSecret: secret)
    }
}
```

Note: `DaemonHandle.start(cfClientId:...)` is the UniFFI-generated ctor name per Rust's `impl DaemonHandle { pub async fn start(...) }`. Verify in `apps/macos/Minos/Generated/*.swift`.

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "feat(macos): rewrite DaemonBootstrap for env/Keychain precedence + two observers"
```

---

### Task I.7: Phase I closer

- [ ] **Step 1: Build**

```bash
cargo xtask gen-uniffi
cargo xtask gen-xcode
xcodebuild -scheme Minos -destination 'platform=macOS' -configuration Debug build
```

Expected: xcodebuild fails at `MenuBarView` and `StatusIcon` etc. — those are not yet updated. Phase J covers them.

- [ ] **Step 2: Run Rust check-all**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: green.

---

## Phase J — Swift Presentation layer

### Task J.1: Add `OnboardingSheet.swift` + `SettingsSheet.swift`

**Files:**
- Create: `apps/macos/Minos/Presentation/OnboardingSheet.swift`
- Create: `apps/macos/Minos/Presentation/SettingsSheet.swift`

- [ ] **Step 1: Write OnboardingSheet**

```swift
import SwiftUI

struct OnboardingSheet: View {
    @Bindable var appState: AppState
    @State private var clientId: String = ""
    @State private var clientSecret: String = ""
    @State private var saving: Bool = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("配置 Cloudflare Access Service Token")
                .font(.headline)
            Text("首次使用 Minos 需要 Service Token 才能连接后端。请在 Cloudflare Zero Trust 控制台生成后粘贴下方。")
                .font(.caption)
                .foregroundStyle(.secondary)

            VStack(alignment: .leading, spacing: 8) {
                Text("Client ID").font(.subheadline)
                TextField("", text: $clientId, prompt: Text("xxxxxxxxxx.access"))
                    .textFieldStyle(.roundedBorder)
            }

            VStack(alignment: .leading, spacing: 8) {
                Text("Client Secret").font(.subheadline)
                SecureField("", text: $clientSecret, prompt: Text("paste from dashboard"))
                    .textFieldStyle(.roundedBorder)
            }

            if let error = error {
                Text(error).font(.caption).foregroundStyle(.red)
            }

            HStack {
                Spacer()
                Button("保存") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(clientId.isEmpty || clientSecret.isEmpty || saving)
            }
        }
        .padding(24)
        .frame(width: 420)
    }

    private func save() {
        saving = true
        do {
            try KeychainRelayConfig.write(
                .init(clientId: clientId, clientSecret: clientSecret)
            )
            appState.onboardingVisible = false
            Task { await DaemonBootstrap.bootstrap(appState) }
        } catch {
            self.error = "保存失败：\(error.localizedDescription)"
        }
        saving = false
    }
}
```

- [ ] **Step 2: Write SettingsSheet (variant with Cancel)**

```swift
import SwiftUI

struct SettingsSheet: View {
    @Bindable var appState: AppState
    @State private var clientId: String = ""
    @State private var clientSecret: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Relay 设置").font(.headline)

            if ProcessInfo.processInfo.environment["CF_ACCESS_CLIENT_ID"] != nil {
                Text("当前有环境变量覆盖生效，本次保存的值在 unset 环境变量之前不会生效。")
                    .font(.caption)
                    .foregroundStyle(.orange)
            }

            TextField("Client ID", text: $clientId)
                .textFieldStyle(.roundedBorder)
            SecureField("Client Secret", text: $clientSecret)
                .textFieldStyle(.roundedBorder)

            HStack {
                Button("取消") { appState.settingsVisible = false }
                Spacer()
                Button("保存") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(clientId.isEmpty || clientSecret.isEmpty)
            }
        }
        .padding(24)
        .frame(width: 420)
        .onAppear {
            if let creds = KeychainRelayConfig.read() {
                clientId = creds.clientId
                clientSecret = creds.clientSecret
            }
        }
    }

    private func save() {
        try? KeychainRelayConfig.write(.init(clientId: clientId, clientSecret: clientSecret))
        appState.settingsVisible = false
        Task {
            try? await appState.daemon?.stop()
            appState.relayLinkSubscription?.cancel()
            appState.peerSubscription?.cancel()
            await DaemonBootstrap.bootstrap(appState)
        }
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add apps/macos/Minos/Presentation/OnboardingSheet.swift apps/macos/Minos/Presentation/SettingsSheet.swift
git commit -m "feat(macos): add OnboardingSheet + SettingsSheet for CF token entry"
```

---

### Task J.2: Rewrite `MenuBarView.swift` for phase-aware layouts

**Files:**
- Modify: `apps/macos/Minos/Presentation/MenuBarView.swift`

- [ ] **Step 1: Replace body with phase ladder**

```swift
struct MenuBarView: View {
    @Bindable var appState: AppState

    var body: some View {
        if let bootError = appState.bootError {
            bootErrorContent(bootError)
        } else {
            switch appState.phase {
            case .awaitingConfig:
                awaitingConfigContent
            case .bootFailed:
                bootErrorContent(appState.bootError ?? .relayInternal(message: "unknown"))
            case .running:
                runningContent
            }
        }
    }

    private var awaitingConfigContent: some View {
        VStack(alignment: .leading) {
            Label("Minos · 等待配置", systemImage: "bolt.slash")
                .foregroundStyle(.red)
            Divider()
            Button("Relay 设置…") { appState.onboardingVisible = true }
            Button("退出 Minos") { appState.terminator() }
        }
        .sheet(isPresented: $appState.onboardingVisible) {
            OnboardingSheet(appState: appState)
        }
    }

    @ViewBuilder
    private var runningContent: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                StatusIcon(link: appState.relayLink, peer: appState.peer)
                Text(appState.relayLink.displayLabel).font(.headline)
            }
            Text(appState.peer.displayLabel)
                .font(.caption)
                .foregroundStyle(.secondary)

            if let err = appState.displayError {
                ErrorBanner(error: err)
            }

            Divider()

            if appState.canShowQr {
                Button("显示配对二维码…") { showQr() }
            }
            if appState.canForgetPeer, case .paired = appState.peer {
                Button("忘记已配对设备") { forgetPeer() }
            } else if case .paired = appState.peer {
                // paired but relay disconnected → menu disabled
                Button("忘记已配对设备 (需要后端在线)") { }.disabled(true)
            }
            Button("Relay 设置…") { appState.settingsVisible = true }
            Button("在 Finder 中显示今日日志…") { revealLog() }
            Button("退出 Minos") { appState.terminator() }
        }
        .sheet(isPresented: $appState.settingsVisible) {
            SettingsSheet(appState: appState)
        }
        .sheet(isPresented: $appState.isShowingQr) {
            if let qr = appState.currentQr {
                PairingQRView(qr: qr, appState: appState)
            }
        }
    }

    private func bootErrorContent(_ e: MinosError) -> some View {
        /* existing impl, updated to use e.userMessage(lang: .zh) */
    }

    private func showQr() {
        Task {
            do {
                let qr = try await appState.daemon?.pairingQr()
                appState.currentQr = qr
                appState.currentQrGeneratedAt = Date()
                appState.isShowingQr = true
            } catch let err as MinosError {
                appState.displayError = err
            } catch { }
        }
    }

    private func forgetPeer() {
        guard case .paired(_, let name, _) = appState.peer else { return }
        // Use forgetConfirmation closure (NSAlert)
        Task {
            guard let record = try? appState.daemon?.currentTrustedDevice(),
                  let record = record,
                  await appState.forgetConfirmation(record) else { return }
            try? await appState.daemon?.forgetPeer()
        }
    }

    private func revealLog() { /* unchanged */ }
}
```

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "feat(macos): MenuBarView with phase-aware layouts + dual-axis status"
```

---

### Task J.3: Update `StatusIcon.swift` for `(RelayLinkState, PeerState)` matrix

**Files:**
- Modify: `apps/macos/Minos/Presentation/StatusIcon.swift`

- [ ] **Step 1: Replace body**

```swift
struct StatusIcon: View {
    let link: RelayLinkState
    let peer: PeerState

    var body: some View {
        Image(systemName: iconName)
            .foregroundStyle(tint)
    }

    private var iconName: String {
        if case .connected = link {
            switch peer {
            case .unpaired: return "bolt.circle"
            case .pairing: return "qrcode"
            case .paired(_, _, let online):
                return online ? "bolt.circle.fill" : "bolt.circle"
            }
        } else if case .connecting = link {
            return "bolt.circle"
        } else {
            return "bolt.slash"
        }
    }

    private var tint: Color {
        if case .connected = link {
            if case .paired(_, _, let online) = peer, online { return .green }
            return .accentColor
        } else if case .connecting = link {
            return .orange
        } else {
            return .red
        }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "feat(macos): StatusIcon derives symbol+tint from (relay, peer) matrix"
```

---

### Task J.4: Update `PairingQRView.swift` for `RelayQrPayload`

**Files:**
- Modify: `apps/macos/Minos/Presentation/PairingQRView.swift`

- [ ] **Step 1: Change prop from `QrPayload` to `RelayQrPayload`**

Adjust the view's initializer parameter type and the `QRCodeRenderer.image(for:)` call (QRCodeRenderer must also be updated to accept the new type — or serialize the new struct to JSON and encode the JSON bytes). Simplest: `RelayQrPayload` conforms to `Encodable`; serialize inline:

```swift
let data = try! JSONEncoder().encode(qr)
let image = QRCodeRenderer.image(fromData: data)
```

Adjust `QRCodeRenderer` if needed:

```swift
enum QRCodeRenderer {
    static func image(fromData data: Data) -> CGImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.setValue(data, forKey: "inputMessage")
        filter.setValue("M", forKey: "inputCorrectionLevel")
        return filter.outputImage.flatMap { /* scale + convert */ }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "feat(macos): PairingQRView + QRCodeRenderer consume RelayQrPayload"
```

---

### Task J.5: Phase J closer — xcodebuild green

- [ ] **Step 1: Build**

```bash
cargo xtask check-all
```

Expected: green (Rust + Swift + swiftlint + flutter + frb drift).

If any Xcode errors remain, fix inline. Common culprits:
- UniFFI-generated method names differ from what I wrote (e.g., `subscribeRelayLink(observer:)` vs `subscribeRelayLink`). Check `apps/macos/Minos/Generated/MinosCore.swift`.
- `MinosError` Swift enum case names differ from Rust (UniFFI camelCases): `.cfAuthFailed` vs `.CfAuthFailed`.

---

## Phase K — XCTests rewrite

### Task K.1: Rewrite `MockDaemon` for new protocol

**Files:**
- Modify: `apps/macos/MinosTests/TestSupport/MockDaemon.swift`

- [ ] **Step 1: Rewrite**

```swift
import Foundation
@testable import Minos

final class MockDaemon: DaemonDriving, @unchecked Sendable {
    var relayLink: RelayLinkState = .disconnected
    var peer: PeerState = .unpaired
    var trustedDevice: PeerRecord?

    var pairingQrResult: Result<RelayQrPayload, MinosError> = .failure(.relayInternal(message: "mock"))
    var forgetPeerResult: Result<Void, MinosError> = .success(())
    var stopCallCount = 0
    var forgetPeerCallCount = 0

    private(set) var relayLinkObservers: [RelayLinkStateObserver] = []
    private(set) var peerObservers: [PeerStateObserver] = []

    func currentRelayLink() -> RelayLinkState { relayLink }
    func currentPeer() -> PeerState { peer }
    func currentTrustedDevice() throws -> PeerRecord? { trustedDevice }
    func pairingQr() async throws -> RelayQrPayload { try pairingQrResult.get() }
    func forgetPeer() async throws {
        forgetPeerCallCount += 1
        try forgetPeerResult.get()
    }
    func stop() async throws { stopCallCount += 1 }

    // Agent methods stubbed
    func currentAgentState() -> AgentState { .idle }
    func startAgent(_ req: StartAgentRequest) async throws -> StartAgentResponse { fatalError("unused") }
    func sendUserMessage(_ req: SendUserMessageRequest) async throws { fatalError("unused") }
    func stopAgent() async throws { fatalError("unused") }

    func subscribeRelayLink(_ observer: RelayLinkStateObserver) -> any SubscriptionHandle {
        relayLinkObservers.append(observer)
        return StubSubscription()
    }
    func subscribePeer(_ observer: PeerStateObserver) -> any SubscriptionHandle {
        peerObservers.append(observer)
        return StubSubscription()
    }
    func subscribeAgentState(_ observer: AgentStateObserver) -> any SubscriptionHandle {
        StubSubscription()
    }

    // Test helpers
    func fireRelayLink(_ state: RelayLinkState) {
        relayLink = state
        for o in relayLinkObservers { o.onState(state: state) }
    }
    func firePeer(_ state: PeerState) {
        peer = state
        for o in peerObservers { o.onState(state: state) }
    }
}

final class StubSubscription: SubscriptionHandle, @unchecked Sendable {
    var canceled = false
    func cancel() { canceled = true }
}
```

- [ ] **Step 2: Commit**

```bash
git add -u apps/macos/MinosTests/TestSupport/MockDaemon.swift
git commit -m "test(macos): MockDaemon rewritten for DaemonDriving dual-axis protocol"
```

---

### Task K.2: Rewrite `AppStateTests.swift`

**Files:**
- Modify: `apps/macos/MinosTests/Application/AppStateTests.swift`

- [ ] **Step 1: Replace with per-scenario cases from spec §9.2**

Representative cases:

```swift
import XCTest
@testable import Minos

@MainActor
final class AppStateTests: XCTestCase {
    func testFirstLaunchNoCredsShowsOnboarding() {
        let appState = AppState(/* ... */)
        // Keychain mocked to return nil, env empty
        XCTAssertEqual(appState.phase, .awaitingConfig)
        XCTAssertTrue(appState.onboardingVisible)
        XCTAssertFalse(appState.canShowQr)
    }

    func testRelayLinkObserverUpdatesState() {
        let (appState, mock) = makeConnectedState()
        mock.fireRelayLink(.connecting(attempt: 1))
        XCTAssertEqual(appState.relayLink, .connecting(attempt: 1))
    }

    func testReconnectPreservesPeer() {
        let (appState, mock) = makeConnectedState()
        let did = DeviceId()
        mock.firePeer(.paired(peerId: did, peerName: "iPhone", online: true))
        mock.fireRelayLink(.connecting(attempt: 1))
        XCTAssertEqual(appState.peer, .paired(peerId: did, peerName: "iPhone", online: true))
    }

    func testCanForgetPeerFalseWhenDisconnected() {
        let (appState, mock) = makeConnectedState()
        let did = DeviceId()
        mock.firePeer(.paired(peerId: did, peerName: "iPhone", online: true))
        mock.fireRelayLink(.disconnected)
        XCTAssertFalse(appState.canForgetPeer)
    }

    func testForgetPeerSuccessCallsMockAndClears() async {
        let (appState, mock) = makeConnectedState()
        let did = DeviceId()
        mock.firePeer(.paired(peerId: did, peerName: "iPhone", online: true))
        try? await mock.forgetPeer()
        mock.firePeer(.unpaired)
        XCTAssertEqual(mock.forgetPeerCallCount, 1)
        XCTAssertEqual(appState.peer, .unpaired)
    }

    // ... etc per spec §9.2 table
}
```

- [ ] **Step 2: Run**

```bash
xcodebuild -scheme Minos -destination 'platform=macOS' test
```

Expected: green.

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "test(macos): rewrite AppStateTests per §9.2 matrix"
```

---

### Task K.3: Phase K closer

- [ ] **Step 1: Run full `check-all`**

```bash
cargo xtask check-all
```

Expected: green everything.

---

## Phase L — Fake-peer dev bin

### Task L.1: Add `fake-peer.rs` binary to `minos-mobile`

**Files:**
- Modify: `crates/minos-mobile/Cargo.toml`
- Create: `crates/minos-mobile/src/bin/fake-peer.rs`

- [ ] **Step 1: Update Cargo.toml**

Add:

```toml
[[bin]]
name = "fake-peer"
path = "src/bin/fake-peer.rs"
required-features = ["cli"]

[features]
cli = ["clap", "tokio/rt-multi-thread", "tokio/macros"]

[dependencies]
# ...existing...
clap = { workspace = true, optional = true }
```

Add `tokio-tungstenite` and `futures-util` to dependencies if not already there; also `serde_json`.

- [ ] **Step 2: Write the bin**

```rust
//! fake-peer: dev tool that impersonates an ios-client role to smoke-test
//! the Mac app's pairing flow without iOS. See spec §10.3.

use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::envelope::{Envelope, EventKind, LocalRpcMethod};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest,
    http::HeaderName,
    Message,
};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "ws://127.0.0.1:8787/devices")]
    backend: String,
    #[arg(long)]
    token: String,
    #[arg(long, default_value = "fake-peer")]
    device_name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let device_id = DeviceId::new();
    let mut req = args.backend.clone().into_client_request()?;
    req.headers_mut().insert(HeaderName::from_static("x-device-id"), device_id.to_string().parse()?);
    req.headers_mut().insert(HeaderName::from_static("x-device-role"), DeviceRole::IosClient.to_string().parse()?);

    let (ws, _resp) = tokio_tungstenite::connect_async(req).await?;
    let (mut sink, mut stream) = ws.split();

    // Send Pair LocalRpc
    let pair_req = Envelope::LocalRpc {
        v: 1,
        id: 1,
        method: LocalRpcMethod::Pair,
        params: serde_json::json!({
            "token": args.token,
            "device_name": args.device_name,
        }),
    };
    sink.send(Message::Text(serde_json::to_string(&pair_req)?)).await?;

    // Read responses until interrupted
    while let Some(msg) = stream.next().await {
        match msg? {
            Message::Text(t) => {
                let env: Envelope = serde_json::from_str(&t)?;
                match env {
                    Envelope::LocalRpcResponse { .. } => eprintln!("local_rpc_response: {}", t),
                    Envelope::Event { event, .. } => eprintln!("event: {:?}", event),
                    other => eprintln!("other: {:?}", other),
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Build and smoke**

```bash
cargo build -p minos-mobile --bin fake-peer --features cli
# start a local relay in another terminal first
cargo run -p minos-mobile --bin fake-peer --features cli -- --help
```

- [ ] **Step 4: Commit**

```bash
git add crates/minos-mobile/Cargo.toml crates/minos-mobile/src/bin/fake-peer.rs
git commit -m "feat(mobile): add fake-peer dev bin for Mac-side smoke"
```

---

## Phase M — ADR, README, final smoke

### Task M.1: ADR body already written

The ADR was authored alongside the spec (commit `040708d`). Nothing to do here beyond verifying the file exists.

- [ ] **Step 1: Verify**

```bash
ls docs/adr/0013-macos-relay-client-cutover.md
```

Expected: file exists.

### Task M.2: Update README status line

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Edit Status line**

Find the "Status" section and update the bullet about plan 04 / next step. Append a line:

```
- Plan 05 (this branch): Mac app migrated from Tailscale P2P to minos-relay WSS client.
  Tailscale removed, onboarding sheet for CF token, two-axis state.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(readme): note plan 05 Mac relay-client migration in status"
```

### Task M.3: CI yaml release-job TODO placeholder

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Append a comment block**

At the end of `.github/workflows/ci.yml`, under the last job or in a trailing comment:

```yaml
# TODO: follow-up release job (P1.5) will inject secrets.MINOS_BACKEND_URL as env
# var at cargo build time for minos-ffi-uniffi, produce a signed & notarized .app,
# and upload to the releases page. Tracked in spec macos-relay-client-migration-design.md §2.2.
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "docs(ci): reserve release-job slot for MINOS_BACKEND_URL secret injection"
```

### Task M.4: Full smoke checklist

Run the spec §2.3 checklist by hand. Mark each box. Any failure → fix + add a commit.

- [ ] **Step 1: Start a local relay**

```bash
cargo run -p minos-relay -- --listen 127.0.0.1:8787 --db /tmp/relay.db
```

- [ ] **Step 2: Launch the freshly built Mac app**

```bash
cargo xtask build-macos
cargo xtask gen-uniffi
cargo xtask gen-xcode
xcodebuild -scheme Minos -destination 'platform=macOS' -configuration Debug build
open apps/macos/build/Debug/Minos.app
```

- [ ] **Step 3: Execute the 12-box checklist from spec §2.3**

Tick each box. Any stuck item → open a sub-issue / add a fix commit.

### Task M.5: Phase M closer — final `check-all` + ready for PR

- [ ] **Step 1: Final green**

```bash
cargo xtask check-all
```

- [ ] **Step 2: Push branch and open PR from worktree**

```bash
cd /Users/zhangfan/develop/github.com/minos-worktrees/macos-relay-migration
git push -u origin feat/macos-relay-migration
gh pr create --title "feat(macos): migrate app from Tailscale P2P to minos-relay WSS client" \
  --body "$(cat <<'EOF'
## Summary

- Mac app is now an outbound WSS client of `minos-relay` (landed in PR #1)
- Tailscale code (`tailscale.rs`, `discover_tailscale_ip`, WsServer, port-retry) fully removed
- CF Service Token onboarding via two-field Keychain sheet; env var `CF_ACCESS_CLIENT_ID` / `CF_ACCESS_CLIENT_SECRET` overrides for dev
- Backend URL baked at compile time via `option_env!("MINOS_BACKEND_URL")`; CI uses local fallback, future release job injects prod URL from secrets
- Connection state split into two orthogonal axes: `RelayLinkState` (to relay) + `PeerState` (peer online/offline)
- New dev bin `cargo run -p minos-mobile --bin fake-peer` for end-to-end smoke without iOS
- Keychain `device-secret` stored via `security-framework` (Rust); CF tokens stored via `Security.framework` (Swift)
- iOS / Flutter untouched — pairing across platforms breaks during the gap until iOS migration spec lands

## Test plan

- [ ] `cargo xtask check-all` green (Rust + Swift + Flutter + frb drift)
- [ ] Spec §2.3 smoke 12-box checklist passes on maintainer's Mac
- [ ] Onboarding + SettingsSheet + reconnect flows visually confirmed
- [ ] Keychain Access.app shows `ai.minos.macos` entries after pairing
- [ ] `grep -r "tailscale\|discover_tailscale_ip\|WsServer" crates apps` yields zero production hits

## Design refs

- Spec: `docs/superpowers/specs/macos-relay-client-migration-design.md`
- ADR: `docs/adr/0013-macos-relay-client-cutover.md`
- Plan: `docs/superpowers/plans/05-macos-relay-client-migration.md`
EOF
)"
```

---

## Appendix: Spec coverage check (self-review)

| Spec section | Covered by |
|---|---|
| §2.1 In scope item 1 (auto-connect boot) | H, I (DaemonBootstrap), J (MenuBarView) |
| §2.1 item 2 (onboarding sheet) | J.1, I.6 |
| §2.1 item 3 (settings sheet) | J.1, J.2 |
| §2.1 item 4 (QR schema change) | B.2 (RelayQrPayload), E.4 |
| §2.1 item 5 (two-axis state) | A.1, A.2, F.1, I.3, I.5 |
| §2.1 item 6 (Tailscale removal) | D.1, F.2 |
| §2.1 item 7 (persistence split) | B.3 (LocalState), C.2 (KeychainStore), H.1 (KeychainRelayConfig) |
| §2.1 item 8 (forget behavior) | E.5, F.1 (forget_peer), I.5 (canForgetPeer), J.2 (disabled menu) |
| §2.1 item 9 (fake-peer bin) | L.1 |
| §2.1 item 10 (check-all green) | Every phase closer |
| §5.1 per-crate deltas | Phase A–G maps 1:1 |
| §5.2 six Phase 0 steps | F.1 (step 1, 2, 3, 4, 5), F.2 (step 6) |
| §6.1–6.8 data flows | Phase I bootstrap + Phase J views + Phase E–F dispatch |
| §7 persistence | B.3, C.2, H.1 |
| §8 error handling | A.4 (CfAuthFailed), D.4 (close code mapping), I.2 (Display) |
| §9 testing strategy | Tests embedded in each task + K.1, K.2 (XCTests rewrite) |
| §10 tooling & ops | B.1 (config), L.1 (fake-peer), M.3 (CI TODO) |
| §12 open questions | Resolved during plan writing; see Divergence note |
| §13 ADR | Already committed (`040708d`) |
| §14 file inventory | Matches Phase A–M output |

**Gap:** Spec §2.2 mentions "relay admin console on `/admin`" as deferred. Plan does not address. ✓ correctly out of scope.

**Gap:** Spec §6.5 CF auth failure path says "daemon stores last error on handle". Plan F.1 does not add a `last_error` field to `DaemonHandle`. Action: add a small field + `last_error()` method to the Rust side so Swift can fetch it; amend Phase F.1 Step 2 to include this. (For the implementer: when coding F.1, add `last_error: std::sync::Mutex<Option<MinosError>>` to `DaemonInner` and populate from `relay_client` task on fatal errors.)

No other gaps identified.

---

## Execution handoff

**Plan complete and saved to `docs/superpowers/plans/05-macos-relay-client-migration.md`.** Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Good for this plan because the per-task TDD loops are well-scoped and independently verifiable.

2. **Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

Which approach?
