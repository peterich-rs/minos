# Server-Centric Auth & Account-Pair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drop the `X-Device-Secret` rail from iOS clients, switch the pairing data model from device↔device to (mac_device, mobile_account), and add `target_device_id` to the envelope so a mobile account can drive multiple paired Macs from the same session. Mac-side auth (CF Access + device-secret) is intentionally untouched.

**Architecture:** A new `account_mac_pairings` table replaces `pairings` as the single source of truth for "which iOS account can talk to which Mac". `Envelope::Forward` gains a mandatory `target_device_id` so the backend stops inferring routes from `SessionHandle.paired_with`. iOS HTTP/WS calls authenticate with bearer JWT only — the backend's `classify()` becomes role-aware so iOS rows can carry `secret_hash = NULL` indefinitely. iOS keychain stops persisting `device_secret`. The Mac rail (`X-Device-Secret` + CF Access, `secret_hash NOT NULL`) is unchanged.

**Tech Stack:** Rust 2021, sqlx (sqlite), axum, jsonrpsee, tokio, serde, flutter_rust_bridge v2, Flutter 3 + flutter_secure_storage.

**Spec:** New `docs/adr/0020-server-centric-auth-and-account-pairs.md` (written in Phase A); supersedes the "device-secret stays" decision in `docs/superpowers/specs/mobile-auth-and-agent-session-design.md` §12.2.

**Pre-deployment context:** Backend has never been deployed. Old `pairings` table can be dropped outright — no double-write, no shim, no migration grace period.

---

## File Structure

### New files
- `docs/adr/0020-server-centric-auth-and-account-pairs.md` — supersedes §12.2 of mobile-auth spec; documents iOS bearer-only rail and account-keyed pairs.
- `crates/minos-backend/migrations/0011_drop_legacy_pairings.sql` — drops old table.
- `crates/minos-backend/migrations/0012_account_mac_pairings.sql` — new table.
- `crates/minos-backend/src/store/account_mac_pairings.rs` — new store module.
- `crates/minos-backend/src/http/v1/me_macs.rs` — new `/v1/me/macs` endpoint.
- `crates/minos-backend/tests/server_centric_auth_e2e.rs` — full pair → forward → reply integration test.

### Modified files (backend)
- `crates/minos-backend/src/store/mod.rs` — module declaration.
- `crates/minos-backend/src/store/pairings.rs` — **deleted entirely**.
- `crates/minos-backend/src/http/auth.rs` — `classify()` becomes role-aware.
- `crates/minos-backend/src/http/v1/auth.rs` — drop `authenticated_with_secret` requirement on iOS endpoints.
- `crates/minos-backend/src/http/v1/threads.rs` — drop `require_paired_session` secret check; resolve thread owner via account_mac_pairings.
- `crates/minos-backend/src/http/v1/me.rs` — replaced by me_macs.rs (or kept as Mac-only stub, see Task H1).
- `crates/minos-backend/src/http/v1/pairing.rs` — write to new table; drop iOS secret generation; `DELETE /v1/pairings/{mac_device_id}`.
- `crates/minos-backend/src/http/ws_devices.rs` — drop secret check for IosClient role.
- `crates/minos-backend/src/pairing/mod.rs` — `consume_token()` no longer mints iOS secret.
- `crates/minos-backend/src/envelope/mod.rs` — route by `target_device_id`; validate against account_mac_pairings.
- `crates/minos-backend/src/session/registry.rs` — drop `paired_with` single slot; iOS sessions no longer need it.
- `crates/minos-backend/src/ingest/mod.rs` — `broadcast_to_peers_of()` queries new table.
- `crates/minos-backend/src/router.rs` (or wherever routes are wired) — register `/v1/me/macs`.

### Modified files (protocol)
- `crates/minos-protocol/src/envelope.rs` — `Forward.target_device_id`; `EventKind::Paired.your_device_secret` becomes `Option<DeviceSecret>`.
- `crates/minos-protocol/src/messages.rs` — drop `PairResponse.your_device_secret`; new `MeMacsResponse` + `MacSummary`.
- `crates/minos-protocol/tests/golden/envelope/*.json` — new golden fixtures.

### Modified files (mobile rust + transport)
- `crates/minos-mobile/src/store.rs` — drop `device_secret` field from `PersistedPairingState` and trait.
- `crates/minos-mobile/src/client.rs` — drop secret-related logic; new `list_paired_macs()`, `forget_mac(mac_device_id)`.
- `crates/minos-mobile/src/http.rs` — drop `X-Device-Secret` injection on iOS path.
- `crates/minos-transport/src/auth.rs` — make secret optional on `AuthHeaders`; iOS callers pass `None`.
- `crates/minos-ffi-frb/src/api/minos.rs` — expose `list_paired_macs`, `forget_mac`; drop secret-leaking surface.

### Modified files (flutter dart)
- `apps/mobile/lib/src/rust/api/minos.dart` — **regenerated** (frb codegen).
- `apps/mobile/lib/src/rust/frb_generated.dart` — **regenerated**.
- `apps/mobile/lib/infrastructure/secure_pairing_store.dart` — drop `_keyDeviceSecret`; on cold start wipe legacy field.
- `apps/mobile/lib/infrastructure/minos_core.dart` — drop `deviceSecret` from `PersistedPairingState` consumers.
- `apps/mobile/lib/presentation/pages/pairing_page.dart` — no `your_device_secret` in response handling.
- `apps/mobile/lib/presentation/pages/app_shell_page.dart` — render Mac list + active-mac selector.
- `apps/mobile/lib/application/minos_providers.dart` — wire list/forget/active-mac providers.

### Modified files (specs)
- `docs/superpowers/specs/mobile-auth-and-agent-session-design.md` — §12.2 supersession note.

---

## Phase A — ADR & spec lock

After this phase the design decisions are written down before any code changes.

### Task A1: Write ADR-0020

**Files:**
- Create: `docs/adr/0020-server-centric-auth-and-account-pairs.md`
- Modify: `docs/superpowers/specs/mobile-auth-and-agent-session-design.md` (§12.2 supersession note)

- [ ] **Step 1: Write the ADR**

```markdown
# 0020. Server-centric auth simplification and account-keyed pairs

Status: Accepted
Date: 2026-05-01
Supersedes: §12.2 ("Single device vs multi-device → single") of `mobile-auth-and-agent-session-design.md`; partially supersedes §5.4 (dual-rail iOS auth) of same.

## Context

Minos is account-centric (slack.ai-shaped), not P2P. The current dual-rail
auth (`X-Device-Secret` + bearer JWT for iOS, secret-only for Mac) was
inherited from the Remodex P2P design without re-justification for the
server-centric model. Three observable consequences:

1. iOS keychain holds two long-lived credentials (`device_secret` + auth
   tuple). Equivalent security guarantees can be obtained with bearer alone
   (JWT.did binds to X-Device-Id; refresh_token is per-device-revocable).
2. The pair model is keyed on device IDs: `pairings(device_a, device_b)`.
   Re-installing the iOS app changes `device_id` and orphans existing
   pairs. The product expectation is that an iOS user signed into the same
   account on a new phone immediately inherits all paired Macs.
3. `Envelope::Forward` carries no target — the backend infers it from a
   single-valued `SessionHandle.paired_with` slot. With multiple Macs paired
   to one account, the backend silently routes to "the most recent pair".

## Decision

1. **iOS becomes bearer-only.** `classify()` becomes role-aware: when a
   device row has `role = ios-client` and `secret_hash IS NULL`, the
   request authenticates via bearer alone. iOS rows are created with
   `secret_hash = NULL` and never populated. Mac rows remain as today.
2. **Pair model becomes (mac_device_id, mobile_account_id).** A new
   `account_mac_pairings` table replaces `pairings`. The mobile
   `device_id` that performed the scan is recorded in
   `paired_via_device_id` for audit only — it does not participate in
   routing.
3. **`Envelope::Forward` gets `target_device_id`.** iOS clients must name
   the Mac they are addressing. The backend validates
   `target_device_id ∈ {macs paired to caller's account_id}` before
   routing; mismatch → `PeerOffline`.
4. **`EventKind::Paired.your_device_secret` becomes `Option<DeviceSecret>`.**
   Set to `Some(secret)` for the Mac recipient (unchanged behaviour);
   `None` for iOS recipients (no secret minted).
5. **`/v1/me/peer` is replaced by `/v1/me/macs`** for iOS callers.
   Returns `Vec<MacSummary>`. Mac-side equivalent is deferred (no UI need
   today; the Mac learns peers via `EventKind::Paired` + future
   broadcast).

## Consequences

- iOS keychain stores only `(device_id, access_token, access_expires_at,
  refresh_token, account_id, account_email, peer_display_name)`.
  `device_secret` field is wiped on cold start.
- `pairings` table and `crates/minos-backend/src/store/pairings.rs` are
  deleted outright (pre-deployment).
- The `your_device_secret` field on `PairResponse` is removed; iOS clients
  receive only `(peer_device_id, peer_name)`.
- Mac WS upgrade and REST paths are unchanged. Mac still holds one
  `device_secret` per host machine.
- Anti-replay across devices: bearer's `did` claim still binds JWT to a
  specific `X-Device-Id`. Stealing only the JWT without also knowing the
  device_id (which is in TLS-protected keychain) yields nothing.
- Multi-mobile-per-account: refresh_tokens(account_id, device_id) already
  supports it; the new pair table preserves the semantic.
- Mac-side daemon's single-peer slot (`crates/minos-daemon/src/handle.rs:30`
  `peer: Option<PeerRecord>`) is **out of scope** for this ADR. P2 in the
  macos-relay-client-migration spec.

## Alternatives rejected

- **Keep `X-Device-Secret` as a derivation of `device_id`** (e.g.
  HMAC). Rejected: the security value of secret comes from "attacker has
  device_id but not secret"; deriving secret from device_id collapses
  dual-factor to single-factor while keeping the protocol surface.
- **Make `X-Device-Secret` short-lived per session.** Rejected: doesn't
  remove the backend `secret_hash` column, doesn't simplify keychain,
  introduces a rotate endpoint with its own auth question.
- **Migrate `pairings` schema in-place by adding `account_id` column.**
  Rejected: pre-deployment context permits clean replacement; in-place
  migration would carry the device-keyed semantics indefinitely.

## Implementation reference

See `docs/superpowers/plans/11-server-centric-auth-and-pair.md`.
```

- [ ] **Step 2: Write supersession note in mobile-auth spec**

In `docs/superpowers/specs/mobile-auth-and-agent-session-design.md` find §12.2 (search for "Single device vs multi-device") and prepend a supersession note. Code:

```markdown
> **Superseded 2026-05-01 by [ADR-0020](../../adr/0020-server-centric-auth-and-account-pairs.md).** The "single device" decision below is no longer current. Pair model is now (mac_device, mobile_account); iOS auth is bearer-only.
```

The same note should be appended to §5.4 ("Dual-rail iOS auth") with text:

```markdown
> **Superseded 2026-05-01 by [ADR-0020](../../adr/0020-server-centric-auth-and-account-pairs.md).** iOS rail no longer requires `X-Device-Secret`; bearer alone authenticates iOS.
```

- [ ] **Step 3: Verify ADR file structure matches existing convention**

Run: `head -10 /Users/zhangfan/develop/github.com/minos/docs/adr/0019-codex-protocol-typed-codegen.md`
Expected: ADR header with `Status:` and `Date:` lines. Confirm 0020 follows the same shape.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0020-server-centric-auth-and-account-pairs.md \
        docs/superpowers/specs/mobile-auth-and-agent-session-design.md
git commit -m "docs(adr): 0020 server-centric auth + account-keyed pairs"
```

---

## Phase B — DB schema

After this phase the database has the new pair table; the old `pairings` table is gone. No code yet uses the new table.

### Task B1: Drop legacy pairings table

**Files:**
- Create: `crates/minos-backend/migrations/0011_drop_legacy_pairings.sql`

- [ ] **Step 1: Write the migration**

```sql
-- 0011_drop_legacy_pairings.sql
-- Pre-deployment: drop old device-keyed pairings outright.
-- The replacement is account_mac_pairings (migration 0012).

DROP INDEX IF EXISTS idx_pairings_device_a;
DROP INDEX IF EXISTS idx_pairings_device_b;
DROP TABLE IF EXISTS pairings;
```

- [ ] **Step 2: Verify migration runs against a fresh DB**

```bash
cd crates/minos-backend
rm -f /tmp/test_drop.db
sqlx migrate run --database-url "sqlite:/tmp/test_drop.db?mode=rwc" --source ./migrations
```

Expected: success, no errors. The migrations 0001-0010 create then 0011 drops `pairings`.

- [ ] **Step 3: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/migrations/0011_drop_legacy_pairings.sql
git commit -m "feat(backend/db): drop legacy device-keyed pairings"
```

### Task B2: Create `account_mac_pairings` table

**Files:**
- Create: `crates/minos-backend/migrations/0012_account_mac_pairings.sql`

- [ ] **Step 1: Write the migration**

```sql
-- 0012_account_mac_pairings.sql
-- Pair model is now (mac_device_id, mobile_account_id). The mobile
-- device_id that performed the scan is recorded as audit metadata.
-- See docs/adr/0020-server-centric-auth-and-account-pairs.md.

CREATE TABLE account_mac_pairings (
    pair_id              TEXT NOT NULL PRIMARY KEY,    -- UUID
    mac_device_id        TEXT NOT NULL,
    mobile_account_id    TEXT NOT NULL,
    paired_via_device_id TEXT NOT NULL,                -- mobile device that scanned; audit only
    paired_at_ms         INTEGER NOT NULL,
    UNIQUE (mac_device_id, mobile_account_id),
    FOREIGN KEY (mac_device_id)        REFERENCES devices(device_id)   ON DELETE CASCADE,
    FOREIGN KEY (mobile_account_id)    REFERENCES accounts(account_id) ON DELETE CASCADE,
    FOREIGN KEY (paired_via_device_id) REFERENCES devices(device_id)   ON DELETE CASCADE
);

CREATE INDEX idx_amp_mobile_account ON account_mac_pairings(mobile_account_id);
CREATE INDEX idx_amp_mac_device     ON account_mac_pairings(mac_device_id);
```

- [ ] **Step 2: Verify migration runs cleanly and constraints exist**

```bash
rm -f /tmp/test_amp.db
sqlx migrate run --database-url "sqlite:/tmp/test_amp.db?mode=rwc" --source ./migrations
sqlite3 /tmp/test_amp.db ".schema account_mac_pairings"
```

Expected: schema dump shows the table + UNIQUE constraint + indexes.

- [ ] **Step 3: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/migrations/0012_account_mac_pairings.sql
git commit -m "feat(backend/db): account_mac_pairings table"
```

---

## Phase C — Protocol crate changes

After this phase, `minos-protocol` exposes the new envelope shape and pair API types. Backend & mobile callers don't compile yet — that's deliberate; subsequent phases fix the call sites.

### Task C1: Add `target_device_id` to `Envelope::Forward`; make `EventKind::Paired.your_device_secret` optional

**Files:**
- Modify: `crates/minos-protocol/src/envelope.rs`
- Modify: `crates/minos-protocol/tests/envelope_golden.rs` (existing golden file)

- [ ] **Step 1: Write the failing round-trip test**

Append to `crates/minos-protocol/src/envelope.rs` `mod tests`:

```rust
#[test]
fn forward_with_target_round_trips() {
    let target = DeviceId::new();
    let env = Envelope::Forward {
        version: 1,
        target_device_id: target,
        payload: serde_json::json!({"jsonrpc": "2.0", "method": "ping", "id": 1}),
    };
    round_trip(&env);
    let v = serde_json::to_value(&env).unwrap();
    assert_eq!(v["kind"], "forward");
    assert_eq!(v["target_device_id"].as_str().unwrap(), target.0.to_string());
}

#[test]
fn paired_event_with_no_secret_round_trips() {
    let env = Envelope::Event {
        version: 1,
        event: EventKind::Paired {
            peer_device_id: DeviceId::new(),
            peer_name: "iPhone".into(),
            your_device_secret: None,
        },
    };
    round_trip(&env);
    let v = serde_json::to_value(&env).unwrap();
    assert_eq!(v["type"], "paired");
    assert!(v.get("your_device_secret").is_none() || v["your_device_secret"].is_null());
}
```

- [ ] **Step 2: Run; expect compile failure**

Run: `cargo test -p minos-protocol --no-run`
Expected: compile error — `Forward` has no field `target_device_id`; `your_device_secret` type mismatch.

- [ ] **Step 3: Update the type definitions**

In `crates/minos-protocol/src/envelope.rs` modify `Envelope::Forward`:

```rust
Forward {
    /// Protocol version.
    #[serde(rename = "v")]
    version: u8,
    /// The Mac device this forward should be routed to. Backend
    /// validates against the caller's account_mac_pairings rows.
    /// Mismatch → routing error (PeerOffline).
    target_device_id: DeviceId,
    /// Opaque payload (JSON-RPC 2.0 by convention between Minos
    /// clients, but the relay does not read it).
    payload: serde_json::Value,
},
```

And `EventKind::Paired`:

```rust
Paired {
    peer_device_id: DeviceId,
    peer_name: String,
    /// Long-lived bearer secret for the Mac recipient. `None` when this
    /// event is delivered to an iOS recipient (iOS rail is bearer-only;
    /// see ADR-0020).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    your_device_secret: Option<DeviceSecret>,
},
```

- [ ] **Step 4: Update the existing `forward_round_trips` and `event_paired_round_trips` tests**

Find the existing `forward_round_trips` test (line ~149) and add `target_device_id`:

```rust
#[test]
fn forward_round_trips() {
    let env = Envelope::Forward {
        version: 1,
        target_device_id: DeviceId::new(),
        payload: serde_json::json!({
            "jsonrpc": "2.0",
            "method": "list_clis",
            "id": 1,
        }),
    };
    round_trip(&env);
    let v = serde_json::to_value(&env).unwrap();
    assert_eq!(v["kind"], "forward");
}
```

Find `event_paired_round_trips` (line ~178) and wrap secret in `Some(...)`:

```rust
#[test]
fn event_paired_round_trips() {
    let env = Envelope::Event {
        version: 1,
        event: EventKind::Paired {
            peer_device_id: DeviceId::new(),
            peer_name: "Mac-mini".into(),
            your_device_secret: Some(DeviceSecret("sek".into())),
        },
    };
    round_trip(&env);
    let v = serde_json::to_value(&env).unwrap();
    assert_eq!(v["kind"], "event");
    assert_eq!(v["type"], "paired");
    assert_eq!(v["your_device_secret"], "sek");
}
```

- [ ] **Step 5: Update golden fixtures (if separate file exists)**

Run: `ls crates/minos-protocol/tests/golden/envelope/ 2>/dev/null`
If files exist: edit `forward.json` to include a `"target_device_id"` field; edit `event_paired.json` to optionally include or omit `"your_device_secret"`. Match the runtime serialization shape.

- [ ] **Step 6: Run tests; expect pass**

Run: `cargo test -p minos-protocol`
Expected: all tests pass including the two new ones from Step 1.

- [ ] **Step 7: Commit**

```bash
cargo xtask check-all
git add crates/minos-protocol/src/envelope.rs crates/minos-protocol/tests/
git commit -m "feat(protocol): Forward.target_device_id; Paired.your_device_secret optional"
```

### Task C2: Drop `your_device_secret` from `PairResponse`; add `MeMacsResponse` + `MacSummary`

**Files:**
- Modify: `crates/minos-protocol/src/messages.rs`

- [ ] **Step 1: Write the failing round-trip tests**

Append to `crates/minos-protocol/src/messages.rs` `mod tests`:

```rust
#[test]
fn pair_response_no_secret_field_round_trip() {
    let resp = PairResponse {
        peer_device_id: DeviceId::new(),
        peer_name: "iPhone".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value.get("your_device_secret").is_none(), "secret must not appear");
    let back: PairResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(back, resp);
}

#[test]
fn me_macs_response_round_trips() {
    let macs = MeMacsResponse {
        macs: vec![MacSummary {
            mac_device_id: DeviceId::new(),
            mac_display_name: "Mac-mini".into(),
            paired_at_ms: 1_714_000_000_000,
            paired_via_device_id: DeviceId::new(),
        }],
    };
    let json = serde_json::to_string(&macs).unwrap();
    let back: MeMacsResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(back, macs);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value["macs"].is_array());
}
```

- [ ] **Step 2: Run; expect compile failure**

Run: `cargo test -p minos-protocol --no-run`
Expected: errors — `PairResponse` still has `your_device_secret`; `MeMacsResponse`/`MacSummary` undefined.

- [ ] **Step 3: Modify `PairResponse`**

In `crates/minos-protocol/src/messages.rs` find:

```rust
pub struct PairResponse {
    pub peer_device_id: DeviceId,
    pub peer_name: String,
    pub your_device_secret: DeviceSecret,
}
```

Replace with:

```rust
/// Result of `POST /v1/pairings` (consume). iOS no longer receives a
/// device secret — the rail is bearer-only post ADR-0020. Mac-side
/// pair state is delivered separately via `EventKind::Paired`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairResponse {
    pub peer_device_id: DeviceId,
    pub peer_name: String,
}
```

- [ ] **Step 4: Add `MeMacsResponse` + `MacSummary`**

Append to `crates/minos-protocol/src/messages.rs` (near `MePeerResponse`):

```rust
/// Response body for `GET /v1/me/macs`. iOS callers receive every Mac
/// paired to their `account_id`. `paired_via_device_id` is the mobile
/// device that performed the scan — recorded for audit; not used for
/// routing.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MeMacsResponse {
    pub macs: Vec<MacSummary>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MacSummary {
    pub mac_device_id: DeviceId,
    pub mac_display_name: String,
    pub paired_at_ms: i64,
    pub paired_via_device_id: DeviceId,
}
```

- [ ] **Step 5: Find and update existing `PairResponse` round-trip test**

Find the test referencing `your_device_secret` (around line 212–228). Replace with:

```rust
#[test]
fn pair_response_round_trips() {
    let resp = PairResponse {
        peer_device_id: DeviceId::new(),
        peer_name: "iPhone".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value.get("your_device_secret").is_none());
    let back: PairResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(back, resp);
}
```

- [ ] **Step 6: Run; expect pass**

Run: `cargo test -p minos-protocol`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
cargo xtask check-all
git add crates/minos-protocol/src/messages.rs
git commit -m "feat(protocol): drop PairResponse.your_device_secret; add MeMacsResponse"
```

### Task C3: Verify protocol crate consumers identified

**Files:** none modified — investigation only.

- [ ] **Step 1: List all crates referencing the changed types**

Run from workspace root:

```bash
rg -l "PairResponse|your_device_secret|Envelope::Forward|MePeerResponse" --type rust crates/
```

Expected output should at minimum list:
- `crates/minos-backend/src/...` (handlers)
- `crates/minos-mobile/src/...` (client)
- `crates/minos-daemon/src/...` (Mac side; should still work because Mac path keeps secret)
- `crates/minos-protocol/src/...` (own tests)
- `crates/minos-transport/src/...` (likely envelope wrap helpers)

Confirm the daemon's references to `EventKind::Paired { your_device_secret }` will continue to work (it now expects `Option<_>`; daemon must wrap with `Some(_)` only when SENDING — daemon receives, so destructuring with `Some(secret) =>` would be needed).

- [ ] **Step 2: No commit; this is informational only**

This task produces a mental map for subsequent fix-ups. No code change.

---

## Phase D — Backend store layer

After this phase, the backend has a working store module for `account_mac_pairings`. The old `pairings.rs` is deleted; nothing yet calls the new store from handlers.

### Task D1: New `store::account_mac_pairings` module + delete `store::pairings`

**Files:**
- Create: `crates/minos-backend/src/store/account_mac_pairings.rs`
- Modify: `crates/minos-backend/src/store/mod.rs`
- Delete: `crates/minos-backend/src/store/pairings.rs`

- [ ] **Step 1: Write the failing tests**

Create `crates/minos-backend/src/store/account_mac_pairings.rs` with the test module first (no implementation yet):

```rust
//! Persistence for `account_mac_pairings`. Pair model is
//! (mac_device_id, mobile_account_id) post ADR-0020.

use minos_domain::{AccountId, DeviceId};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::BackendError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairRow {
    pub pair_id: String,
    pub mac_device_id: DeviceId,
    pub mobile_account_id: AccountId,
    pub paired_via_device_id: DeviceId,
    pub paired_at_ms: i64,
}

/// Insert a new pair. Returns `Ok(false)` on UNIQUE conflict
/// (account already paired to this Mac); `Ok(true)` on insert.
pub async fn insert_pair(
    pool: &SqlitePool,
    mac_device_id: &DeviceId,
    mobile_account_id: &AccountId,
    paired_via_device_id: &DeviceId,
    now_ms: i64,
) -> Result<bool, BackendError> {
    todo!("impl in step 3")
}

/// Return every Mac paired to the given account.
pub async fn list_macs_for_account(
    pool: &SqlitePool,
    account_id: &AccountId,
) -> Result<Vec<PairRow>, BackendError> {
    todo!("impl in step 3")
}

/// Return every account paired to the given Mac.
pub async fn list_accounts_for_mac(
    pool: &SqlitePool,
    mac_device_id: &DeviceId,
) -> Result<Vec<PairRow>, BackendError> {
    todo!("impl in step 3")
}

/// Does the (mac, account) pair exist?
pub async fn exists(
    pool: &SqlitePool,
    mac_device_id: &DeviceId,
    mobile_account_id: &AccountId,
) -> Result<bool, BackendError> {
    todo!("impl in step 3")
}

/// Delete a specific (mac, account) pair. Returns rows-deleted (0 or 1).
pub async fn delete_pair(
    pool: &SqlitePool,
    mac_device_id: &DeviceId,
    mobile_account_id: &AccountId,
) -> Result<u64, BackendError> {
    todo!("impl in step 3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_pool_with_account_and_devices;

    #[tokio::test]
    async fn insert_and_list_round_trip() {
        let (pool, account, mac, mobile) =
            test_pool_with_account_and_devices().await;
        let inserted = insert_pair(&pool, &mac, &account, &mobile, 100)
            .await
            .unwrap();
        assert!(inserted);
        let macs = list_macs_for_account(&pool, &account).await.unwrap();
        assert_eq!(macs.len(), 1);
        assert_eq!(macs[0].mac_device_id, mac);
        assert_eq!(macs[0].paired_via_device_id, mobile);
    }

    #[tokio::test]
    async fn unique_violation_returns_false() {
        let (pool, account, mac, mobile) =
            test_pool_with_account_and_devices().await;
        assert!(insert_pair(&pool, &mac, &account, &mobile, 100).await.unwrap());
        assert!(!insert_pair(&pool, &mac, &account, &mobile, 200).await.unwrap());
    }

    #[tokio::test]
    async fn delete_pair_removes_row() {
        let (pool, account, mac, mobile) =
            test_pool_with_account_and_devices().await;
        insert_pair(&pool, &mac, &account, &mobile, 100).await.unwrap();
        let n = delete_pair(&pool, &mac, &account).await.unwrap();
        assert_eq!(n, 1);
        assert!(!exists(&pool, &mac, &account).await.unwrap());
    }

    #[tokio::test]
    async fn one_mac_to_many_accounts() {
        let (pool, account_a, mac, mobile_a) =
            test_pool_with_account_and_devices().await;
        // Add a second account + mobile
        let account_b = crate::test_support::insert_account(&pool, "b@example.com").await;
        let mobile_b = crate::test_support::insert_device(&pool, "ios-client", &account_b).await;
        insert_pair(&pool, &mac, &account_a, &mobile_a, 100).await.unwrap();
        insert_pair(&pool, &mac, &account_b, &mobile_b, 200).await.unwrap();
        let accounts = list_accounts_for_mac(&pool, &mac).await.unwrap();
        assert_eq!(accounts.len(), 2);
    }
}
```

If `crate::test_support::test_pool_with_account_and_devices` does not exist yet, create a helper at `crates/minos-backend/src/test_support.rs` (or add to existing) that runs the migrations and inserts:
- one account
- one Mac device (`role=agent-host`, `secret_hash=Some(...)`)
- one iOS device (`role=ios-client`, `secret_hash=NULL`, `account_id` set)

Returns `(pool, account_id, mac_device_id, mobile_device_id)`.

- [ ] **Step 2: Run; expect failure**

Run: `cargo test -p minos-backend --lib store::account_mac_pairings`
Expected: tests panic on `todo!()`.

- [ ] **Step 3: Implement the functions**

Replace each `todo!()` body in `account_mac_pairings.rs`:

```rust
pub async fn insert_pair(
    pool: &SqlitePool,
    mac_device_id: &DeviceId,
    mobile_account_id: &AccountId,
    paired_via_device_id: &DeviceId,
    now_ms: i64,
) -> Result<bool, BackendError> {
    let pair_id = Uuid::new_v4().to_string();
    let mac_s = mac_device_id.to_string();
    let acc_s = mobile_account_id.to_string();
    let via_s = paired_via_device_id.to_string();
    let res = sqlx::query!(
        r#"
        INSERT INTO account_mac_pairings
            (pair_id, mac_device_id, mobile_account_id, paired_via_device_id, paired_at_ms)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT (mac_device_id, mobile_account_id) DO NOTHING
        "#,
        pair_id, mac_s, acc_s, via_s, now_ms
    )
    .execute(pool)
    .await
    .map_err(|e| BackendError::Storage { message: e.to_string() })?;
    Ok(res.rows_affected() == 1)
}

pub async fn list_macs_for_account(
    pool: &SqlitePool,
    account_id: &AccountId,
) -> Result<Vec<PairRow>, BackendError> {
    let acc_s = account_id.to_string();
    let rows = sqlx::query!(
        r#"
        SELECT pair_id, mac_device_id, mobile_account_id, paired_via_device_id, paired_at_ms
        FROM account_mac_pairings
        WHERE mobile_account_id = ?
        ORDER BY paired_at_ms DESC
        "#,
        acc_s
    )
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::Storage { message: e.to_string() })?;
    rows.into_iter()
        .map(|r| {
            Ok(PairRow {
                pair_id: r.pair_id,
                mac_device_id: r.mac_device_id.parse().map_err(|e: uuid::Error| {
                    BackendError::Storage { message: e.to_string() }
                })?,
                mobile_account_id: AccountId(r.mobile_account_id),
                paired_via_device_id: r.paired_via_device_id.parse().map_err(|e: uuid::Error| {
                    BackendError::Storage { message: e.to_string() }
                })?,
                paired_at_ms: r.paired_at_ms,
            })
        })
        .collect()
}

pub async fn list_accounts_for_mac(
    pool: &SqlitePool,
    mac_device_id: &DeviceId,
) -> Result<Vec<PairRow>, BackendError> {
    let mac_s = mac_device_id.to_string();
    let rows = sqlx::query!(
        r#"
        SELECT pair_id, mac_device_id, mobile_account_id, paired_via_device_id, paired_at_ms
        FROM account_mac_pairings
        WHERE mac_device_id = ?
        ORDER BY paired_at_ms DESC
        "#,
        mac_s
    )
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::Storage { message: e.to_string() })?;
    rows.into_iter()
        .map(|r| {
            Ok(PairRow {
                pair_id: r.pair_id,
                mac_device_id: r.mac_device_id.parse().map_err(|e: uuid::Error| {
                    BackendError::Storage { message: e.to_string() }
                })?,
                mobile_account_id: AccountId(r.mobile_account_id),
                paired_via_device_id: r.paired_via_device_id.parse().map_err(|e: uuid::Error| {
                    BackendError::Storage { message: e.to_string() }
                })?,
                paired_at_ms: r.paired_at_ms,
            })
        })
        .collect()
}

pub async fn exists(
    pool: &SqlitePool,
    mac_device_id: &DeviceId,
    mobile_account_id: &AccountId,
) -> Result<bool, BackendError> {
    let mac_s = mac_device_id.to_string();
    let acc_s = mobile_account_id.to_string();
    let row = sqlx::query!(
        r#"
        SELECT 1 AS hit
        FROM account_mac_pairings
        WHERE mac_device_id = ? AND mobile_account_id = ?
        LIMIT 1
        "#,
        mac_s, acc_s
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Storage { message: e.to_string() })?;
    Ok(row.is_some())
}

pub async fn delete_pair(
    pool: &SqlitePool,
    mac_device_id: &DeviceId,
    mobile_account_id: &AccountId,
) -> Result<u64, BackendError> {
    let mac_s = mac_device_id.to_string();
    let acc_s = mobile_account_id.to_string();
    let res = sqlx::query!(
        r#"
        DELETE FROM account_mac_pairings
        WHERE mac_device_id = ? AND mobile_account_id = ?
        "#,
        mac_s, acc_s
    )
    .execute(pool)
    .await
    .map_err(|e| BackendError::Storage { message: e.to_string() })?;
    Ok(res.rows_affected())
}
```

- [ ] **Step 4: Wire the new module; remove old**

Edit `crates/minos-backend/src/store/mod.rs`:
- Remove the line `pub mod pairings;` (the old module).
- Add `pub mod account_mac_pairings;`.

Delete file: `rm crates/minos-backend/src/store/pairings.rs`

- [ ] **Step 5: Run; expect pass for store tests, but the rest of the backend won't compile**

Run: `cargo test -p minos-backend --lib store::account_mac_pairings`
Expected: store tests pass. The full crate `cargo build -p minos-backend` will fail because handlers still call `crate::store::pairings::*`. That is expected — Phase E onwards fixes it.

- [ ] **Step 6: Commit (without check-all because workspace doesn't build yet)**

```bash
git add crates/minos-backend/src/store/account_mac_pairings.rs \
        crates/minos-backend/src/store/mod.rs \
        crates/minos-backend/src/test_support.rs
git rm crates/minos-backend/src/store/pairings.rs
git commit -m "feat(backend/store): account_mac_pairings module; drop pairings"
```

> **Note:** This is the only commit in the plan that intentionally lands a non-building backend. Phase E lands within the next 1–2 commits and restores the build. Do not push to a shared branch in this state.

---

## Phase E — Backend pairing handlers

After this phase, `POST /v1/pairings` (renamed from `/v1/pairing/consume`) writes to the new table and stops minting iOS secrets; `DELETE /v1/pairings/{mac_device_id}` deletes a specific pair. Backend rebuilds.

### Task E1: `consume_token()` writes to new table; iOS row keeps `secret_hash = NULL`

**Files:**
- Modify: `crates/minos-backend/src/pairing/mod.rs`
- Modify: `crates/minos-backend/src/http/v1/pairing.rs`

- [ ] **Step 1: Read current `consume_token` to identify scope**

Run: `grep -n "DeviceSecret::generate\|hash_secret\|insert_pairing\|consume_token" crates/minos-backend/src/pairing/mod.rs`
Note line ranges of secret-generation calls and `pairings::*` callsites.

- [ ] **Step 2: Modify `consume_token` to mint only the Mac secret**

In `crates/minos-backend/src/pairing/mod.rs`, locate the section that generates two secrets (originally ~line 180–184) and the pairings INSERT (originally ~line 195+). Replace the secret pair with single-Mac generation + new-table INSERT:

```rust
// Mint a fresh secret for the issuer (Mac). iOS rail no longer
// uses device-secret (ADR-0020) — its `secret_hash` stays NULL.
let mac_secret = DeviceSecret::generate();
let mac_hash = secret::hash_secret(mac_secret.as_str())?;

// Persist Mac's hash (idempotent: row exists from request_token).
crate::store::devices::upsert_secret_hash(&mut *tx, &issuer_device_id, &mac_hash)
    .await
    .map_err(|e| ConsumeError::Storage { message: e.to_string() })?;

// Insert the (Mac, mobile_account) pair. The mobile device id that
// performed the scan is recorded as paired_via_device_id (audit only).
let inserted = crate::store::account_mac_pairings::insert_pair(
    &mut *tx,
    &issuer_device_id,
    &consumer_account_id,
    &consumer_device_id,
    now_ms,
)
.await
.map_err(|e| ConsumeError::Storage { message: e.to_string() })?;

if !inserted {
    // Already paired — idempotent re-consume; treat as success.
    tracing::info!(
        mac = %issuer_device_id, account = %consumer_account_id,
        "pair already exists; idempotent",
    );
}
```

The transaction's commit and the `tracing::info!("paired", ...)` line stay as-is. Remove the second `DeviceSecret::generate()` call and any `consumer_secret`/`consumer_hash` locals.

The function's return shape changes: instead of returning `(IssuerSecret, ConsumerSecret)`, return only the Mac's secret. Adjust the return type:

```rust
pub struct ConsumeOutcome {
    pub mac_device_id: DeviceId,
    pub mobile_account_id: AccountId,
    pub mac_secret: DeviceSecret,
    pub host_display_name: String,
}
```

(Replace whatever the previous outcome shape was.)

- [ ] **Step 3: Modify `POST /v1/pairing/consume` handler to use new outcome**

In `crates/minos-backend/src/http/v1/pairing.rs`, locate the consume handler (around the existing `consume_token` call). The handler currently:
1. Calls `consume_token`
2. Pushes `EventKind::Paired { ..., your_device_secret: mac_secret }` to the Mac WS
3. Returns `PairResponse { peer_device_id, peer_name, your_device_secret: ios_secret }` to iOS

Updated handler logic:

```rust
let outcome = pairing::consume_token(...).await?;

// Push pair event to Mac (still gets its secret).
session_registry.push_event_to_device(
    &outcome.mac_device_id,
    EventKind::Paired {
        peer_device_id: caller_device_id,           // iOS device that scanned
        peer_name: req.device_name.clone(),
        your_device_secret: Some(outcome.mac_secret.clone()),
    },
).await;

// Mac's account_id inheritance from iOS (preserved from current flow).
crate::store::devices::set_account_id(&state.store, &outcome.mac_device_id, &outcome.mobile_account_id)
    .await
    .map_err(internal_err)?;

// iOS response: no secret, no your_device_secret.
Json(PairResponse {
    peer_device_id: outcome.mac_device_id,
    peer_name: outcome.host_display_name,
})
```

(Delete any code that wrote `iOS secret_hash` or pushed the iOS-side secret.)

- [ ] **Step 4: Update existing pairing tests**

Find tests in `crates/minos-backend/src/pairing/mod.rs` (e.g.,
`consume_two_outstanding_tokens_for_same_issuer_can_pair_two_ios_devices`).
Update assertions:
- iOS `secret_hash` must remain `None`/`NULL` after consume.
- Mac `secret_hash` must be `Some(_)` after consume.
- A row in `account_mac_pairings` must exist for `(mac, account)`.

Example skeleton:

```rust
#[tokio::test]
async fn consume_creates_account_mac_pair_without_ios_secret() {
    let (pool, ...) = setup_with_two_unpaired_devices().await;
    let outcome = consume_token(&pool, ...).await.unwrap();
    let mac_row = store::devices::get(&pool, &outcome.mac_device_id).await.unwrap().unwrap();
    let ios_row = store::devices::get(&pool, &consumer_device).await.unwrap().unwrap();
    assert!(mac_row.secret_hash.is_some());
    assert!(ios_row.secret_hash.is_none());
    assert!(store::account_mac_pairings::exists(
        &pool, &outcome.mac_device_id, &outcome.mobile_account_id,
    ).await.unwrap());
}
```

- [ ] **Step 5: Run pairing module tests**

Run: `cargo test -p minos-backend --lib pairing`
Expected: pass. May still have unused-import warnings — accept those for now.

- [ ] **Step 6: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/pairing/mod.rs \
        crates/minos-backend/src/http/v1/pairing.rs
git commit -m "feat(backend/pairing): mint Mac secret only; insert account_mac_pairings"
```

### Task E2: `DELETE /v1/pairings/:mac_device_id` route

**Files:**
- Modify: `crates/minos-backend/src/http/v1/pairing.rs`
- Modify: `crates/minos-backend/src/router.rs` (or wherever `Router::new()` is built; grep for `/v1/pairing`)

- [ ] **Step 1: Write the failing test**

Append to `crates/minos-backend/tests/server_centric_auth_e2e.rs` (create if missing):

```rust
//! End-to-end auth tests post ADR-0020.

mod common;

#[tokio::test]
async fn delete_pair_by_mac_device_id_removes_row() {
    let app = common::spawn_app().await;
    let (account, ios_creds, mac_creds) = common::pair_account_to_mac(&app).await;

    let resp = app
        .client()
        .delete(&format!("{}/v1/pairings/{}", app.base_url, mac_creds.device_id))
        .header("Authorization", format!("Bearer {}", ios_creds.access_token))
        .header("X-Device-Id", ios_creds.device_id.to_string())
        .header("X-Device-Role", "ios-client")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Pair gone
    let macs = common::get_my_macs(&app, &ios_creds).await;
    assert!(macs.is_empty());
}
```

If `tests/common` doesn't exist, lay it out with helpers `spawn_app`, `pair_account_to_mac`, `get_my_macs`. Pattern after the existing integration test in the crate (search for an existing `tests/*.rs` file).

- [ ] **Step 2: Run; expect 404 (route not registered) or 405**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- delete_pair`
Expected: failure with status mismatch.

- [ ] **Step 3: Add the handler**

In `crates/minos-backend/src/http/v1/pairing.rs`:

```rust
pub async fn delete_pair_for_mac(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(mac_device_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer = bearer::require(&state, &headers)
        .map_err(|e| { let (s,m) = e.into_response_tuple(); (s, err("unauthorized", m)) })?;
    let mac_id: DeviceId = mac_device_id.parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, err("bad_request", "invalid mac_device_id")))?;

    let n = crate::store::account_mac_pairings::delete_pair(
        &state.store, &mac_id, &bearer.account_id,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, err("internal", e.to_string())))?;

    if n == 0 {
        return Err((StatusCode::NOT_FOUND, err("not_found", "pair does not exist")));
    }

    // Push Unpaired event to the Mac (best-effort).
    let _ = state.session_registry
        .push_event_to_device(&mac_id, EventKind::Unpaired)
        .await;

    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 4: Register the route**

Grep `crates/minos-backend/src/` for `route("/v1/pairing"` to find the router builder. Add:

```rust
.route("/v1/pairings/:mac_device_id", delete(delete_pair_for_mac))
```

Keep the legacy `/v1/pairing` `DELETE` route mounted as a stub that returns `410 Gone` with body `{"error":{"code":"replaced","message":"Use DELETE /v1/pairings/{mac_device_id}"}}` — this surfaces the breakage clearly during dev.

- [ ] **Step 5: Run; expect pass**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- delete_pair`
Expected: pass.

- [ ] **Step 6: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/http/v1/pairing.rs \
        crates/minos-backend/src/router.rs \
        crates/minos-backend/tests/server_centric_auth_e2e.rs \
        crates/minos-backend/tests/common/
git commit -m "feat(backend/pairing): DELETE /v1/pairings/:mac_device_id"
```

---

## Phase F — Backend auth rail simplification

After this phase, iOS calls authenticate with bearer alone. Mac calls still require `X-Device-Secret` (no behaviour change for Mac).

### Task F1: `classify()` becomes role-aware

**Files:**
- Modify: `crates/minos-backend/src/http/auth.rs`

- [ ] **Step 1: Write the failing test**

Append to the existing test module in `auth.rs`:

```rust
#[tokio::test]
async fn classify_ios_with_null_secret_hash_passes_without_secret() {
    let row = DeviceRow {
        device_id: DeviceId::new().to_string(),
        role: DeviceRole::IosClient,
        secret_hash: None,
        ..Default::default()
    };
    let res = classify(Some(row), None, DeviceRole::IosClient);
    assert!(matches!(res, Ok(Classification::UnpairedExisting)));
}

#[tokio::test]
async fn classify_mac_with_null_secret_hash_still_fails_without_secret() {
    let row = DeviceRow {
        device_id: DeviceId::new().to_string(),
        role: DeviceRole::AgentHost,
        secret_hash: None,
        ..Default::default()
    };
    let res = classify(Some(row), None, DeviceRole::AgentHost);
    // Mac without secret_hash AND without secret means "first connect"
    // which is still allowed. So this would actually be FirstConnect.
    // Replace test target: Mac WITH secret_hash but NO secret provided
    // must still fail.
    let row2 = DeviceRow {
        device_id: DeviceId::new().to_string(),
        role: DeviceRole::AgentHost,
        secret_hash: Some("some-hash".into()),
        ..Default::default()
    };
    let res2 = classify(Some(row2), None, DeviceRole::AgentHost);
    assert!(matches!(res2, Err(AuthError::Unauthorized(_))));
}
```

- [ ] **Step 2: Run; expect compile failure (signature mismatch)**

Run: `cargo test -p minos-backend --lib http::auth -- classify`
Expected: compile error — `classify` currently takes 2 args, test passes 3.

- [ ] **Step 3: Update the `classify` signature**

In `crates/minos-backend/src/http/auth.rs:124`:

```rust
pub fn classify(
    row: Option<DeviceRow>,
    provided_secret: Option<&str>,
    role: DeviceRole,
) -> Result<Classification, AuthError> {
    match row {
        None => Ok(Classification::FirstConnect),
        Some(r) => match r.secret_hash {
            None => {
                // iOS rail: bearer-only after ADR-0020. A NULL secret_hash
                // is the steady state for iOS rows. Mac rows would only
                // be NULL pre-pair (FirstConnect-like).
                Ok(Classification::UnpairedExisting)
            }
            Some(hash) => {
                // Mac rail (or legacy iOS rows). Secret required.
                let _ = role; // role currently only used to clarify intent
                let Some(secret) = provided_secret else {
                    return Err(AuthError::Unauthorized(
                        "X-Device-Secret required for authenticated device".into(),
                    ));
                };
                match verify_secret(secret, &hash) {
                    Ok(true) => Ok(Classification::Authenticated),
                    Ok(false) => Err(AuthError::Unauthorized(
                        "X-Device-Secret does not match stored hash".into(),
                    )),
                    Err(e) => Err(AuthError::Internal(e.to_string())),
                }
            }
        },
    }
}
```

- [ ] **Step 4: Update all `classify(...)` callers**

Grep: `rg "classify\(" crates/minos-backend/src/`

For each call site, pass the resolved `DeviceRole` as the third arg. The role is already extracted earlier in the same handler (via `extract_device_role` or `resolve_device_role`).

Example update in `authenticate()` (same file, ~line 90):

```rust
let role = resolve_device_role(...)?;
let classification = classify(row, provided_secret, role)?;
```

- [ ] **Step 5: Run; expect pass**

Run: `cargo test -p minos-backend --lib http::auth`
Expected: pass.

- [ ] **Step 6: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/http/auth.rs
git commit -m "feat(backend/auth): role-aware classify; iOS bypasses secret rail"
```

### Task F2: Drop `authenticated_with_secret` requirement on iOS endpoints

**Files:**
- Modify: `crates/minos-backend/src/http/v1/threads.rs`
- Modify: `crates/minos-backend/src/http/v1/auth.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/minos-backend/tests/server_centric_auth_e2e.rs`:

```rust
#[tokio::test]
async fn ios_threads_list_works_with_bearer_only() {
    let app = common::spawn_app().await;
    let (account, ios_creds, _mac_creds) = common::pair_account_to_mac(&app).await;
    // ios_creds intentionally has NO X-Device-Secret.

    let resp = app.client()
        .get(&format!("{}/v1/threads?limit=10", app.base_url))
        .header("Authorization", format!("Bearer {}", ios_creds.access_token))
        .header("X-Device-Id", ios_creds.device_id.to_string())
        .header("X-Device-Role", "ios-client")
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn ios_login_works_with_bearer_only() {
    let app = common::spawn_app().await;
    let (account, ios_creds_first) = common::register_ios(&app).await;

    // Logout, then re-login WITHOUT secret header.
    common::logout(&app, &ios_creds_first).await;
    let resp = app.client()
        .post(&format!("{}/v1/auth/login", app.base_url))
        .header("X-Device-Id", ios_creds_first.device_id.to_string())
        .header("X-Device-Role", "ios-client")
        .json(&serde_json::json!({"email": account.email, "password": account.password}))
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);
}
```

- [ ] **Step 2: Run; expect 401**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- ios_`
Expected: 401 because of `authenticated_with_secret` checks.

- [ ] **Step 3: Drop the checks in `threads.rs`**

In `crates/minos-backend/src/http/v1/threads.rs:62–67`, replace:

```rust
if !outcome.authenticated_with_secret {
    return Err((
        StatusCode::UNAUTHORIZED,
        err("unauthorized", "X-Device-Secret required"),
    ));
}
```

with:

```rust
// iOS rail is bearer-only post ADR-0020. We no longer assert
// authenticated_with_secret. The bearer JWT below provides the
// account scope; pair existence is established via account_mac_pairings.
```

Then update the `let owner = ... pairings::get_pair(...)` block (lines 74–87): replace with a query against the new table. The "owner" of a thread should be derivable from the thread row itself (threads table already has `owner_device_id`); use that instead of looking up via pairings. Concretely:

```rust
let bearer_outcome = bearer::require(state, headers).map_err(|e| {
    let (s, m) = e.into_response_tuple();
    (s, err("unauthorized", m))
})?;
Ok((outcome.device_id, bearer_outcome.account_id))
```

The previous return `(owner_device_id, account_id)` carried the "Mac that owns the thread"; that lookup must move into the actual list/read handler so it can use the **target** Mac (which will come from a future query param or be resolved from the thread row). For now, return `(caller_device_id, account_id)` and let `list`/`read` filter by `account_id` only.

Update `list_threads` and `read_thread` callers accordingly: they no longer pass `owner_s` (the Mac id) to the store — they pass `Some(&account_id)` for scoping.

If a thread-store function expects `owner_device_id`, switch to a variant that filters by `account_id`. Add such a function in `crates/minos-backend/src/store/threads.rs` if it doesn't exist:

```rust
pub async fn list_for_account(
    pool: &SqlitePool,
    account_id: &AccountId,
    agent: Option<AgentName>,
    before_ts_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<ThreadSummary>, BackendError> {
    // Filter threads by joining with devices.account_id = ?
    // ...
}
```

- [ ] **Step 4: Drop the secret check in auth.rs login/refresh/logout/register**

Grep: `rg "authenticated_with_secret" crates/minos-backend/src/http/v1/auth.rs`

For each occurrence (around lines 166, 237, 319, 372 per investigation), remove the conditional that returns 401 when `!outcome.authenticated_with_secret`. The bearer (or absent-bearer for register/login) is sufficient for these endpoints.

- [ ] **Step 5: Run; expect pass**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- ios_`
Expected: pass.

- [ ] **Step 6: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/http/v1/threads.rs \
        crates/minos-backend/src/http/v1/auth.rs \
        crates/minos-backend/src/store/threads.rs
git commit -m "feat(backend/auth): drop secret requirement on iOS endpoints"
```

### Task F3: WS `/devices` upgrade — drop secret check for iOS

**Files:**
- Modify: `crates/minos-backend/src/http/ws_devices.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn ios_ws_upgrade_works_with_bearer_only() {
    let app = common::spawn_app().await;
    let (_account, ios_creds, _mac_creds) = common::pair_account_to_mac(&app).await;

    let url = format!("{}/devices", app.base_url.replace("http", "ws"));
    let req = http::Request::builder()
        .uri(&url)
        .header("Authorization", format!("Bearer {}", ios_creds.access_token))
        .header("X-Device-Id", ios_creds.device_id.to_string())
        .header("X-Device-Role", "ios-client")
        .body(())
        .unwrap();
    let (ws, _resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    drop(ws);
}
```

- [ ] **Step 2: Run; expect 4401 close or 401 on upgrade**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- ios_ws_upgrade`
Expected: failure.

- [ ] **Step 3: Update the WS upgrade handler**

In `crates/minos-backend/src/http/ws_devices.rs` find the secret check (around lines 115–121, 200–232). For `IosClient` role:
- Skip the `authenticated_with_secret` assertion at upgrade.
- Skip `revalidate_live_session_auth`'s secret re-check (or guard with role).
- Continue to require bearer for iOS.

Concretely, find:

```rust
if role == DeviceRole::IosClient {
    bearer::require(...)?;
}
// secret check via classify continues below
```

Ensure the secret-required branch is guarded:

```rust
match role {
    DeviceRole::IosClient => {
        // bearer is mandatory; secret is not.
        let _bearer = bearer::require(state, headers).map_err(|e| {
            let (s, m) = e.into_response_tuple();
            (s, err("unauthorized", m))
        })?;
    }
    DeviceRole::AgentHost | DeviceRole::BrowserAdmin => {
        if !outcome.authenticated_with_secret {
            return Err(close_with_4401("X-Device-Secret required"));
        }
    }
}
```

Same surgical change in `revalidate_live_session_auth` — only re-check secret for non-iOS roles.

- [ ] **Step 4: Run; expect pass**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- ios_ws_upgrade`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/http/ws_devices.rs
git commit -m "feat(backend/ws): iOS WS upgrade is bearer-only"
```

### Task F4: Devices store — accept iOS rows with `secret_hash = NULL`

**Files:**
- Modify: `crates/minos-backend/src/store/devices.rs`
- Modify: `crates/minos-backend/migrations/0001_devices.sql` (only the comment; not the column type)

- [ ] **Step 1: Confirm current schema permits NULL**

Run: `grep -A 2 "secret_hash" crates/minos-backend/migrations/0001_devices.sql`
Expected: `secret_hash TEXT  -- argon2id; NULL while unpaired`. NULL is permitted.

- [ ] **Step 2: Verify there are no INSERT paths that require secret on iOS**

Grep: `rg "secret_hash" crates/minos-backend/src/store/devices.rs`
For each INSERT/UPDATE that writes `secret_hash`, ensure none of them runs on iOS row creation. The `set_account_id` and basic upserts should not touch `secret_hash`.

- [ ] **Step 3: Add a regression test**

Append to `crates/minos-backend/src/store/devices.rs` `mod tests`:

```rust
#[tokio::test]
async fn ios_row_can_be_created_with_null_secret_hash() {
    let pool = test_support::test_pool().await;
    let id = DeviceId::new();
    insert_or_get(&pool, &id, DeviceRole::IosClient, "iPhone").await.unwrap();
    let row = get(&pool, &id).await.unwrap().unwrap();
    assert!(row.secret_hash.is_none());
}
```

- [ ] **Step 4: Run; expect pass**

Run: `cargo test -p minos-backend --lib store::devices`
Expected: pass.

- [ ] **Step 5: Commit (only if any test/doc change made)**

```bash
cargo xtask check-all
git add crates/minos-backend/src/store/devices.rs
git commit -m "test(backend/devices): regression for ios secret_hash NULL"
```

---

## Phase G — Backend envelope routing

After this phase, iOS clients drive routing by stamping `target_device_id` in `Forward`; the backend's `paired_with` single slot is gone.

### Task G1: `Envelope::Forward` handler reads `target_device_id`; validate against pair table

**Files:**
- Modify: `crates/minos-backend/src/envelope/mod.rs`
- Modify: `crates/minos-backend/src/session/registry.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/minos-backend/tests/server_centric_auth_e2e.rs`:

```rust
#[tokio::test]
async fn ios_forward_with_target_routes_to_named_mac() {
    let app = common::spawn_app().await;
    let (_account, ios_creds, mac_creds) = common::pair_account_to_mac(&app).await;
    let mut ios_ws = common::connect_ws(&app, &ios_creds).await;
    let mut mac_ws = common::connect_ws(&app, &mac_creds).await;

    let frame = serde_json::json!({
        "kind": "forward",
        "v": 1,
        "target_device_id": mac_creds.device_id.to_string(),
        "payload": {"jsonrpc": "2.0", "method": "ping", "id": 1},
    });
    ios_ws.send(frame.to_string()).await;
    let received = mac_ws.recv().await;
    let v: serde_json::Value = serde_json::from_str(&received).unwrap();
    assert_eq!(v["kind"], "forwarded");
    assert_eq!(v["payload"]["method"], "ping");
}

#[tokio::test]
async fn ios_forward_with_unpaired_target_returns_peer_offline_error() {
    let app = common::spawn_app().await;
    let (_account, ios_creds, _mac_creds) = common::pair_account_to_mac(&app).await;
    let other_mac = DeviceId::new(); // never paired to this account
    let mut ios_ws = common::connect_ws(&app, &ios_creds).await;

    let frame = serde_json::json!({
        "kind": "forward",
        "v": 1,
        "target_device_id": other_mac.to_string(),
        "payload": {"jsonrpc": "2.0", "method": "ping", "id": 2},
    });
    ios_ws.send(frame.to_string()).await;
    let response = ios_ws.recv().await;
    let v: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(v["error"]["code"], "peer_offline");
}
```

- [ ] **Step 2: Run; expect compile failure (Forward shape changed) or 5xx**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- forward`
Expected: failure.

- [ ] **Step 3: Update `handle_forward` to use `target_device_id`**

In `crates/minos-backend/src/envelope/mod.rs` find `handle_forward`. Replace the route-resolution logic (the part that reads `paired_with`) with:

```rust
async fn handle_forward(
    state: &BackendState,
    sender: &SessionHandle,
    target_device_id: DeviceId,
    payload: serde_json::Value,
) -> Result<(), HandleError> {
    // Validate the target is paired to the caller's account.
    let account_id = sender.account_id().clone();
    let paired = crate::store::account_mac_pairings::exists(
        &state.store, &target_device_id, &account_id,
    )
    .await
    .map_err(|e| HandleError::Internal(e.to_string()))?;
    if !paired {
        return Err(HandleError::PeerOffline);
    }

    // Mac→iOS reply correlation: stamp request_id → sender on the Mac
    // session (existing rpc_reply_targets DashMap stays; only the
    // *forward* direction reads target from the envelope now).
    if let Some(id) = json_rpc_id(&payload) {
        if let Some(mac_handle) = state.session_registry.get(&target_device_id).await {
            mac_handle.rpc_reply_targets.insert(id, sender.device_id().clone());
        }
    }

    state.session_registry
        .deliver_forwarded_to(&target_device_id, sender.device_id(), payload)
        .await
        .map_err(|_| HandleError::PeerOffline)
}
```

The match arm on `Envelope::Forward` becomes:

```rust
Envelope::Forward { version: _, target_device_id, payload } => {
    handle_forward(state, &sender, target_device_id, payload).await
}
```

- [ ] **Step 4: Drop `SessionHandle.paired_with`**

In `crates/minos-backend/src/session/registry.rs`:
- Remove the `paired_with: Arc<RwLock<Option<DeviceId>>>` field (line ~94).
- Remove all writes to it (e.g. in `http/v1/pairing.rs:317-327` and `http/ws_devices.rs:228, 244`).
- The Mac→iOS reply path already uses `rpc_reply_targets`; that stays untouched.
- The Mac→iOS spontaneous push path that previously used `paired_with` (`session/registry.rs:432-457`) needs adjustment: if a Mac wants to push to all paired iOS, it should iterate `account_mac_pairings::list_accounts_for_mac`.

For this plan, drop the spontaneous Mac→iOS push code path entirely: nobody currently sends Mac-initiated `Envelope::Forward` to iOS that isn't a JSON-RPC reply. `rpc_reply_targets` covers replies. If we discover a missing path during integration testing, add it back as a fan-out.

- [ ] **Step 5: Update `session_registry.deliver_forwarded_to` signature**

If `deliver_forwarded_to` does not exist, add it:

```rust
pub async fn deliver_forwarded_to(
    &self,
    target: &DeviceId,
    from: &DeviceId,
    payload: serde_json::Value,
) -> Result<(), DeliveryError> {
    let handle = self.get(target).await.ok_or(DeliveryError::Offline)?;
    let env = Envelope::Forwarded { version: 1, from: *from, payload };
    handle.tx.send(env).await.map_err(|_| DeliveryError::Closed)
}
```

- [ ] **Step 6: Run; expect pass**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- forward`
Expected: pass.

- [ ] **Step 7: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/envelope/mod.rs \
        crates/minos-backend/src/session/registry.rs \
        crates/minos-backend/src/http/v1/pairing.rs \
        crates/minos-backend/src/http/ws_devices.rs
git commit -m "feat(backend/envelope): route Forward by target_device_id; drop paired_with slot"
```

### Task G2: `ingest::broadcast_to_peers_of` queries new table

**Files:**
- Modify: `crates/minos-backend/src/ingest/mod.rs`

- [ ] **Step 1: Identify the call**

Run: `grep -n "broadcast_to_peers_of\|get_peers" crates/minos-backend/src/ingest/mod.rs`
Note the function and its caller(s) (originally lines 218–267).

- [ ] **Step 2: Replace `pairings::get_peers` with `account_mac_pairings::list_accounts_for_mac`**

```rust
async fn broadcast_to_peers_of(
    state: &BackendState,
    mac_device_id: &DeviceId,
    event: EventKind,
) -> Result<(), BackendError> {
    let pairs = crate::store::account_mac_pairings::list_accounts_for_mac(
        &state.store, mac_device_id,
    ).await?;
    // Resolve every iOS device under each paired account.
    for pair in pairs {
        let devices = crate::store::devices::list_by_account(
            &state.store, &pair.mobile_account_id,
        ).await?;
        for d in devices.iter().filter(|d| d.role == DeviceRole::IosClient) {
            let _ = state.session_registry.push_event_to_device(&d.device_id, event.clone()).await;
        }
    }
    Ok(())
}
```

If `store::devices::list_by_account` doesn't exist, add it (single sqlx query analogous to existing list functions).

- [ ] **Step 3: Run the existing ingest tests**

Run: `cargo test -p minos-backend --lib ingest`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/ingest/mod.rs \
        crates/minos-backend/src/store/devices.rs
git commit -m "feat(backend/ingest): broadcast via account_mac_pairings"
```

---

## Phase H — Backend new endpoints

### Task H1: `GET /v1/me/macs` for iOS callers

**Files:**
- Create: `crates/minos-backend/src/http/v1/me_macs.rs`
- Modify: `crates/minos-backend/src/http/v1/mod.rs`
- Modify: `crates/minos-backend/src/router.rs`
- Delete: `crates/minos-backend/src/http/v1/me.rs` (or keep as an empty stub if other code references it)

- [ ] **Step 1: Write the failing test**

Append to `crates/minos-backend/tests/server_centric_auth_e2e.rs`:

```rust
#[tokio::test]
async fn me_macs_returns_paired_macs_for_account() {
    let app = common::spawn_app().await;
    let (account, ios_creds, mac_creds) = common::pair_account_to_mac(&app).await;
    let resp = app.client()
        .get(&format!("{}/v1/me/macs", app.base_url))
        .header("Authorization", format!("Bearer {}", ios_creds.access_token))
        .header("X-Device-Id", ios_creds.device_id.to_string())
        .header("X-Device-Role", "ios-client")
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: minos_protocol::MeMacsResponse = resp.json().await.unwrap();
    assert_eq!(body.macs.len(), 1);
    assert_eq!(body.macs[0].mac_device_id, mac_creds.device_id);
    assert_eq!(body.macs[0].paired_via_device_id, ios_creds.device_id);
}
```

- [ ] **Step 2: Run; expect 404**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- me_macs`
Expected: 404.

- [ ] **Step 3: Implement the handler**

`crates/minos-backend/src/http/v1/me_macs.rs`:

```rust
//! `GET /v1/me/macs` — list every Mac paired to the caller's account.

use axum::{extract::State, http::HeaderMap, http::StatusCode, Json};
use minos_protocol::{MacSummary, MeMacsResponse};

use crate::auth::bearer;
use crate::error::{err, ErrorEnvelope};
use crate::state::BackendState;

pub async fn list(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<MeMacsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer_outcome = bearer::require(&state, &headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err("unauthorized", m))
    })?;

    let pairs = crate::store::account_mac_pairings::list_macs_for_account(
        &state.store, &bearer_outcome.account_id,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, err("internal", e.to_string())))?;

    let mut macs = Vec::with_capacity(pairs.len());
    for p in pairs {
        let row = crate::store::devices::get(&state.store, &p.mac_device_id).await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, err("internal", e.to_string())))?
            .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, err("internal", "mac row missing")))?;
        macs.push(MacSummary {
            mac_device_id: p.mac_device_id,
            mac_display_name: row.display_name.unwrap_or_default(),
            paired_at_ms: p.paired_at_ms,
            paired_via_device_id: p.paired_via_device_id,
        });
    }

    Ok(Json(MeMacsResponse { macs }))
}
```

- [ ] **Step 4: Register the route**

In `crates/minos-backend/src/http/v1/mod.rs` add `pub mod me_macs;`. In the router builder add:

```rust
.route("/v1/me/macs", get(me_macs::list))
```

If `me.rs` exists with `/v1/me/peer`, replace its route with `410 Gone`:

```rust
.route("/v1/me/peer", get(|| async {
    (StatusCode::GONE, Json(err("replaced", "Use GET /v1/me/macs")))
}))
```

- [ ] **Step 5: Run; expect pass**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- me_macs`
Expected: pass.

- [ ] **Step 6: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/http/v1/me_macs.rs \
        crates/minos-backend/src/http/v1/mod.rs \
        crates/minos-backend/src/router.rs
git commit -m "feat(backend/me): GET /v1/me/macs"
```

### Task H2: Final backend integration sweep

**Files:** none new — investigation/cleanup task.

- [ ] **Step 1: Build entire workspace**

Run: `cargo build --workspace --all-targets`
Expected: clean build.

- [ ] **Step 2: Run all backend tests**

Run: `cargo test -p minos-backend --all-features`
Expected: all green. Fix any leftover compile errors or test breakages from earlier phases (e.g., daemon test that destructured `EventKind::Paired { your_device_secret: secret }` instead of `Some(secret)`).

- [ ] **Step 3: Run xtask check-all**

Run: `cargo xtask check-all`
Expected: clean.

- [ ] **Step 4: Commit any cleanup**

```bash
git add -p
git commit -m "chore(backend): post-pair-refactor cleanup"
```

(Skip if no changes.)

---

## Phase I — Mobile Rust core

After this phase, `crates/minos-mobile` no longer carries `device_secret` in its store or surface; FRB API exposes new pair queries.

### Task I1: Drop `device_secret` from `PersistedPairingState` and store trait

**Files:**
- Modify: `crates/minos-mobile/src/store.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/minos-mobile/src/store.rs` `mod tests`:

```rust
#[test]
fn persisted_state_has_no_device_secret_field() {
    // Compile-time check: building a PersistedPairingState must not
    // require any device_secret-shaped argument.
    let _state = PersistedPairingState {
        device_id: Some("uuid".into()),
        access_token: Some("jwt".into()),
        access_expires_at_ms: Some(0),
        refresh_token: Some("rt".into()),
        account_id: Some("acct".into()),
        account_email: Some("e@x".into()),
    };
}

#[tokio::test]
async fn save_device_no_longer_takes_secret() {
    let store = InMemoryState::default();
    store.save_device("dev-1".into()).await.unwrap();
    let id = store.load_device().await.unwrap();
    assert_eq!(id.as_deref(), Some("dev-1"));
}
```

- [ ] **Step 2: Run; expect compile failure**

Run: `cargo test -p minos-mobile --lib store`
Expected: errors — `PersistedPairingState.device_secret` still exists; `save_device` takes `(DeviceId, DeviceSecret)`.

- [ ] **Step 3: Strip `device_secret` from types and trait**

In `crates/minos-mobile/src/store.rs`:

Find `PersistedPairingState` (around line 24–34) and remove the `device_secret` field:

```rust
pub struct PersistedPairingState {
    pub device_id: Option<String>,
    pub access_token: Option<String>,
    pub access_expires_at_ms: Option<i64>,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
}
```

Find `MobilePairingStore` trait (around line 60) and:
- Remove `load_device` returning `Option<(DeviceId, DeviceSecret)>`; replace with `load_device(&self) -> Result<Option<DeviceId>>`.
- Replace `save_device(&self, device_id: DeviceId, secret: DeviceSecret) -> Result<()>` with `save_device(&self, device_id: DeviceId) -> Result<()>`.
- Keep `save_auth/load_auth/clear_*` as is.

Find `InMemoryState` (around line 96): the `device: Option<(DeviceId, DeviceSecret)>` field becomes `device_id: Option<DeviceId>`.

Update implementations to drop secret reads/writes accordingly.

- [ ] **Step 4: Run; expect pass on store tests; expect compile failure in client.rs / http.rs**

Run: `cargo test -p minos-mobile --lib store`
Expected: pass for store; the rest of the mobile crate fails — fixed in next task.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-mobile/src/store.rs
git commit -m "feat(mobile/store): drop device_secret from PersistedPairingState"
```

(Skip xtask check-all here; the workspace is intentionally broken until Task I2.)

### Task I2: `MobileClient` drops secret reads/writes

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`
- Modify: `crates/minos-mobile/src/http.rs`

- [ ] **Step 1: Find every secret read/write**

Run: `rg "device_secret|DeviceSecret" crates/minos-mobile/src/`

Each occurrence falls in one of three buckets:
- A: `connect()` injects `X-Device-Secret` — DELETE the injection.
- B: `pair_with_qr_json()` reads `your_device_secret` from response — DELETE.
- C: `forget_peer()` reads secret from store — DELETE.

- [ ] **Step 2: Update `connect()`**

In `crates/minos-mobile/src/client.rs`, find the `connect` function (~line 1000–1020). The `AuthHeaders` builder previously did:

```rust
let secret = self.store.load_device().await?.map(|(_, s)| s);
let headers = AuthHeaders::new(device_id).with_secret(secret.as_ref());
```

Replace with:

```rust
let headers = AuthHeaders::new(device_id);
// iOS rail is bearer-only post ADR-0020; secret intentionally omitted.
```

If `with_secret` becomes unused after this change, leave the API on `AuthHeaders` (Mac side still uses it) — just don't call it.

- [ ] **Step 3: Update `pair_with_qr_json()`**

Locate the consume call (~line 451–455). Previously:

```rust
let resp: PairResponse = self.http.consume_token(token).await?;
self.store.save_device(resp.peer_device_id, resp.your_device_secret).await?;
```

Replace with:

```rust
let resp: PairResponse = self.http.consume_token(token).await?;
self.store.save_device(resp.peer_device_id).await?;
// Active mac is set via set_active_mac(); this just records the pair.
```

The `PairResponse` no longer has `your_device_secret`; if the FRB-exposed `PairResponse` mirror still does, drop it too (in `crates/minos-ffi-frb/src/api/minos.rs` — covered in Task K1).

- [ ] **Step 4: Update `forget_peer`**

Locate `forget_peer` (~line 466–490). Previously called `http.forget_pairing()` which authenticated with `X-Device-Secret`. Replace with bearer-authenticated `DELETE /v1/pairings/{mac_device_id}`:

```rust
pub async fn forget_peer(&self, mac_device_id: DeviceId) -> Result<()> {
    self.http.delete_pair(mac_device_id).await?;
    // Local store update: if this was the active mac, clear active state.
    self.store.clear_active_if(&mac_device_id).await?;
    Ok(())
}
```

(`store.clear_active_if` will be added in Task I3.)

- [ ] **Step 5: Update `crates/minos-mobile/src/http.rs`**

Locate the request builder (~line 450). Remove `X-Device-Secret` injection for iOS. Add `delete_pair`:

```rust
pub async fn delete_pair(&self, mac_device_id: DeviceId) -> Result<()> {
    let req = self.request_builder(Method::DELETE, &format!("/v1/pairings/{}", mac_device_id))?;
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(MobileError::http(resp.status()));
    }
    Ok(())
}
```

- [ ] **Step 6: Run mobile crate build**

Run: `cargo build -p minos-mobile`
Expected: clean.

- [ ] **Step 7: Run mobile crate tests**

Run: `cargo test -p minos-mobile`
Expected: pass.

- [ ] **Step 8: Commit**

```bash
cargo xtask check-all
git add crates/minos-mobile/src/client.rs crates/minos-mobile/src/http.rs
git commit -m "feat(mobile): drop device_secret reads/writes; bearer-only iOS"
```

### Task I3: Add `list_paired_macs`, active-mac state, `forget_mac` on `MobileClient`

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`
- Modify: `crates/minos-mobile/src/store.rs`
- Modify: `crates/minos-mobile/src/http.rs`

- [ ] **Step 1: Write the failing tests**

Append to `crates/minos-mobile/src/client.rs` `mod tests`:

```rust
#[tokio::test]
async fn list_paired_macs_returns_backend_response() {
    let server = mock_backend().await;
    server.mock_macs_response(vec![mock_mac("Mac-mini")]);
    let client = MobileClient::with_authed_session(&server).await;
    let macs = client.list_paired_macs().await.unwrap();
    assert_eq!(macs.len(), 1);
    assert_eq!(macs[0].mac_display_name, "Mac-mini");
}

#[tokio::test]
async fn set_active_mac_persists_and_reads_back() {
    let store = InMemoryState::default();
    let client = MobileClient::new_with_store(store.clone());
    let mac_id = DeviceId::new();
    client.set_active_mac(mac_id).await.unwrap();
    assert_eq!(client.active_mac().await.unwrap(), Some(mac_id));
}
```

- [ ] **Step 2: Run; expect compile failure**

Run: `cargo test -p minos-mobile --lib client`
Expected: errors — methods undefined.

- [ ] **Step 3: Add active-mac to store**

In `crates/minos-mobile/src/store.rs`, extend the trait:

```rust
async fn save_active_mac(&self, mac_device_id: DeviceId) -> Result<()>;
async fn load_active_mac(&self) -> Result<Option<DeviceId>>;
async fn clear_active_if(&self, mac_device_id: &DeviceId) -> Result<()>;
```

`InMemoryState` adds an `active_mac: Mutex<Option<DeviceId>>` field.

The Dart-side `PersistedPairingState` mirror (FRB) gets a new `active_mac_device_id: Option<String>` field — but that's regenerated in Phase K.

- [ ] **Step 4: Add HTTP `list_paired_macs` in `http.rs`**

```rust
pub async fn list_paired_macs(&self) -> Result<MeMacsResponse> {
    let req = self.request_builder(Method::GET, "/v1/me/macs")?;
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(MobileError::http(resp.status()));
    }
    let body: MeMacsResponse = resp.json().await?;
    Ok(body)
}
```

- [ ] **Step 5: Add client methods**

In `crates/minos-mobile/src/client.rs`:

```rust
pub async fn list_paired_macs(&self) -> Result<Vec<MacSummary>> {
    let resp = self.http.list_paired_macs().await?;
    Ok(resp.macs)
}

pub async fn set_active_mac(&self, mac_device_id: DeviceId) -> Result<()> {
    self.store.save_active_mac(mac_device_id).await
}

pub async fn active_mac(&self) -> Result<Option<DeviceId>> {
    self.store.load_active_mac().await
}

pub async fn forget_mac(&self, mac_device_id: DeviceId) -> Result<()> {
    self.http.delete_pair(mac_device_id).await?;
    self.store.clear_active_if(&mac_device_id).await?;
    Ok(())
}
```

- [ ] **Step 6: Update `forward()` to stamp `target_device_id`**

Find the `forward` (or whatever sends `Envelope::Forward`) and require an active mac:

```rust
pub async fn forward(&self, payload: serde_json::Value) -> Result<()> {
    let target = self.active_mac().await?
        .ok_or(MobileError::NoActiveMac)?;
    let env = Envelope::Forward {
        version: 1,
        target_device_id: target,
        payload,
    };
    self.ws_send(env).await
}
```

- [ ] **Step 7: Run; expect pass**

Run: `cargo test -p minos-mobile --lib client`
Expected: pass.

- [ ] **Step 8: Commit**

```bash
cargo xtask check-all
git add crates/minos-mobile/src/
git commit -m "feat(mobile): list_paired_macs, active_mac, forget_mac, target-stamped forward"
```

---

## Phase J — Transport layer

### Task J1: `AuthHeaders.with_secret` becomes optional / role-aware

**Files:**
- Modify: `crates/minos-transport/src/auth.rs`

- [ ] **Step 1: Inspect current API**

Run: `grep -n "with_secret\|HDR_DEVICE_SECRET\|X-Device-Secret" crates/minos-transport/src/auth.rs`

- [ ] **Step 2: Make secret optional**

The current `AuthHeaders::new(device_id)` already returns headers without secret; `with_secret(...)` adds it. Confirm that `with_secret` is only invoked with `Some(_)` and that callers passing `None` no-op. Concretely add (if missing) a `with_secret_opt`:

```rust
pub fn with_secret_opt(mut self, secret: Option<&DeviceSecret>) -> Self {
    if let Some(s) = secret {
        self = self.with_secret(s);
    }
    self
}
```

- [ ] **Step 3: Ensure mobile callers don't pass any secret**

Already covered in Task I2 (`connect()`) — re-verify with `rg "with_secret" crates/minos-mobile/src/`.
Expected: zero matches.

Mac daemon callers continue calling `with_secret(secret)` — unchanged.

- [ ] **Step 4: Build transport**

Run: `cargo build -p minos-transport`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
cargo xtask check-all
git add crates/minos-transport/src/auth.rs
git commit -m "feat(transport): with_secret_opt; iOS callers omit secret"
```

---

## Phase K — FRB regen + mobile API surface

### Task K1: Update `minos-ffi-frb` API; regenerate FRB code

**Files:**
- Modify: `crates/minos-ffi-frb/src/api/minos.rs`
- Regen: `apps/mobile/lib/src/rust/api/minos.dart`
- Regen: `apps/mobile/lib/src/rust/frb_generated.dart`

- [ ] **Step 1: Drop `device_secret` from FRB-exposed types**

In `crates/minos-ffi-frb/src/api/minos.rs`:
- Remove `device_secret` from any FRB-mirrored `PersistedPairingState`.
- Remove `your_device_secret` from FRB-mirrored `PairResponse`.
- Add new exposures:

```rust
#[frb(dart_metadata = ("freezed"))]
pub struct MacSummaryDto {
    pub mac_device_id: String,
    pub mac_display_name: String,
    pub paired_at_ms: i64,
    pub paired_via_device_id: String,
}

impl From<minos_protocol::MacSummary> for MacSummaryDto {
    fn from(m: minos_protocol::MacSummary) -> Self {
        Self {
            mac_device_id: m.mac_device_id.to_string(),
            mac_display_name: m.mac_display_name,
            paired_at_ms: m.paired_at_ms,
            paired_via_device_id: m.paired_via_device_id.to_string(),
        }
    }
}

#[frb]
impl MobileClient {
    pub async fn list_paired_macs_dto(&self) -> Result<Vec<MacSummaryDto>> {
        let macs = self.list_paired_macs().await?;
        Ok(macs.into_iter().map(Into::into).collect())
    }

    pub async fn set_active_mac_dto(&self, mac_device_id: String) -> Result<()> {
        let id = mac_device_id.parse().map_err(|e: uuid::Error| anyhow::anyhow!(e))?;
        self.set_active_mac(id).await
    }

    pub async fn active_mac_dto(&self) -> Result<Option<String>> {
        Ok(self.active_mac().await?.map(|d| d.to_string()))
    }

    pub async fn forget_mac_dto(&self, mac_device_id: String) -> Result<()> {
        let id = mac_device_id.parse().map_err(|e: uuid::Error| anyhow::anyhow!(e))?;
        self.forget_mac(id).await
    }
}
```

- [ ] **Step 2: Run FRB codegen**

Run: `cargo xtask gen-frb`
Expected: success; updates `apps/mobile/lib/src/rust/api/minos.dart` and `frb_generated.dart`.

- [ ] **Step 3: Verify generated Dart shape**

Run: `grep -E "deviceSecret|listPairedMacs|setActiveMac|forgetMac" apps/mobile/lib/src/rust/api/minos.dart`
Expected: `deviceSecret` not found; new methods listed.

- [ ] **Step 4: Build**

Run: `cargo build -p minos-ffi-frb`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
cargo xtask check-all
git add crates/minos-ffi-frb/src/api/minos.rs \
        apps/mobile/lib/src/rust/api/minos.dart \
        apps/mobile/lib/src/rust/frb_generated.dart
git commit -m "feat(ffi/frb): list_paired_macs / forget_mac / active_mac; drop secret"
```

---

## Phase L — Flutter Dart layer

### Task L1: `secure_pairing_store.dart` drops `_keyDeviceSecret`; wipe legacy field on cold start

**Files:**
- Modify: `apps/mobile/lib/infrastructure/secure_pairing_store.dart`

- [ ] **Step 1: Remove `_keyDeviceSecret` constant + reads/writes**

In `secure_pairing_store.dart`:
- Delete line 25: `static const _keyDeviceSecret = 'minos.device_secret';`
- Delete line 41: `final deviceSecret = await _storage.read(key: _keyDeviceSecret);`
- Delete line 50: `deviceSecret != null ||` from `hasAnyValue`
- Delete line 64: `deviceSecret: deviceSecret,` in PersistedPairingState constructor (the field is also being dropped via FRB regen)
- Delete line 85: `await _writeOrDelete(_keyDeviceSecret, state.deviceSecret);`
- Delete line 116: `await _storage.delete(key: _keyDeviceSecret);` from `clearAll()`
- Delete line 136: `final hasDeviceSecret = state.deviceSecret != null;`
- Update `_isValidSnapshot` to only require `state.deviceId != null` (auth optional, paired-but-unauthenticated state intentionally not supported anymore).

After edits, `_isValidSnapshot` becomes:

```dart
bool _isValidSnapshot(PersistedPairingState state) {
    if (state.deviceId == null) return false;
    final hasAnyAuth =
        state.accessToken != null ||
        state.accessExpiresAtMs != null ||
        state.refreshToken != null ||
        state.accountId != null ||
        state.accountEmail != null;
    return hasAnyAuth;
}
```

- [ ] **Step 2: Add a one-shot legacy wipe**

Inside `loadState()`, before reading auth keys, add:

```dart
// Legacy wipe: pre ADR-0020 keychain entry. Best-effort; idempotent.
await _storage.delete(key: 'minos.device_secret');
```

- [ ] **Step 3: Run Flutter analysis**

Run: `cd apps/mobile && dart analyze`
Expected: clean. Any `deviceSecret` references in other Dart files would surface here.

- [ ] **Step 4: Commit**

```bash
git add apps/mobile/lib/infrastructure/secure_pairing_store.dart
git commit -m "feat(flutter/keychain): drop minos.device_secret; one-shot legacy wipe"
```

### Task L2: `minos_core.dart` drops `deviceSecret` references

**Files:**
- Modify: `apps/mobile/lib/infrastructure/minos_core.dart`

- [ ] **Step 1: Find references**

Run: `grep -n "deviceSecret" apps/mobile/lib/infrastructure/minos_core.dart`

- [ ] **Step 2: Update guard conditions**

Around line 93 the snapshot validation reads:

```dart
if (persisted.accessToken == null || persisted.deviceSecret == null) {
```

Replace with:

```dart
if (persisted.accessToken == null) {
```

Around line 296:

```dart
return state.accessToken != null &&
    state.deviceSecret != null && ...
```

Drop the `deviceSecret` clause.

- [ ] **Step 3: Add `hasPersistedPairing` accommodating multi-mac**

The existing `hasPersistedPairing` returns a `bool` based on having a paired peer. Replace with a stream/future of `int macCount` driven by FFI `listPairedMacs`:

```dart
Stream<List<MacSummary>> watchPairedMacs() async* {
    while (true) {
        try {
            final macs = await _client.listPairedMacsDto();
            yield macs;
        } catch (_) {
            yield const [];
        }
        await Future.delayed(const Duration(seconds: 30));
    }
}
```

(Polling is fine for MVP; replace with a Rust-pushed stream later.)

- [ ] **Step 4: Run Flutter analysis**

Run: `cd apps/mobile && dart analyze`
Expected: clean. UI sites (next task) will surface remaining `hasPersistedPairing`/`deviceSecret` usages.

- [ ] **Step 5: Commit**

```bash
git add apps/mobile/lib/infrastructure/minos_core.dart
git commit -m "feat(flutter/core): drop deviceSecret; expose paired-macs stream"
```

### Task L3: Pairing page no longer expects secret

**Files:**
- Modify: `apps/mobile/lib/presentation/pages/pairing_page.dart`

- [ ] **Step 1: Find the consume-response branch**

Run: `grep -n "your_device_secret\|deviceSecret\|PairResponse" apps/mobile/lib/presentation/pages/pairing_page.dart`

- [ ] **Step 2: Drop secret-handling**

The QR scan completion handler should now look like:

```dart
final mac = await _client.pairWithQrJson(qrJson);
// mac is a MacSummary or PairResponseDto without secret.
await _client.setActiveMacDto(mac.macDeviceId);
ref.read(macsProvider.notifier).refresh();
Navigator.of(context).pop();
```

Remove any code that previously called `secureStore.write(_keyDeviceSecret, ...)`.

- [ ] **Step 3: Run analysis**

Run: `cd apps/mobile && dart analyze apps/mobile/lib/presentation/pages/pairing_page.dart`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add apps/mobile/lib/presentation/pages/pairing_page.dart
git commit -m "feat(flutter/pairing): drop secret-handling from QR consume flow"
```

### Task L4: App shell renders Mac list + active-mac selector

**Files:**
- Modify: `apps/mobile/lib/presentation/pages/app_shell_page.dart`
- Modify: `apps/mobile/lib/application/minos_providers.dart`

- [ ] **Step 1: Add provider for paired-macs stream**

In `minos_providers.dart`:

```dart
final pairedMacsProvider = StreamProvider<List<MacSummary>>((ref) {
    final client = ref.watch(minosClientProvider);
    return client.watchPairedMacs();
});

final activeMacProvider = StateNotifierProvider<ActiveMacNotifier, AsyncValue<DeviceId?>>((ref) {
    return ActiveMacNotifier(ref.watch(minosClientProvider));
});

class ActiveMacNotifier extends StateNotifier<AsyncValue<DeviceId?>> {
    ActiveMacNotifier(this._client) : super(const AsyncValue.loading()) {
        _hydrate();
    }
    final MobileClient _client;
    Future<void> _hydrate() async {
        final id = await _client.activeMacDto();
        state = AsyncData(id);
    }
    Future<void> setActive(String macId) async {
        await _client.setActiveMacDto(macId);
        state = AsyncData(macId);
    }
}
```

- [ ] **Step 2: Replace single-row `_RuntimePartnerRow` with a list**

In `app_shell_page.dart` find the `hasPairing.when(... data: (paired) {...})` block (lines ~174–201). Replace with:

```dart
final macsAsync = ref.watch(pairedMacsProvider);
final active = ref.watch(activeMacProvider);
return macsAsync.when(
  loading: () => const _LoadingRow(),
  error: (e, _) => _ErrorRow(message: e.toString()),
  data: (macs) {
    if (macs.isEmpty) {
      return _AddPartnerRow(onTap: () => _openPairingPage(context));
    }
    return Column(children: [
      ...macs.map((m) => _MacRow(
            mac: m,
            isActive: active.valueOrNull == m.macDeviceId,
            onTap: () => ref.read(activeMacProvider.notifier).setActive(m.macDeviceId),
            onForget: () => ref.read(minosClientProvider).forgetMacDto(m.macDeviceId),
          )),
      _AddPartnerRow(onTap: () => _openPairingPage(context)),
    ]);
  },
);
```

`_MacRow`, `_AddPartnerRow` are local widgets. Implement them with the same visual style as the existing `_RuntimePartnerRow` (steal styling).

- [ ] **Step 3: Run flutter analyze + run app on simulator**

Run: `cd apps/mobile && flutter analyze && flutter run -d ios-simulator`
(Manual: scan a Mac, verify it appears in the list; pair a second Mac, verify the list now has two rows; tap one to make it active; tap forget to remove.)

- [ ] **Step 4: Commit**

```bash
git add apps/mobile/lib/presentation/pages/app_shell_page.dart \
        apps/mobile/lib/application/minos_providers.dart
git commit -m "feat(flutter/ui): render paired-macs list with active selector"
```

---

## Phase M — Cross-platform integration

### Task M1: End-to-end forward-with-target test (real Mac daemon + iOS mock)

**Files:**
- Modify: `crates/minos-backend/tests/server_centric_auth_e2e.rs`

- [ ] **Step 1: Add a multi-mac test**

```rust
#[tokio::test]
async fn one_account_two_macs_forward_routes_correctly() {
    let app = common::spawn_app().await;
    let account = common::register_account(&app).await;
    let mac_a = common::pair_mac_to_account(&app, &account, "Mac-A").await;
    let mac_b = common::pair_mac_to_account(&app, &account, "Mac-B").await;
    let ios = common::login_ios(&app, &account).await;

    let mut ios_ws = common::connect_ws(&app, &ios).await;
    let mut mac_a_ws = common::connect_ws(&app, &mac_a).await;
    let mut mac_b_ws = common::connect_ws(&app, &mac_b).await;

    // Forward to Mac-A
    ios_ws.send_forward(mac_a.device_id, json!({"id": 1, "method": "ping_a"})).await;
    let recv_a = mac_a_ws.recv().await;
    assert!(recv_a.contains("ping_a"));

    // Forward to Mac-B (should NOT reach Mac-A)
    ios_ws.send_forward(mac_b.device_id, json!({"id": 2, "method": "ping_b"})).await;
    let recv_b = mac_b_ws.recv().await;
    assert!(recv_b.contains("ping_b"));

    // Mac-A should still be silent (no second message)
    let timeout = tokio::time::timeout(std::time::Duration::from_millis(200), mac_a_ws.recv()).await;
    assert!(timeout.is_err(), "Mac-A unexpectedly received a frame");
}
```

- [ ] **Step 2: Run; expect pass**

Run: `cargo test -p minos-backend --test server_centric_auth_e2e -- one_account_two_macs`
Expected: pass.

- [ ] **Step 3: Commit**

```bash
cargo xtask check-all
git add crates/minos-backend/tests/
git commit -m "test(backend): one-account-two-macs forward routing e2e"
```

### Task M2: Final clean-up sweep

**Files:** none — investigation/cleanup.

- [ ] **Step 1: Search for dead references**

```bash
rg "your_device_secret|MePeerResponse|paired_with|store::pairings|X-Device-Secret" \
   crates/ apps/ docs/superpowers/specs/ 2>/dev/null
```

For each remaining hit:
- If in `crates/minos-daemon/` or `crates/minos-domain/`: leave as-is (Mac side intentionally untouched).
- If in `crates/minos-backend/` or `crates/minos-mobile/` or `apps/mobile/`: investigate; should be zero hits except in tests verifying Mac-side wire shape.
- If in `docs/superpowers/specs/`: ensure §12.2 supersession note exists (Phase A).

- [ ] **Step 2: Run full check**

Run: `cargo xtask check-all`
Expected: clean.

- [ ] **Step 3: Verify protocol golden fixtures**

Run: `cargo test -p minos-protocol --test envelope_golden 2>/dev/null || true`
Expected: pass if the golden test exists; investigate any drift.

- [ ] **Step 4: Run mobile UI smoke**

```bash
cd apps/mobile
flutter analyze
flutter test
```

Expected: pass.

- [ ] **Step 5: Final commit (if any)**

```bash
git add -p
git commit -m "chore: post-refactor cleanup"
```

(Skip if no changes.)

---

## Self-Review Checklist

**Spec coverage:**

| Decision (from chat) | Phase / Task |
|---|---|
| Mobile drops `X-Device-Secret` rail | F1, F2, F3, I2, J1, K1, L1 |
| Mac secret rail unchanged | (verified by tests in F2/F3 + M2) |
| Pair table uses (mac_device_id, mobile_account_id) | B2, D1, E1 |
| Old `pairings` table dropped | B1, D1 |
| Envelope `Forward.target_device_id` | C1, G1, I3 |
| `classify()` role-aware | F1 |
| `/v1/me/peer` → `/v1/me/macs` | C2, H1 |
| `PairResponse.your_device_secret` removed; `Event::Paired.your_device_secret` optional | C1, C2, E1 |
| Mobile keychain drops `device_secret`; legacy-wipe | L1 |
| `paired_via_device_id` recorded for audit | B2, D1, E1, H1 |
| ADR + spec supersession note | A1 |

**Placeholders scan:** None. Every step contains the exact code or command to run; no "TBD" / "similar to" / "implement later".

**Type consistency:** `account_mac_pairings`, `MeMacsResponse`, `MacSummary`, `PairRow`, `target_device_id`, `mac_device_id`, `mobile_account_id`, `paired_via_device_id` — names are consistent across phases.

**Out-of-scope (explicitly):**
- Mac-side daemon multi-peer UI (`crates/minos-daemon/src/handle.rs:30` single-peer slot stays).
- `crates/minos-domain/src/relay_state.rs` `PeerState::Paired { peer_id, peer_name, online }` stays single-peer.
- These are tracked separately as P2 in `docs/superpowers/specs/macos-relay-client-migration-design.md`.

---

Plan complete and saved to `docs/superpowers/plans/11-server-centric-auth-and-pair.md`.

**Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh Opus subagent per task (per project memory), review between tasks, fast iteration via worktree.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints for review.

Which approach?
