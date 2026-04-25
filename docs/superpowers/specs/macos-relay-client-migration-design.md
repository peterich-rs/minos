# Minos · macOS App Relay-Client Migration — Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-24 |
| Owner | fannnzhang |
| Scope | Mac-only (iOS/Flutter migration is a separate future spec) |
| Branch | `feat/macos-relay-migration` |
| Worktree | `../minos-worktrees/macos-relay-migration/` |
| Supersedes (partial) | `macos-app-and-uniffi-design.md` §2.1 item 1, §4 (Tailscale arch branch), §5.1 (Phase 0 bind/discover behavior), §6.1 (boot via Tailscale), §7.4 rows 1–2 |
| Depends on | `minos-relay-backend-design.md` (broker already landed) |
| Related ADRs | 0009–0012 retained; proposes 0013 (see §13) |

---

## 1. Context

The `minos-relay` broker shipped on `main` as of commit `79bcbdf` (PR #1). Backend, persistence, envelope protocol, pairing service, and integration tests all land. What does **not** land in that PR is the Mac-side migration: `minos-daemon` still binds a Tailscale-IP WebSocket server, `DaemonHandle::start_autobind` still calls `discover_tailscale_ip_with_reason`, the UniFFI surface still exports `discoverTailscaleIp`, and `apps/macos/Minos.app` still boots the old P2P flow.

This spec closes that gap for the Mac side: the app becomes a pure outbound WSS client of the relay, Tailscale code is removed entirely, onboarding gains a CF Service Token sheet, and the connection state model splits into two axes (relay link × peer state) that match the relay's server-pushed events.

iOS / Flutter stays on Tailscale until a future dedicated spec migrates it. During the gap the Mac and iOS cannot pair across the two architectures — that is an accepted asymmetry for this phase; the gap is covered by a Rust-bin `fake-peer` dev tool (§5) that exercises the pairing and forwarding paths without needing the iOS app.

---

## 2. Goals

### 2.1 In scope

1. **Auto-connect boot.** On launch, Mac app reads CF Service Token (Keychain or env), reads self device id and peer from a local JSON, reads optional device secret from Keychain, and dials `${MINOS_BACKEND_URL}/devices` with appropriate auth headers. No user click required beyond first-run onboarding.
2. **Onboarding sheet.** First launch without CF credentials presents `OnboardingSheet` with two text fields (CF Client ID / CF Client Secret) and a Save button. Saving writes to Keychain, dismisses the sheet, and triggers bootstrap.
3. **Settings sheet.** MenuBar menu exposes "Relay 设置…" which opens `SettingsSheet` (same layout as OnboardingSheet, with Cancel). Saving overwrites Keychain, stops the current relay client task, and starts a new one with the fresh credentials.
4. **QR schema change.** The pairing QR encodes `{v: 1, backend_url, token, mac_display_name}` — no IP, no port. Token is issued by the relay via `LocalRpc{method: RequestPairingToken}`, 5-minute TTL.
5. **Two-axis state model.** `minos-domain` replaces `ConnectionState` with `RelayLinkState` (relay WS up/down) and `PeerState` (unpaired / pairing / paired-online / paired-offline). UniFFI exports two callback traits: `RelayLinkStateObserver` and `PeerStateObserver`.
6. **Full Tailscale removal.** `crates/minos-daemon/src/tailscale.rs`, `discover_tailscale_ip` / `_with_reason`, `minos-transport::server`, port-retry in `start_autobind`, and the doctor CLI's Tailscale line are all deleted.
7. **Persistence split.** CF Client ID/Secret and issued `DeviceSecret` live in Keychain (service = `ai.minos.macos`); self device id and peer record live in `~/Library/Application Support/Minos/local-state.json`.
8. **Forget behavior.** Forget requires the relay link to be connected (menu item disabled and tooltipped otherwise). Forget issues `LocalRpc{method: ForgetPeer}`, waits for the relay's `Unpaired` event, then clears Keychain device_secret and sets `peer = null` in the JSON file.
9. **Fake-peer dev bin.** `crates/minos-mobile/src/bin/fake-peer.rs` provides a small CLI to simulate an `ios-client` pairing and forwarding session for end-to-end smoke on the migration branch.
10. **`cargo xtask check-all` green** on a fresh clone of the worktree, after Swift XCTests are updated for the new state model.

### 2.2 Non-goals (explicit deferrals)

| Item | Deferred to |
|---|---|
| iOS / Flutter relay migration | Separate future spec |
| "Test Connection" button during onboarding | P1.5 |
| CF Service Token auto-rotation reminder | P1.5 |
| Relay admin console on `/admin` | Reserved in relay spec §12 |
| Offline forget with local nuclear wipe or queue-then-sync | P1 if ever |
| E2EE of forward payloads | Relay spec §12 (P2) |
| Backup / migration of old `devices.json` from Tailscale days | Not done — we assume no user has production-grade state on plan 02; worktree cut is a clean start |
| Code signing, notarization, DMG, Sparkle | Existing P1.5 release pipeline spec |
| Release CI job that injects `MINOS_BACKEND_URL` from GitHub Secrets to produce a signed `.app` | Same P1.5 spec; this spec only reserves the plumbing |
| LaunchAgent autostart | P1.5 |

### 2.3 Success criteria (end-of-plan smoke, run manually by the implementer)

```
□ Fresh Keychain + no env vars → launch Minos.app → OnboardingSheet appears
□ Bad CF creds → status shows "Cloudflare Access 认证失败", "Relay 设置…" highlighted
□ Valid CF creds + running local relay (via `cargo run -p minos-relay`) →
    MenuBar shows "已连接后端 · 未配对"
□ Click "显示配对二维码" → QR renders; payload contains new schema
□ `cargo run -p minos-mobile --bin fake-peer -- --backend ws://127.0.0.1:8787/devices \
      --token <extracted-from-QR>` → Mac app shows "已连接后端 · 手机在线"
□ Keychain Access.app → entry at service=ai.minos.macos account=device-secret exists
□ Ctrl-C fake-peer → relay pushes PeerOffline → Mac shows "已连接后端 · 手机离线"
□ Restart Mac app → auto-reconnects to "已连接后端 · 手机离线" without user action
□ Stop relay → "正在重连后端 · 手机: fake-peer (离线)" loop
□ Restart relay → auto-recovers
□ Forget → relay mediates → Keychain device-secret gone, JSON peer=null, UI "未配对"
□ grep -r "tailscale\|discover_tailscale_ip\|WsServer" crates apps — no production hits
□ cargo xtask check-all green
```

---

## 3. Tech Stack (deltas only)

The workspace inherits everything from `minos-architecture-and-mvp-design.md` §3 and `minos-relay-backend-design.md` §3. This spec adds:

| Concern | Choice | Note |
|---|---|---|
| Compile-time backend URL | `option_env!("MINOS_BACKEND_URL")` in `minos-daemon::config` | Fallback `ws://127.0.0.1:8787/devices`. CI jobs for tests run with the fallback; a future release job injects from `secrets.MINOS_BACKEND_URL` |
| Keychain access (Rust side) | `security-framework` crate | Used by `KeychainTrustedDeviceStore` (production and tests). Writes `device-secret` to service `ai.minos.macos` directly — no UniFFI callback bounce |
| Keychain access (Swift side) | `Security.framework` (`SecItemAdd` / `SecItemCopyMatching`) | Writes CF Client ID / Secret to the same service. No third-party wrapper |
| Fake-peer CLI parsing | `clap` (already a workspace dep) | 2 flags: `--backend`, `--token`; optional `--device-name` |

---

## 4. Architecture Overview

```
┌──────────────── apps/macos/Minos.app (single process) ────────────────┐
│ Swift / SwiftUI, no Dock icon, LSUIElement=true                       │
│                                                                       │
│  Presentation                                                         │
│    ├─ MenuBarView           3 layouts: awaiting-config / unpaired /   │
│    │                                   paired; Settings menu item     │
│    ├─ QRSheet               new-schema QR + 5min countdown            │
│    ├─ OnboardingSheet       NEW: 2 TextField + Save (no Cancel)       │
│    ├─ SettingsSheet         NEW: same layout + Cancel                 │
│    └─ StatusIcon            SF Symbol × (RelayLinkState, PeerState)   │
│                                                                       │
│  Application                                                          │
│    ├─ AppState @Observable  phase, relayLink, peer, currentQr,        │
│    │                        onboardingVisible, displayError,          │
│    │                        bootError, trustedDevice (Option)         │
│    ├─ RelayLinkObserver     RelayLinkStateObserver → @MainActor       │
│    └─ PeerObserver          PeerStateObserver       → @MainActor      │
│                                                                       │
│  Domain                                                               │
│    ├─ RelayLinkState+Display   .displayLabel / .iconName / .tint      │
│    ├─ PeerState+Display        .displayLabel / .peerName              │
│    └─ MinosError+Display       extended for new variants              │
│                                                                       │
│  Infrastructure                                                       │
│    ├─ DaemonBootstrap          Keychain/env read → start or onboarding │
│    ├─ KeychainRelayConfig      read/write CF pair (generic password)  │
│    ├─ QRCodeRenderer           unchanged (CIFilter)                   │
│    └─ DiagnosticsReveal        unchanged (today-log Finder reveal)    │
│                                                                       │
│                           UniFFI async + 2 callback traits            │
│  ┌───────────────────────────────▼───────────────────────────────┐   │
│  │ libminos_ffi_uniffi.a  (universal arm64 + x86_64 staticlib)   │   │
│  │   re-exports → DaemonHandle (new surface)                     │   │
│  │                 RelayLinkStateObserver (trait)                │   │
│  │                 PeerStateObserver      (trait)                │   │
│  │                 DeviceSecret           (custom_newtype)       │   │
│  └───────────────────────────────┬───────────────────────────────┘   │
│                                  │                                    │
│  minos-daemon (tokio, in-process)                                     │
│    ├─ Composition root                                                │
│    ├─ Relay-client task (WsClient → envelope dispatcher)              │
│    ├─ jsonrpsee server impls (list_clis, etc.) via Forwarded payload  │
│    └─ mars-xlog writer → ~/Library/Logs/Minos/daemon_*.xlog           │
│                                                                       │
│  minos-transport::WsClient + AuthHeaders                              │
│  minos-daemon::KeychainTrustedDeviceStore                             │
│                                                                       │
│  Outbound WSS → ${MINOS_BACKEND_URL}/devices                          │
│    (fallback ws://127.0.0.1:8787/devices for local dev / tests)       │
└───────────────────────────────────────────────────────────────────────┘
```

### 4.1 Process model

Unchanged from plan 02: single macOS process, SwiftUI scene and tokio runtime coexist via UniFFI. What changes is **what** the tokio runtime drives — no longer a WS server accept loop, now a WS client connect loop that dispatches envelope frames.

### 4.2 Protocol stack

```
Cloudflare edge (Access + TLS)
  → HTTP/2 or QUIC tunnel
  → cloudflared
  → HTTP/1.1 Upgrade on 127.0.0.1:8787
  → WebSocket
  → Envelope JSON frames (minos-protocol::envelope)
  → backend-local RPC (request_pairing_token / forget_peer)
    OR forwarded peer-to-peer JSON-RPC 2.0 (list_clis, future agent RPCs)
```

### 4.3 State model

Two independent axes, two watch channels, two UniFFI callback traits:

```rust
// minos-domain/src/relay_state.rs (new file)
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum RelayLinkState {
    Disconnected,
    Connecting { attempt: u32 },
    Connected,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum PeerState {
    Unpaired,
    Pairing,
    Paired { peer_id: DeviceId, peer_name: String, online: bool },
}
```

These are orthogonal: reconnecting to relay does not flip peer state (the "peer was paired; relay blip" situation is a legitimate UI state and the two-axis model preserves it). `ConnectionState` in `minos-domain` is deleted. `PairingState` (legacy enum used by `minos-mobile`'s Tailscale path) is untouched since iOS is not migrating in this spec.

---

## 5. Components

### 5.1 Per-crate change table

| Crate | Additions | Deletions | Modifications |
|---|---|---|---|
| `minos-domain` | `RelayLinkState`, `PeerState`, `DeviceSecret` newtype, `DeviceRole` (verify already present from relay spec); `MinosError` variants `Unauthorized` / `ConnectionStateMismatch` / `EnvelopeVersionUnsupported` / `PeerOffline` / `RelayInternal` / `CfAuthFailed` | `ConnectionState` enum and its display impls; `MinosError::BindFailed`'s Mac-side semantics (variant retained for relay crate; zh/en strings rewritten) | `ErrorKind` discriminant set extended; `ConnectFailed` zh/en strings rewritten to say "relay backend URL" |
| `minos-pairing` | new `QrPayload { v: 1, backend_url, token, mac_display_name }` | `Pairing` state machine module; `ActiveToken`; `generate_qr_payload` free function | `PairingStore` trait signature unchanged; `TrustedDevice` struct drops `tailscale_ip` field |
| `minos-transport` | `AuthHeaders` struct; `WsClient::connect(url, auth)` new signature | `server.rs` (entire file) and all its tests | `client.rs` header injection; 401 → `CfAuthFailed`, 4401 → `DeviceNotTrusted`, 4400 → `EnvelopeVersionUnsupported` |
| `minos-daemon` | `KeychainTrustedDeviceStore` in `src/keychain_store.rs` (replaces `FilePairingStore`); envelope dispatcher loop; `request_pairing_token` and `forget_peer` LocalRpc senders with response correlation; `RelayLinkStateObserver` + `PeerStateObserver` trait definitions (host side of UniFFI callback); `subscribe_relay_link` + `subscribe_peer` methods; `Subscription` reused; `config::BACKEND_URL` constant | `tailscale.rs` (entire file); `file_store.rs` (entire file); `discover_tailscale_ip` and `_with_reason` free fns; `host()` / `port()` / `addr()` accessors; `start_autobind` port-retry loop; `rpc_server.rs` server-binding code (jsonrpsee **server trait impls** for `list_clis` etc. move into the relay-client task that dispatches `Forwarded` payloads) | `start_autobind` renamed to `start(RelayConfig, self_device_id, peer, secret, mac_name)`; `events_stream` split into two watch receivers (kept `pub(crate)`); `current_trusted_device()` returns new shape; `stop(&self)` idempotency unchanged |
| `minos-ffi-uniffi` | `RelayLinkStateObserver`, `PeerStateObserver` exports; `DeviceSecret` `custom_newtype!` | `discover_tailscale_ip` free function export; `ConnectionStateObserver` export | Re-export list aligned with new daemon surface |
| `minos-mobile` | `src/bin/fake-peer.rs` dev binary (~100 lines: clap args, raw `WsClient` + envelope send/recv, pair + optional forget) | — | `Cargo.toml` gains `[[bin]]` target + `clap` optional dep scoped to bin |
| `apps/macos` | `OnboardingSheet.swift`, `SettingsSheet.swift`, `KeychainRelayConfig.swift`, `RelayLinkObserver.swift`, `PeerObserver.swift`, `RelayLinkState+Display.swift`, `PeerState+Display.swift` | `ObserverAdapter.swift`, `ConnectionState+Display.swift` | `AppState`, `DaemonDriving`, `DaemonBootstrap`, `DaemonHandle+DaemonDriving`, `MenuBarView`, `StatusIcon`, `QRSheet`, `MinosError+Display`, `AppStateTests`, `MockDaemon` all updated |

### 5.2 `DaemonHandle` Phase 0 surgery (6 atomic steps)

Each step is an independent commit; `cargo xtask check-all` must pass at every step. Order matters:

1. **Introduce new domain types.** Add `relay_state.rs` (`RelayLinkState`, `PeerState`) and `DeviceSecret` to `minos-domain`. Add the new `MinosError` variants and extend `ErrorKind`. `ConnectionState` kept for now — `minos-mobile` still uses it. Tests: serde golden + `ErrorKind::user_message` zh/en coverage.
2. **Shrink `DaemonInner`.** Replace `WsServer` field with `None` placeholder; `start_autobind` temporarily returns `unimplemented!()`; add two `watch::Sender` (for link, peer). This commit **cannot** keep the old bind flow working — it is the breaking checkpoint. Tests that exercised the old server path are migrated out or deleted in the same commit.
3. **Write envelope codec + dispatcher skeleton.** No WS wiring yet — unit tests only, covering `Envelope` round-trip, `EventKind` dispatch to the right sender, `LocalRpcResponse` correlation map.
4. **Wire `WsClient::connect(auth)`.** `minos-transport` gains `AuthHeaders`; `client.rs` does the upgrade with headers; `minos-daemon::start` spawns a task that does connect + dispatch + exponential reconnect. End-to-end test: run against an in-process `axum` echo server fixture (subset of relay's handshake).
5. **Implement `request_pairing_token` + `forget_peer`.** Mac-side LocalRpc senders; wraps response into `QrPayload` and `()` respectively. Tests use the full in-process relay from `minos-relay`'s integration test setup (the relay crate already has this scaffolding).
6. **Delete Tailscale.** Remove `tailscale.rs`, `discover_tailscale_ip*`, doctor CLI's tailscale branch, and any remaining `WsServer` / `start_autobind`-era dead code. `grep -r tailscale crates/ apps/` must yield zero hits outside commit history.

After step 6, the daemon crate's surface is final; UniFFI binding regeneration (step 7, not counted above) produces new Swift sources; Swift layer is updated (plan executes that as a separate sub-phase).

### 5.3 Sharing matrix (post-migration)

| Crate | Relay bin | Mac binary | iOS binary |
|---|---|---|---|
| `minos-domain` | ✓ | ✓ | ✓ |
| `minos-protocol` | ✓ (envelope) | ✓ | ✓ |
| `minos-pairing` | — | ✓ (new QrPayload schema; trait + new TrustedDevice shape) | ✓ (Dart-backed PairingStore impl; still produces/consumes old QR schema until iOS migrates) |
| `minos-transport` | — | ✓ (client only) | ✓ (client only) |
| `minos-cli-detect` | — | ✓ | — |
| `minos-daemon` | — | ✓ | — |
| `minos-mobile` | — | — (but dev `fake-peer` bin runs from Mac dev box) | ✓ |
| `minos-relay` | ✓ | — | — |
| `minos-ffi-uniffi` | — | ✓ | — |
| `minos-ffi-frb` | — | — | ✓ |

Asymmetry on `minos-pairing` across platforms:
- Mac: uses the new `QrPayload` schema, new `TrustedDevice` (no `tailscale_ip`), and `minos-daemon::KeychainTrustedDeviceStore` as the `PairingStore` impl.
- iOS: keeps its Dart-backed `PairingStore` impl (invoked through frb callback) and still produces/consumes the old QR schema until the iOS migration spec lands.

These are parallel code paths in the same crate — the `QrPayload` struct change is breaking, but iOS's Dart layer handles its own serialization for the legacy shape, so the Rust crate can ship the new `QrPayload` without iOS runtime breakage (iOS just won't recognize QRs emitted by the new Mac app and vice versa, which is the expected asymmetry until iOS migrates).

The `Pairing` state machine module is deleted because it was Mac-owned (token issuance moves to relay) — iOS never hosted that logic.

---

## 6. Data Flow

### 6.1 First launch (no Keychain CF creds, no env)

```
MinosApp.init()
 └─ Task { await DaemonBootstrap.bootstrap(appState) }

DaemonBootstrap.bootstrap(appState):
 1. try? initLogging()
 2. let cf =
      env CF_ACCESS_CLIENT_ID + CF_ACCESS_CLIENT_SECRET
      ?? KeychainRelayConfig.read()
    if cf == nil:
      appState.phase = .awaitingConfig
      appState.onboardingVisible = true
      return                              // halts bootstrap; sheet drives rest
 3. local-state.json:
      missing → write { self_device_id: UUIDv4(), peer: null }
      present → load (may throw StoreCorrupt)
 4. secret = Keychain.read("device-secret")          // may be nil
 5. daemon = try await DaemonHandle.start(
       RelayConfig { backend_url: BACKEND_URL, cf },
       self_device_id, peer, secret,
       mac_name: hostName()
    )
 6. subRelay = daemon.subscribe_relay_link(RelayLinkObserver { appState })
    subPeer  = daemon.subscribe_peer(PeerObserver { appState })
 7. await MainActor.run { appState.daemon = daemon; ... }

OnboardingSheet Save button handler:
 1. KeychainRelayConfig.write(clientId, clientSecret)
 2. appState.onboardingVisible = false
 3. Task { await DaemonBootstrap.bootstrap(appState) }   // retry from step 1
```

### 6.2 Already-onboarded + already-paired launch

```
Bootstrap (steps 1–7 above):
 - step 2 finds CF creds in Keychain
 - step 3 reads local-state.json with peer: Some
 - step 4 reads device_secret: Some
 - step 5 starts daemon:
    - initial RelayLinkState::Connecting{0}
    - initial PeerState::Paired{peer_id, peer_name, online: false}
    - relay-client task does WS connect with AuthHeaders including device_secret
 - step 5's relay task:
    - HTTP 101 → RelayLinkState::Connected
    - relay verifies argon2(secret), session = PAIRED
    - relay sends Event{type: PeerOnline | PeerOffline}
    - Mac updates PeerState::Paired{..., online: true|false}

UI: MenuBar header reflects composed label:
    "已连接后端 · 手机在线 · fannnzhang's iPhone"
 or "已连接后端 · 手机离线 · fannnzhang's iPhone"
```

### 6.3 Pairing (Mac requests; fake-peer consumes)

```
[Mac app]                          [Relay]                         [fake-peer (dev bin)]

1. user clicks "显示配对二维码"
   └─ AppState.showQr()
      └─ try await daemon.requestPairingToken()
         └─ LocalRpc{method: RequestPairingToken}  ────►
                                     2. issue token, TTL=5m
                                     ◄── LocalRpcResponse{result:{token, expires_at}}
   └─ qr = QrPayload{v:1, backend_url, token, mac_display_name}
   └─ appState.currentQr = qr; appState.isQrSheetPresented = true
   └─ PeerState transitions to Pairing

3. QRSheet renders; TimelineView countdowns 5min

                                                        4. `cargo run --bin fake-peer \
                                                              --backend ws://127.0.0.1:8787/devices \
                                                              --token <TOKEN>`
                                                           ├─ WS connect role=ios-client unpaired
                                                           └─ LocalRpc{method: Pair,
                                                               params:{token, device_name}}
                                                           ─────►
                                     5. consume token, gen
                                        DeviceSecret_mac + _phone,
                                        argon2 hash, update DB,
                                        push Event{type:Paired,...}
                                        to both sessions
   ◄── Event{type:Paired,          ────►  LocalRpcResponse{result:{peer_id, your_device_secret}}
        peer_device_id, peer_name,
        your_device_secret}
6. daemon on Paired:
   ├─ Keychain.write("device-secret", your_device_secret)
   ├─ local-state.json.peer = {device_id, name, paired_at: now}
   └─ state_tx_peer.send(Paired{..., online: true})

7. appState.peer_state → Paired; isQrSheetPresented = false
   MenuBar: "已连接后端 · 手机在线 · fake-peer"
```

### 6.4 Reconnect

```
WS drops (network blip, relay restart, SIGPIPE):
 1. WsClient loop catches, sends state_tx_relay.send(Connecting{attempt+1})
 2. backoff wait (1s → 2s → 4s → … → 30s cap)
 3. retry connect with same AuthHeaders (including device_secret)
 4. on success: state_tx_relay.send(Connected)
    - relay re-validates argon2(secret), session = PAIRED (assuming still paired)
    - relay pushes Event{type: PeerOnline | PeerOffline}
    - state_tx_peer updates accordingly
 5. PeerState preserved throughout the Connecting phase — the paired peer_name
    stays visible in the UI ("正在重连 · 手机: ..."). Only PeerOnline/Offline
    toggles `online` flag.
```

### 6.5 CF auth failure

```
WS connect → HTTP 401 from CF edge (relay never sees the request)
 1. WsClient surfaces MinosError::CfAuthFailed{message}
 2. state_tx_relay.send(Disconnected)      // no backoff retry — creds are wrong
 3. daemon stores last error on handle so AppState can read it
 4. Observer callback gives Swift a Disconnected transition;
    Swift cross-references daemon.lastError() → sees CfAuthFailed
    → appState.displayError = .cfAuthFailed
    → MenuBar icon tint red, subtext "Cloudflare Access 认证失败"
    → Settings 菜单 item highlighted
 5. User opens SettingsSheet, corrects creds, saves → see §6.6
```

### 6.6 Settings update (token rotation)

```
User clicks "Relay 设置…"
 └─ SettingsSheet opens; pre-populated with current Keychain values (client_id
    visible, client_secret shown as •••• — edit requires re-enter)
 └─ user edits, Save
    ├─ KeychainRelayConfig.write(new values)
    ├─ await daemon.stop()
    ├─ subRelay.cancel(); subPeer.cancel()
    └─ Task { await DaemonBootstrap.bootstrap(appState) }
        // bootstrap 从步骤 2 重新读 Keychain，新 creds 生效

If new creds are valid → Connected within seconds.
If still invalid → §6.5 loop.
```

### 6.7 Forget peer

```
Precondition: relay_link == .Connected AND peer != .Unpaired.
(If not, the Forget menu item is disabled; forget button in UI is gone.)

1. user clicks "忘记已配对设备"
 2. NSAlert confirm ("Forget {peer_name}? 你需要重新扫码才能再次配对。")
    [取消] [忘记]
 3. on confirm:
    └─ try await daemon.forgetPeer()
       └─ LocalRpc{method: ForgetPeer} ──►  Relay DELETE pairings row,
                                            push Event{type:Unpaired} to both sides

 4. daemon on Unpaired event:
    ├─ Keychain.delete("device-secret")
    ├─ local-state.json.peer = null
    └─ state_tx_peer.send(Unpaired)

 5. appState.peer_state → Unpaired
    MenuBar: "已连接后端 · 未配对", "显示配对二维码…" 重新可见
```

### 6.8 Onboarding cancelled

If the user closes `OnboardingSheet` without saving (ESC, red close button, etc.):
```
 └─ appState.phase stays .awaitingConfig
 └─ MenuBar layout: red bolt icon, header "Minos · 等待配置"
    menu items: ["Relay 设置…", "退出 Minos"]
    QR / Forget / diagnostic reveal all hidden
```
User can click "Relay 设置…" at any time to reopen `OnboardingSheet` (or `SettingsSheet` — which one depends on phase, same view backing though).

---

## 7. Persistence

### 7.1 Split

| Data | Storage | Identifier | Cleared on |
|---|---|---|---|
| CF Client ID | Keychain generic password | service=`ai.minos.macos`, account=`cf-client-id` | SettingsSheet overwrite / uninstall |
| CF Client Secret | Keychain generic password | service=`ai.minos.macos`, account=`cf-client-secret` | same |
| Device Secret (from relay) | Keychain generic password | service=`ai.minos.macos`, account=`device-secret` | Forget / `Unpaired` event / 4401 close |
| Self Device ID | JSON file | `self_device_id` | first generation only; stable for app lifetime |
| Peer (id + name + paired_at) | JSON file | `peer` (nullable) | written on Pair; nulled on Forget / Unpaired |

JSON file path: `~/Library/Application Support/Minos/local-state.json`.

Schema:

```json
{
  "self_device_id": "550e8400-e29b-41d4-a716-446655440000",
  "peer": null
}
```

Or with peer:

```json
{
  "self_device_id": "550e8400-e29b-41d4-a716-446655440000",
  "peer": {
    "device_id": "660e8400-e29b-41d4-a716-446655440001",
    "name": "fannnzhang's iPhone",
    "paired_at": "2026-04-24T12:34:56Z"
  }
}
```

No `schema_version` field. If parse fails, the file is treated as corrupt and `MinosError::StoreCorrupt` surfaces — the user manually deletes the file. No automatic migration (see §2.2 deferral note).

### 7.2 CF creds precedence

On every `DaemonBootstrap.bootstrap` call:
1. If both env vars `CF_ACCESS_CLIENT_ID` and `CF_ACCESS_CLIENT_SECRET` are present → use env. Do not read Keychain. (Keychain values are left untouched — env is a dev override, not a wipe.)
2. Else → read Keychain. Both entries must be present; partial presence is treated as "no creds" (surface a single warning log, show onboarding).
3. Else → onboarding.

`SettingsSheet` always writes to Keychain. Under env override, a small footnote on the sheet reads: "当前有环境变量覆盖生效，本次保存的值在 unset 环境变量之前不会生效。"

### 7.3 Keychain access layering

Two independent write paths within the same process, both targeting service `ai.minos.macos`:

- **Rust path (`device-secret`)**: `crates/minos-daemon/src/keychain_store.rs` uses the `security-framework` crate directly to read/write/delete the `device-secret` account. Triggered from the relay-client task on `Paired` / `Unpaired` events. Same crate is used in Rust tests; macOS dev + CI environments both work.
- **Swift path (`cf-client-id` + `cf-client-secret`)**: `apps/macos/Minos/Infrastructure/KeychainRelayConfig.swift` uses `SecItem*` APIs (`Security.framework`) to read/write/delete the two CF credential accounts. Triggered by `OnboardingSheet` / `SettingsSheet` save buttons.

Swift shape:

```swift
enum KeychainRelayConfig {
    static let service = "ai.minos.macos"
    static func read() -> (clientId: String, clientSecret: String)? { ... }
    static func write(clientId: String, clientSecret: String) throws { ... }
    static func clear() throws { ... }
}
```

Swift does **not** call into Rust to write `device-secret`; Rust does **not** call into Swift to write CF creds. Both layers read the other's entries on demand via the same service name. All operations use `kSecClassGenericPassword`. No third-party SPM or crate dep beyond `security-framework` (already a viable Rust ecosystem dep; added to `minos-daemon/Cargo.toml` as `[target.'cfg(target_os = "macos")'.dependencies]`).

---

## 8. Error Handling

### 8.1 New `MinosError` variants

```rust
// minos-domain/src/error.rs — additions; order matches ErrorKind table
#[error("cloudflare access authentication failed: {message}")]
CfAuthFailed { message: String },

#[error("unauthorized for this operation: {reason}")]
Unauthorized { reason: String },

#[error("relay connection state mismatch: expected {expected}, got {actual}")]
ConnectionStateMismatch { expected: String, actual: String },

#[error("envelope version unsupported: {version}")]
EnvelopeVersionUnsupported { version: u8 },

#[error("peer offline: {peer_device_id}")]
PeerOffline { peer_device_id: String },

#[error("relay internal error: {message}")]
RelayInternal { message: String },
```

`ErrorKind` gets six new variants matching these. `ErrorKind::user_message` gains 12 new string entries (6 × zh/en). The existing `BindFailed` zh/en strings are rewritten for the relay's listen-address context (Mac no longer binds). `ConnectFailed` strings rewritten to reference "relay backend URL".

### 8.2 UI bucket routing

| Error | Swift bucket | MenuBar visual | zh 文案 |
|---|---|---|---|
| `CfAuthFailed` | `displayError` + icon red | subtext shown, "Relay 设置…" highlighted | "Cloudflare Access 认证失败，请检查 Service Token" |
| `DeviceNotTrusted` | — (auto-recovered) | transient red flash; `PeerState` returns to `Unpaired` | silently wipes secret; no banner |
| `Unauthorized` | `displayError` 3s banner | amber dot overlay | "操作被拒绝：{reason}" |
| `ConnectionStateMismatch` | `displayError` 3s banner | amber dot overlay | "当前状态无法执行该操作" |
| `EnvelopeVersionUnsupported` | `bootError` | red badge icon, all actions hidden except "退出" | "协议版本不兼容 (v{version})，请更新应用" |
| `PeerOffline` | `displayError` 3s banner | amber dot overlay | "手机离线，请检查对端状态" |
| `RelayInternal` | `displayError` 3s banner | amber dot overlay | "后端异常：{message}" |
| `ConnectFailed` | — | `RelayLinkState::Connecting{attempt}` reconnect loop | no banner; only reflected in status header |
| `StoreIo` / `StoreCorrupt` | `bootError` | red badge, show path + "在 Finder 中显示" | "本地存储异常：{path}" |

### 8.3 WS close codes

| Code | Client behavior |
|---|---|
| 1000 | Normal (Forget / Quit). Stop reconnect loop. |
| 1001 | `ServerShutdown`. Start reconnect with backoff. |
| 4400 | `EnvelopeVersionUnsupported`. `bootError`, no retry. |
| 4401 | `DeviceNotTrusted`. Wipe Keychain `device-secret`, set `peer = null` in JSON, immediately reconnect as UNPAIRED (without secret). This path is how the relay tells the Mac "I revoked your pair, you're starting over." |
| 4409 | `ConnectionStateMismatch`. Display banner, retry with backoff. |

### 8.4 WS auth failure vs. CF auth failure — disambiguation

Because two independent layers can reject:

- HTTP 401 at WS Upgrade response (before 101) → CF edge rejected → `CfAuthFailed`. Retry pointless until creds change.
- WS 101 accepted, then 4401 close code → relay rejected business auth → `DeviceNotTrusted`. Auto-recover as above.

`WsClient::connect` distinguishes by inspecting the upgrade response: a 401 status means `CfAuthFailed`; a 101 status followed by 4401 close means `DeviceNotTrusted`.

---

## 9. Testing Strategy

### 9.1 Rust matrix deltas

| Crate | Deleted | Added | Changed |
|---|---|---|---|
| `minos-daemon` | `start_autobind` port-retry, `discover_tailscale_ip*`, all `tailscale.rs` tests | Envelope dispatcher (LocalRpcResponse correlation, Forwarded → jsonrpsee server, Event → two state_tx); `start` + `stop` idempotency; 401 → `CfAuthFailed`; 4401 → auto-relogin as Unpaired; reconnect preserves PeerState | `subscribe_*` shape from one to two |
| `minos-transport` | All WsServer tests | `WsClient::connect(auth)` header injection; 401 / 4401 / 1001 / 4400 mapping; backoff sequence (mock time) | `auth.rs` header serialization |
| `minos-pairing` | `Pairing` state-machine exhaustive cases; illegal transitions; `generate_qr_payload` entropy property | `QrPayload` new-schema round-trip + golden JSON; `TrustedDevice` (no `tailscale_ip`) round-trip | — |
| `minos-daemon` (store layer) | `FilePairingStore` tests | `KeychainTrustedDeviceStore` unit tests using `security-framework` against a tempdir-backed keychain (macOS dev + CI swift job) | — |
| `minos-domain` | `ConnectionState` golden + tests | `RelayLinkState` / `PeerState` golden; `ErrorKind` coverage of new variants; zh/en user_message for all new strings | — |
| `minos-mobile` | — | `fake-peer` bin compiles (`cargo build -p minos-mobile --bin fake-peer`); a single `#[tokio::test]` exercising "connect fake-peer against in-process relay → Pair → receive secret → close" in <1s | — |
| `minos-relay` | — | — | **not in scope for this spec** |

### 9.2 Swift XCTests (`apps/macos/MinosTests/`)

Rewritten in line with the new state model. Scenarios:

| Scenario | Setup | Assertion |
|---|---|---|
| First launch no creds | `MockDaemon` not constructed; Keychain empty; env unset | `appState.phase == .awaitingConfig`, `onboardingVisible == true`, `canShowQr == false` |
| Onboarding save | Mock Keychain harness; user types creds; tap Save | Keychain stub shows both entries; bootstrap called again; `onboardingVisible == false` |
| Env override | Env has creds; Keychain empty | Bootstrap proceeds without sheet; footnote flag on Settings sheet set |
| RelayLinkObserver fires | `MockDaemon` invokes observer with `.connected` | `appState.relayLink == .connected`, MenuBar subtext updates |
| PeerObserver fires | Same, peer observer with `.paired{online:true}` | `appState.peer == .paired(...); online == true` |
| Reconnect preserves peer | relay_link `.connected → .connecting{1}`, peer unchanged | Peer info still shown in MenuBar subtext |
| CfAuthFailed UI | MockDaemon surfaces `.cfAuthFailed` | `displayError == .cfAuthFailed`, Settings menu flagged |
| Settings save triggers reconnect | MockDaemon records `stop` + new `start` calls | Both call-counts exactly 1 each in sequence |
| Forget disabled when disconnected | Inject `relay_link == .disconnected`, peer == Paired | `canForgetPeer == false`, alert never presented |
| Forget success | `relay_link == .connected`; user confirms NSAlert | MockDaemon records `forgetPeer`; observer then fires `.unpaired`; Keychain device-secret clear call recorded |
| StoreCorrupt at bootstrap | MockDaemon `start` throws StoreCorrupt | `bootError` set, MenuBar shows path + Reveal |

Deleted: all cases tied to `ConnectionState.pairing → connected` transitions (the Tailscale state shape), all cases exercising the port-retry branch, all cases checking `discoverTailscaleIp` presence.

### 9.3 End-to-end smoke (manual, per-branch)

Same as §2.3 success criteria. Run before merging to `main`.

### 9.4 CI updates

`.github/workflows/ci.yml`:

- **rust job** — unchanged invocation (`cargo xtask check-all`); no new secrets required; all new tests run with `option_env!` fallback to local WS URL.
- **swift job** — XCTests file-list expands (Xcodegen picks up via glob); no new workflow steps.
- **release job** — **NOT created in this spec**; only a single-line `TODO` comment in `ci.yml` noting that a future workflow will inject `secrets.MINOS_BACKEND_URL` to build a production `.app`.

### 9.5 `cargo xtask check-all` boundaries

No changes to `xtask` itself. Gate continues to be: `cargo fmt --check` → `cargo clippy --workspace -- -D warnings` → `cargo test --workspace` → `cargo xtask gen-uniffi` (re-gen check) → `cargo xtask build-macos` → `xcodebuild build` → `xcodebuild test` → `swiftlint --strict apps/macos`. Every Phase 0 surgery commit must pass this full gate per the project's relay-plan commit rule.

---

## 10. Tooling and Operations

### 10.1 Compile-time `MINOS_BACKEND_URL`

Central read in `crates/minos-daemon/src/config.rs`:

```rust
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => "ws://127.0.0.1:8787/devices",
};
```

Callers (envelope dispatcher, `DaemonHandle::start`) read from this one constant. Tests override by passing an explicit URL into a test-only constructor that accepts the value at runtime (so `option_env!` isn't in the test hot path).

### 10.2 GitHub Secrets

Secret to configure by the repo owner:

- `MINOS_BACKEND_URL` — e.g., `wss://minos.fan-nn.top/devices`

The CI `rust` and `swift` jobs **do not** read this secret — PRs from forks still run green. A future `release` job will pull it at build time; this spec only reserves the name.

### 10.3 Fake-peer bin

Usage:

```
cargo run -p minos-mobile --bin fake-peer -- \
    --backend ws://127.0.0.1:8787/devices \
    --token <PAIRING_TOKEN> \
    [--device-name "fake-peer"]
```

Behavior:
1. Generate a UUIDv4 as `device_id`.
2. WS connect to `--backend` with headers `X-Device-Id`, `X-Device-Role: ios-client`. No CF headers (local relay does not enforce them).
3. Wait for initial server frame (unpaired event).
4. Send `LocalRpc{method: Pair, params: {token, device_name}}`.
5. On `LocalRpcResponse` success, print the received `device_secret` to stderr.
6. Stay connected, emit `ping` LocalRpc every 5s, print inbound events.
7. On SIGINT: send `LocalRpc{method: ForgetPeer}` (optional — controlled by `--forget-on-exit` flag; default off), then close cleanly.

Total expected size: under 150 lines of Rust. `clap` is the only dep beyond what `minos-mobile` already has (tokio, tokio-tungstenite, serde_json, uuid).

### 10.4 Logging

Unchanged sinks. New structured log fields when the relay task runs:
- `relay_link` — one of `disconnected`, `connecting`, `connected`
- `peer_state` — one of `unpaired`, `pairing`, `paired`
- `envelope_kind` — on every WS frame in/out: `local_rpc`, `local_rpc_response`, `forward`, `forwarded`, `event`

The daemon's `name_prefix` stays `daemon`; no separate relay-client log file.

---

## 11. Out of Scope / Roadmap

| Item | Phase | Rationale |
|---|---|---|
| iOS / Flutter migration to relay | Separate future spec | This spec is explicitly Mac-only to keep diff size and blast radius small |
| Release CI job injecting `MINOS_BACKEND_URL` from secrets | P1.5 release pipeline spec | Adds signing, notarization, artifact upload — too large for this diff |
| `DeviceSecret` rotation UX (without forget) | P1.5 | Keychain overwrite UI, timing with relay for atomic switch |
| Relay admin console on `/admin` | P1 (relay spec reserves path) | Not Mac-app concern |
| Queue-and-replay on peer offline | P1 | Relay currently synthesizes `peer offline` JSON-RPC errors; outbox belongs on relay side |
| E2EE of forward payloads | P2 | Content-layer security; relay-side X25519 exchange |
| Multi-pair (one Mac, multiple peers) | P2 | UI + routing changes; schema already undirected |
| Agent execution (codex app-server, PTY) | Landed separately via plan 04's codex path | Uses `Forward` unchanged; Mac-app contributes nothing beyond the envelope wiring in this spec |
| Telemetry / usage reporting | P3 | Requires privacy policy, opt-in UI |

---

## 12. Open Questions

None remaining. Brainstorming resolved:

1. Spec scope → Mac-only (A).
2. Tailscale retention → full removal (vs. compile-time feature flag, runtime trait, iOS-compat abstraction).
3. `backend_url` source → compile-time `option_env!` with CI secrets, not user config.
4. CF creds source → 2-field onboarding sheet + Keychain, env override for dev.
5. State model → two-axis (`RelayLinkState` + `PeerState`) with two UniFFI observer traits.
6. Onboarding vs. Settings → two separate SwiftUI views (`OnboardingSheet` vs. `SettingsSheet`).
7. End-to-end smoke without iOS → `fake-peer` dev bin.
8. Where the fake-peer lives → `crates/minos-mobile/src/bin/fake-peer.rs` (not xtask, not a new crate).
9. Old `devices.json` migration → **not handled**; fresh-start is acceptable at this phase.

---

## 13. ADR Index

One new ADR accompanies this spec:

| # | Topic |
|---|---|
| 0013 | macOS relay-client cutover shape: Tailscale full removal; compile-time `backend_url`; dual-axis state model |

0013 justifies the bundled decision (rather than three separate ADRs) because the three sub-decisions are coupled — each assumes the others (e.g., full Tailscale removal is only sensible with relay auto-connect, which requires compile-time backend URL for release builds, which is only clean with a two-axis state that matches the relay's event model).

Numbering note: the ADR directory has two 0009 and two 0010 entries from parallel plans landing independently. This spec skips over the collision range and takes 0013.

---

## 14. File Inventory

### 14.1 New files

```
crates/minos-domain/src/relay_state.rs
crates/minos-mobile/src/bin/fake-peer.rs
crates/minos-daemon/src/config.rs                        # BACKEND_URL + RelayConfig
crates/minos-daemon/src/relay_client.rs                  # WS client task + envelope dispatcher
crates/minos-daemon/src/keychain_store.rs                # replaces file_store.rs
apps/macos/Minos/Presentation/OnboardingSheet.swift
apps/macos/Minos/Presentation/SettingsSheet.swift
apps/macos/Minos/Application/RelayLinkObserver.swift
apps/macos/Minos/Application/PeerObserver.swift
apps/macos/Minos/Domain/RelayLinkState+Display.swift
apps/macos/Minos/Domain/PeerState+Display.swift
apps/macos/Minos/Infrastructure/KeychainRelayConfig.swift
apps/macos/MinosTests/Application/OnboardingFlowTests.swift
apps/macos/MinosTests/Application/RelayLinkStateTests.swift
apps/macos/MinosTests/Application/PeerStateTests.swift
docs/adr/0013-macos-relay-client-cutover.md
```

### 14.2 Modified files

```
crates/minos-domain/src/lib.rs                   # pub mod relay_state
crates/minos-domain/src/error.rs                 # 6 new variants + ErrorKind + zh/en strings
crates/minos-domain/src/connection.rs            # ConnectionState deleted
crates/minos-pairing/src/lib.rs                  # Pairing state machine removed; ActiveToken removed
crates/minos-pairing/src/store.rs                # QrPayload schema; TrustedDevice drops tailscale_ip
crates/minos-transport/src/lib.rs                # server module removed
crates/minos-transport/src/client.rs             # connect(url, auth); close-code mapping
crates/minos-transport/src/auth.rs               # AuthHeaders struct + header build
crates/minos-daemon/src/lib.rs                   # tailscale mod + file_store mod removed; relay_client + keychain_store mods added
crates/minos-daemon/src/handle.rs                # start() replaces start_autobind(); 6 Phase 0 steps
crates/minos-daemon/src/main.rs                  # doctor CLI tailscale branch removed
crates/minos-daemon/Cargo.toml                   # add security-framework (macOS-gated)
crates/minos-ffi-uniffi/src/lib.rs               # discover_tailscale_ip removed; 2 observer traits + DeviceSecret added
crates/minos-mobile/Cargo.toml                   # [[bin]] fake-peer target + clap (optional, bin-scoped)
apps/macos/Minos/MinosApp.swift                  # phase-aware bootstrap entry
apps/macos/Minos/Application/AppState.swift      # relayLink / peer / phase / trustedDevice fields
apps/macos/Minos/Application/DaemonDriving.swift # new protocol surface
apps/macos/Minos/Infrastructure/DaemonBootstrap.swift  # Keychain/env, onboarding gate, dual observer setup
apps/macos/Minos/Infrastructure/DaemonHandle+DaemonDriving.swift  # adapt to new surface
apps/macos/Minos/Presentation/MenuBarView.swift  # 3 layouts + Settings menu + disabled-forget logic
apps/macos/Minos/Presentation/QRSheet.swift      # new-schema QrPayload; same CIFilter pipeline
apps/macos/Minos/Presentation/StatusIcon.swift   # (RelayLinkState, PeerState) symbol matrix
apps/macos/Minos/Domain/MinosError+Display.swift # 6 new variants mapped
apps/macos/MinosTests/Application/AppStateTests.swift   # rewritten per §9.2
apps/macos/MinosTests/TestSupport/MockDaemon.swift      # new protocol shape
.github/workflows/ci.yml                         # TODO-comment for future release job (no behavior change)
README.md                                        # Status line refreshed
```

### 14.3 Deleted files

```
crates/minos-daemon/src/tailscale.rs                     # whole file
crates/minos-daemon/src/file_store.rs                    # whole file (replaced by keychain_store.rs)
crates/minos-daemon/tests/autobind.rs                    # Tailscale port-retry test
crates/minos-transport/src/server.rs                     # whole file (and its tests)
crates/minos-pairing/src/state_machine.rs                # if standalone; else folded removal in lib.rs
apps/macos/Minos/Application/ObserverAdapter.swift       # superseded by two observers
apps/macos/Minos/Domain/ConnectionState+Display.swift    # superseded by RelayLinkState+Display
```

---

## 15. Transition Plan for the Implementer

After this spec is approved, the implementation follows on the same branch `feat/macos-relay-migration`:

1. Invoke `superpowers:writing-plans` to produce `docs/superpowers/plans/05-macos-relay-client-migration.md`.
2. Plan document breaks §5.2's 6 Phase 0 steps into sub-tasks with per-commit acceptance criteria.
3. Plan also schedules Swift-layer updates (OnboardingSheet → SettingsSheet → MenuBarView → StatusIcon → XCTests), UniFFI re-gen, fake-peer bin, ADR 0013 body.
4. Implementation proceeds commit-by-commit, with `cargo xtask check-all` gating each commit.
5. Final PR from `feat/macos-relay-migration` to `main` after §2.3 smoke passes on the implementer's machine.
