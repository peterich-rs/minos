# ADR 0013 · macOS Relay-Client Cutover Shape

Status: Refined by 0018 (entry-point and storage)
Date: 2026-04-24
Owner: fannnzhang
Related spec: `docs/superpowers/specs/macos-relay-client-migration-design.md`

## Context

`minos-relay` shipped on `main` as of commit `79bcbdf`. The Mac app has not yet migrated: it still binds a Tailscale-IP WebSocket server, discovers the Tailscale 100.x IP at boot, and exposes `discoverTailscaleIp` through UniFFI. This ADR records three coupled decisions made during brainstorming for the Mac-side migration:

1. Whether to keep any Tailscale code path on the Mac.
2. Where the relay backend URL is configured.
3. How to model connection state now that the relay emits separate signals for "link to backend" and "peer (iPhone) online/offline".

## Decision

### 1 · Full removal of Tailscale and WsServer on the Mac

The Mac app becomes a pure outbound WSS client of the relay. `crates/minos-daemon/src/tailscale.rs` is deleted; `discover_tailscale_ip` / `_with_reason` are deleted; `crates/minos-transport/src/server.rs` is deleted; `start_autobind` port-retry logic is removed; the doctor CLI's Tailscale line is removed. No feature flag, no runtime trait abstraction, no dual-mode switch.

### 2 · `MINOS_BACKEND_URL` baked at compile time

The relay URL is read via `option_env!("MINOS_BACKEND_URL")` in `crates/minos-daemon/src/config.rs`, with a local-development fallback of `ws://127.0.0.1:8787/devices`. Production releases inject the real URL through GitHub Actions from a repo secret (`secrets.MINOS_BACKEND_URL`). CI jobs for tests run with the fallback; no secret required for PRs from forks. The URL is **not** exposed in user-facing configuration UI.

### 3 · Two-axis state model

`minos-domain::ConnectionState` is replaced by two orthogonal enums:

```rust
pub enum RelayLinkState { Disconnected, Connecting { attempt: u32 }, Connected }
pub enum PeerState      { Unpaired, Pairing, Paired { peer_id, peer_name, online } }
```

UniFFI exports two separate callback traits (`RelayLinkStateObserver`, `PeerStateObserver`); Swift's `AppState` subscribes to both and composes the MenuBar label from the pair.

## Rationale (combined)

The three decisions are tightly coupled:

- Full Tailscale removal is only sensible if the relay path is the sole reliable connection, which presumes relay auto-connect.
- Relay auto-connect requires a known URL at launch; any user-facing URL configuration adds friction to the "auto" promise.
- Baking the URL at compile time plus CI secret injection delivers that "auto" without hardcoding a constant into source, and without adding a runtime config surface that duplicates deployment concerns.
- The relay's server-pushed event set (`Paired` / `PeerOnline` / `PeerOffline` / `Unpaired` / `ServerShutdown`) is naturally two-dimensional. Collapsing it into a single enum either loses information (reconnect flushes peer name) or creates a combinatorial variant explosion.

## Alternatives rejected

| Alternative | Rejected because |
|---|---|
| Compile-time feature flag `backend-tailscale` vs. `backend-relay` with two binary flavors | iOS cannot be shipped in two flavors to end users; the abstraction tax moves to iOS which must runtime-dispatch on QR schema anyway — total cost unchanged, distribution worse |
| Runtime trait `MacHostBackend` with `TailscaleHost` and `RelayClientHost` impls | Two auth models, two QR schemas, two trusted-device stores — carrying cost real, value speculative; `minos-relay` spec already committed to retirement |
| User-configured `backend_url` via onboarding | Deployment concern leaking into UI; contradicts "auto-connect" product promise; no real user-adjustable value for single-owner MVP |
| Single-enum state model with new variants (e.g., `Unpaired`, `PeerOnline{name}`, `PeerOffline{name}`) | Reconnect case loses peer info; variant combinatorics grow with additional relay event types later |
| Swift-side composition only (Rust keeps `RelayLinkState`, Swift derives peer) | Peer events must still flow — observer plumbing is unavoidable either way; two observers in Rust is the same cost, clearer intent |

## Consequences

**Positive:**
- Mac code path collapses to one (outbound WSS). Dead branches disappear, not just get `cfg`'d away.
- New UniFFI surface is a clean slate; Swift's `AppState` becomes more honest about what it's observing.
- Failure modes are easier to attribute: CF edge rejection (401 HTTP) vs. relay business auth rejection (4401 close) now map to distinct errors with distinct UI handling.
- Future multi-peer scenarios can extend `PeerState` without touching link state.

**Negative / cost:**
- iOS and Mac cannot pair during the gap between this migration and the iOS migration spec. A `fake-peer` dev bin covers smoke; real user-facing pairing waits for iOS to catch up.
- `minos-mobile` imports `QrPayload` / `TrustedDevice` from `minos-pairing`; the struct shape change may require minor Dart-side or Rust-shim adjustments to keep iOS compiling. Tracked as an implementation detail, not an architectural ADR concern.
- The `security-framework` crate becomes a new `minos-daemon` dependency (macOS-target-gated). Previous plans avoided native dependencies in `minos-daemon`; this trade accepts the dependency for clean Keychain access.

**Neutral:**
- ADR numbering collides with existing 0009/0010 parallel entries; this ADR takes 0013 to avoid adding to the collision range.
