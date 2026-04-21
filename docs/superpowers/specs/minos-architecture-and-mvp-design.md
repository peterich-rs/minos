# Minos · Architecture and MVP Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-21 |
| Owner | fannnzhang |
| Repository | `github.com/peterich-rs/minos` (public) |
| Related | ADR `0001`–`0006` under `docs/adr/` |

---

## 1. Context

Minos is a multi-platform companion for slock.ai-style remote AI-coding control: drive a Mac running coding agents (`codex`, `claude`, `gemini`) from a paired mobile client. The reference open-source project [Emanuele-web04/remodex](https://github.com/Emanuele-web04/remodex) establishes a credible baseline (iPhone ↔ relay ↔ macOS Node bridge ↔ `codex app-server`); Minos goes further on three axes:

- **Native macOS GUI** instead of headless CLI daemon: a status-bar app (SwiftUI `MenuBarExtra`) so users see connection status, paired devices, and detected CLIs without opening a terminal.
- **Cross-platform mobile** with native performance ceiling: Flutter UI + Rust business logic via `flutter_rust_bridge` v2; `PlatformView` reserved as escape hatch for any chat surface that proves to need native rendering.
- **Shared Rust core** between Mac daemon and mobile client, exposed through UniFFI (Swift) and frb (Dart). One source of truth for protocol, pairing, and agent abstractions.

This document specifies the MVP (first deliverable), the architecture that supports it, and the explicit shape of what comes after.

---

## 2. Goals

### 2.1 MVP (this spec scope)

1. End-to-end connection bring-up over Tailscale: install Mac app → click → show QR → scan from iOS app → WebSocket connection established and surfaced in both UIs.
2. Persisted pairing: re-launching either side reconnects without re-scanning.
3. One real RPC over the connection: mobile fetches and displays the list of locally-detected CLIs (`codex` / `claude` / `gemini`) with path, version, and health status.
4. Reconnect resilience: WebSocket drops trigger exponential-backoff reconnect; UI surfaces the `Reconnecting` state.
5. Project scaffolding with quality tooling: `cargo fmt` / `cargo clippy` / `cargo test`, `dart format` / `dart analyze` / `flutter test`, `swiftlint`, all wired into one `cargo xtask check-all` command and a minimal GitHub Actions CI.

### 2.2 Non-goals (MVP)

End-to-end encryption; cloud relay; push notifications; multi-Mac/multi-iPhone pairing; Android validation; LaunchAgent autostart; agent execution (codex `app-server`, claude/gemini PTY); chat UI; streaming markdown rendering; git/workspace operations; release pipeline (notarization, TestFlight, signed builds). All scheduled in §11 Roadmap.

---

## 3. Tech Stack and Defaults

| Layer | Choice | Note |
|---|---|---|
| Project codename | `minos` | Crate prefix; Bundle ID prefix `ai.minos.*` |
| License | MIT | Single LICENSE file |
| Repository | `github.com/peterich-rs/minos` (public) | |
| macOS minimum | macOS 13 (Ventura) | Unlocks SwiftUI `MenuBarExtra` |
| iOS minimum | iOS 16 | |
| Rust toolchain | `stable` (no MSRV pinning) | End-product app, no downstream consumers |
| Async runtime | `tokio` (full features) | |
| RPC | JSON-RPC 2.0 over WebSocket via `jsonrpsee` | Same protocol as `codex app-server` |
| WebSocket | `tokio-tungstenite` | Server (Mac) and client (iOS) |
| UniFFI | 0.30.x | Swift async support |
| flutter_rust_bridge | 2.x (latest) | Native assets / build hooks integrated |
| Flutter | 3.41.x (latest stable at scaffold) | |
| Dart UI library | `shadcn_ui` (nank1ro/flutter-shadcn-ui) latest | `ShadApp` root |
| State management | `flutter_riverpod` 2.x + `riverpod_annotation` + codegen | `riverpod_lint` enforces |
| QR scanning | `mobile_scanner` | |
| Logging | `mars-xlog` (peterich-rs/xlog-rs) on Rust + Dart sides | `tracing::Layer` integration; OSLog complements |
| Mac UI | SwiftUI `MenuBarExtra` (no Dock icon) | `LSUIElement = true` |
| Architecture style | Clean Arch in Swift / Dart; **crate-bordered hexagonal** in Rust | See ADR 0003 |

### 3.1 Tailscale assumption

Users install Tailscale on both Mac and iPhone and log into the same tailnet. Minos does **not** embed `tsnet` (would require macOS Network System Extension and iOS NetworkExtension entitlements — far beyond MVP). The Mac binds the WebSocket server to its `100.x.y.z` Tailscale IP; the iPhone connects to that IP. The OS handles the WireGuard layer.

---

## 4. Architecture Overview

```
┌─────────────────── macOS (100.64.0.10) ────────────────────┐
│ ┌──────────────────────── Minos.app ─────────────────────┐ │
│ │ Swift / SwiftUI                                        │ │
│ │  ├─ MenuBarExtra UI (status, QR, devices, CLIs)        │ │
│ │  └─ Presentation → Application → Domain → Infra        │ │
│ │                            │ UniFFI (sync + async)     │ │
│ │ ┌──────────────────────────▼──────────────────────────┐│ │
│ │ │ libminos_ffi_uniffi  (statically linked into .app)  ││ │
│ │ │   re-exports → minos-daemon::DaemonHandle           ││ │
│ │ └──────────────────────────┬──────────────────────────┘│ │
│ │   minos-daemon (tokio)     │                           │ │
│ │     ├─ minos-transport (jsonrpsee Server / WS server) │ │
│ │     ├─ minos-pairing (state machine + JSON store)     │ │
│ │     ├─ minos-cli-detect (which + --version)           │ │
│ │     └─ (P1+) minos-agent-runtime (codex/PTY)          │ │
│ │                            │                           │ │
│ │  WS listen 100.x.y.z:7878 │ JSON-RPC 2.0 over WS      │ │
│ └────────────────────────────┼───────────────────────────┘ │
└──────────────────────────────┼─────────────────────────────┘
                               │  Tailscale 100.x ↔ 100.x
┌──────────────────────────────┼─────────────────────────────┐
│             iOS (100.64.0.42)│                             │
│ ┌──────────────────────── Minos (Flutter) ───────────────┐ │
│ │ Dart / Flutter UI                                      │ │
│ │  ├─ presentation (scan QR, status, CLIs list)          │ │
│ │  └─ application → domain → infrastructure              │ │
│ │                            │ flutter_rust_bridge v2    │ │
│ │ ┌──────────────────────────▼──────────────────────────┐│ │
│ │ │ libminos_ffi_frb  (statically linked into iOS app)  ││ │
│ │ │   re-exports → minos-mobile::MobileClient           ││ │
│ │ └──────────────────────────┬──────────────────────────┘│ │
│ │   minos-mobile (tokio)     │                           │ │
│ │     ├─ minos-transport (jsonrpsee Client / WS client) │ │
│ │     └─ minos-pairing (client side; trusted Mac store) │ │
│ └────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────┘
```

### 4.1 Process model

- **Mac**: single process. SwiftUI app and tokio runtime in the same process, bridged by UniFFI.
- **iOS**: single process. Flutter app and tokio runtime in the same process, bridged by frb.
- No separate daemon binary in MVP. (A future `minosd` headless variant for LaunchAgent is in P1.5.)

### 4.2 Protocol stack (top-down)

`Tailscale (WireGuard)` → `TCP` → `WebSocket` → `JSON-RPC 2.0` → `pair / list_clis / health / subscribe_events (placeholder)`.

### 4.3 FFI boundary discipline

Swift only calls `DaemonHandle`. Dart only calls `MobileClient`. Both are Rust-side facades that hide tokio, jsonrpsee, transport, and pairing internals. UI layers must not synthesize JSON-RPC payloads themselves.

---

## 5. Components

### 5.1 Rust workspace (9 crates + `xtask`)

| Crate | Hexagonal role | MVP responsibility | Key public types / API |
|---|---|---|---|
| `minos-domain` | Entities | Pure value types; deps limited to `serde`, `uuid`, `thiserror` | `DeviceId`, `PairingToken`, `AgentName{Codex,Claude,Gemini}`, `AgentStatus{Ok,Missing,Error}`, `AgentDescriptor{name,path,version,status}`, `ConnectionState{Disconnected,Pairing,Connected,Reconnecting}`, `MinosError` |
| `minos-protocol` | Adapters / contract | `jsonrpsee::proc-macros::rpc` service trait; serde schema in one place | `trait MinosRpc` with `pair`, `health`, `list_clis` and a placeholder `subscribe_events` |
| `minos-pairing` | Use cases | Pairing state machine + `PairingStore` trait; states `Unpaired → AwaitingPeer → Paired`; token generation (32B `getrandom` + base64url) | `Pairing`, `PairingStore` (trait), `PairingError`, `generate_qr_payload` |
| `minos-cli-detect` | Use cases | Mac-only: `which codex/claude/gemini` + `<bin> --version` with 5s timeout | `detect_all() -> Vec<AgentDescriptor>`; `CommandRunner` trait for testability |
| `minos-transport` | Adapters | WS server (`tokio-tungstenite` accept + jsonrpsee `Server`) / WS client (connect + reconnect, exponential backoff `1s→2s→4s→…→30s`) | `WsServer::bind`, `WsClient::connect`, `ConnectionEvent` |
| `minos-daemon` | Composition root (Mac) | Boot WS server, hold file-backed `PairingStore`, trigger CLI detection, broadcast state changes | `DaemonHandle::start`, `current_state`, `pairing_qr`, `forget_device`, `events_stream` |
| `minos-mobile` | Composition root (iOS) | WS client lifecycle, Keychain-backed `PairingStore` (delegated through frb to Dart), reconnect loop, RPC entry surface | `MobileClient::new`, `pair_with`, `list_clis`, `current_state`, `events_stream` |
| `minos-ffi-uniffi` | Adapters (FFI) | Pure re-export shim; `#[uniffi::export]` on `DaemonHandle`. Outputs cdylib + staticlib | (no business types) |
| `minos-ffi-frb` | Adapters (FFI) | Pure re-export shim; frb v2 macros on `MobileClient`. Outputs cdylib for Android; staticlib for iOS (App Store policy forbids dylibs) | (no business types) |
| `xtask` | Tooling | `cargo xtask` subcommands for codegen and cross-builds | `clap` CLI |

### 5.2 Sharing matrix

| Crate | Mac binary | iOS binary |
|---|---|---|
| `minos-domain` | ✓ | ✓ |
| `minos-protocol` | ✓ | ✓ |
| `minos-pairing` | ✓ | ✓ |
| `minos-transport` | ✓ | ✓ |
| `minos-cli-detect` | ✓ (cfg gated) | — |
| `minos-daemon` | ✓ | — |
| `minos-mobile` | — | ✓ |
| `minos-ffi-uniffi` | ✓ | — |
| `minos-ffi-frb` | — | ✓ |

UniFFI and frb cannot share a single cdylib (panic-handling and runtime initialization conflict). Two FFI shim crates are a physical constraint, not a design choice.

### 5.3 macOS app layers (`apps/macos/Minos/`)

| Layer | Folder | Responsibility |
|---|---|---|
| Presentation | `Presentation/` | `MenuBarView` (status icon + dropdown), `QRSheet` (QR display modal), `DevicesView` (paired devices list), `CLIListView` |
| Application | `Application/` | `AppState: @Observable` singleton subscribing to `DaemonHandle.events_stream()`; maps Rust events to SwiftUI-observable state |
| Domain | `Domain/` | Thin typealias / extension over UniFFI-generated types (`AgentName.displayLabel` and similar); no business logic |
| Infrastructure | `Infrastructure/` | `DaemonBootstrap`: at app launch calls `DaemonHandle::start()`, binds to local port (default `7878`, auto-increments up to 5 times if occupied), generates QR and hands it to UI |

UI framework: SwiftUI `MenuBarExtra` (macOS 13+), no Dock icon (`LSUIElement = true`).

### 5.4 Flutter app layers (`apps/mobile/lib/`)

| Layer | Folder | Responsibility |
|---|---|---|
| presentation | `lib/presentation/` | `pairing_page.dart` (mobile_scanner), `home_page.dart` (connection status + CLI list + "forget this Mac") |
| application | `lib/application/` | `app_state.dart` Riverpod providers subscribing to `MobileClient.events_stream()`, exposed as `Stream<UiState>` |
| domain | `lib/domain/` | Thin Dart typedefs over frb-generated types |
| infrastructure | `lib/infrastructure/` | `client_bootstrap.dart`: instantiate `MobileClient`, inject `KeychainPairingStore` (Dart impl of the pairing-store trait, called back via frb) |

UI library: `ShadApp` (shadcn_ui) at root; QR scanning via `mobile_scanner`; state via `flutter_riverpod` 2.x with `riverpod_annotation` + `riverpod_generator` codegen.

---

## 6. Data Flow

### 6.1 First-time pairing

```
[macOS]                                              [iOS]

1. User opens Minos.app
   ├─ Infra: DaemonHandle::start()
   │   ├─ minos-pairing: file store check → unpaired
   │   ├─ generate PairingToken (32B base64url)
   │   ├─ minos-transport WS server bind 100.x.y.z:7878
   │   └─ state machine: Unpaired → AwaitingPeer
   └─ UI: MenuBarView shows "Awaiting pairing"

2. User clicks "Show QR" in menu
   └─ QRSheet displays QR encoding:
      {
        "v": 1,
        "host": "100.64.0.10",   // Tailscale IP
        "port": 7878,
        "token": "8x7Ja...",     // base64url(32B)
        "name": "fannnzhang's MacBook"
      }
                                                     3. User taps "Scan to pair" in iOS app
                                                        └─ presentation: PairingPage opens mobile_scanner

                                                     4. QR captured
                                                        ├─ infra: KeychainPairingStore writes trusted Mac record
                                                        └─ MobileClient.pair_with(qr) ↓

                                                     5. minos-mobile:
                                                        ├─ minos-transport WS connect ws://100.64.0.10:7878/
                                                        │   header: X-Minos-Token: 8x7Ja...
                                                        └─ JSON-RPC: pair({device_id, name: "fannnzhang's iPhone"})

6. minos-daemon WS server accept
   ├─ verify X-Minos-Token == current PairingToken? else 401 close
   ├─ verify state == AwaitingPeer? else reject
   ├─ pair RPC → persist trusted device (id, name, paired_at)
   ├─ state machine: AwaitingPeer → Paired
   └─ RPC reply {ok: true, mac_name: "fannnzhang's MacBook"}
   ↓ events_stream emits ConnectionState::Connected
   ↓
   UI: MenuBarView shows "Connected (1 device)"
   QRSheet auto-dismisses
                                                     7. RPC succeeds
                                                        ↓ events_stream emits Connected
                                                        ↓
                                                        UI: navigates to HomePage
```

### 6.2 MVP real RPC (mobile fetches CLI list)

```
[iOS]                                              [macOS]

1. HomePage entry / user taps refresh
   └─ application: state.refresh()

2. MobileClient.list_clis()
   └─ JSON-RPC request: list_clis()  ────────►  3. minos-daemon RPC handler
                                                   ├─ minos-cli-detect::detect_all()
                                                   │   ├─ which codex → /usr/local/bin/codex
                                                   │   ├─ codex --version → "0.18.2" (5s timeout)
                                                   │   ├─ which claude → /opt/homebrew/bin/claude
                                                   │   ├─ claude --version → "1.2.0"
                                                   │   ├─ which gemini → not found
                                                   │   └─ Vec<AgentDescriptor>
                                                   └─ JSON-RPC response:
4. RPC reply ◄─────────────────────────────────     [
   └─ application: state.set(clis)                    {name:"codex",  path, version, status: Ok},
   └─ presentation: CLIListView rebuilds              {name:"claude", path, version, status: Ok},
                                                      {name:"gemini", status: Missing}
                                                    ]
```

### 6.3 Reconnect (app relaunch / network blip)

```
[macOS app launch]                                 [iOS app launch / network restored]

1. DaemonHandle::start()
   ├─ PairingStore reads disk → trusted device exists
   ├─ WS server bind 100.x.y.z:7878
   ├─ state machine: Paired (awaiting peer reconnect)
   └─ UI: "Paired, awaiting peer"
                                                 2. MobileClient::start()
                                                    ├─ KeychainPairingStore reads → trusted Mac exists
                                                    ├─ minos-transport WS connect
                                                    │   ws://100.64.0.10:7878/
                                                    │   header: X-Minos-Device-Id: <uuid>
                                                    │   (note: no token; device_id only)
                                                    └─ on failure: backoff 1s/2s/4s/8s/.../30s cap

3. WS server accept
   ├─ verify X-Minos-Device-Id ∈ trusted devices? else 401
   └─ verified → events_stream emits Connected
   ↓
   UI: "Connected"                              4. WS upgrade succeeds → events_stream Connected
                                                    ↓
                                                    UI: HomePage
```

### 6.4 MVP simplifications (explicit)

- Pairing token (32B base64url) validated only on `pair`; subsequent reconnects identify with `device_id` from trusted records.
- Token TTL: 5 minutes. Expired → regenerated on next QR display.
- One pairing token may successfully `pair` at most once. Subsequent attempts rejected.
- Single trusted device pair (one Mac ↔ one iPhone). Second pair attempt shows confirmation dialog "replace existing".
- Tailscale IP captured at pair time and stored in trusted record. No mDNS / Bonjour fallback in MVP. IP changes require manual re-pair.

---

## 7. Error Handling

### 7.1 Philosophy

- **Errors are values**: Rust public APIs return `Result<T, MinosError>`; library crates **never `panic!`** except `unreachable!` for proven-impossible branches.
- **Single error enum**: `MinosError` with `thiserror`. Each variant carries minimal context (operation name / path / exit code / IO source). No nested `ErrorKind` trees, no error-wrapper chains.
- **Logging and errors are separate channels**: `tracing` records process; `MinosError` records results. On error, synchronously emit `tracing::warn!` with the full source chain; UI uses only the `MinosError` value to decide what to display.
- **User-visible text ≠ debug text**: each `MinosError` variant has `fn user_message(&self, lang: Lang) -> &'static str` (MVP ships zh + en constant tables; no i18n library).

### 7.2 `MinosError` enum (MVP shape)

```rust
// minos-domain
#[derive(thiserror::Error, Debug)]
pub enum MinosError {
    // ── network / WS layer ──
    #[error("websocket bind failed on {addr}: {source}")]
    BindFailed { addr: String, #[source] source: std::io::Error },

    #[error("websocket connect to {url} failed: {source}")]
    ConnectFailed { url: String, #[source] source: tokio_tungstenite::tungstenite::Error },

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
    #[error("store io failed at {path}: {source}")]
    StoreIo { path: String, #[source] source: std::io::Error },

    #[error("store payload corrupt at {path}: {source}")]
    StoreCorrupt { path: String, #[source] source: serde_json::Error },

    // ── CLI probe layer ──
    #[error("cli probe timeout: {bin} after {timeout_ms}ms")]
    CliProbeTimeout { bin: String, timeout_ms: u64 },

    #[error("cli probe failed: {bin}: {source}")]
    CliProbeFailed { bin: String, #[source] source: std::io::Error },

    // ── RPC layer ──
    #[error("rpc call failed: {method}: {message}")]
    RpcCallFailed { method: String, message: String },
}
```

MVP intentionally omits a `MinosError::Internal(String)` catch-all variant (becomes a misuse magnet). `anyhow` is **not** used in public APIs; it is allowed only in `xtask` and tests.

### 7.3 Cross-language mapping

| Boundary | Mechanism | Behavior |
|---|---|---|
| **Rust → Swift** (UniFFI 0.30) | `MinosError` annotated `#[uniffi::Error]`; UniFFI generates Swift `enum MinosError: Error` with throwing methods | `do { try await daemon.start() } catch let e as MinosError { ... }`; variants preserved for `switch` in UI |
| **Rust → Dart** (frb v2) | frb maps `Result<T, MinosError>` to `Future<T>`; errors throw a generated sealed-class hierarchy | `try { await client.listClis(); } on MinosErrorPairingTokenInvalid { ... }` |
| **Mac UI rendering** | `MinosError.user_message(.zh)` used in `Alert` or MenuBar tooltip; "Copy log path" affordance | Logs at `~/Library/Logs/Minos/*.xlog` |
| **Mobile UI rendering** | `ShadToast` (shadcn_ui) for short messages; `LogPage` retained in debug builds; OSLog passthrough for system Console | Logs in iOS app `Documents/Minos/Logs/*.xlog` |

### 7.4 Failure modes that must be handled in MVP

| # | Trigger | Rust error | UI behavior | MVP |
|---|---|---|---|---|
| 1 | Tailscale not running / not logged in on Mac | `BindFailed` (cannot bind to 100.x or no 100.x at all) | MenuBarView: "Start and log into Tailscale first" + Retry | ✓ |
| 2 | Port 7878 occupied | `BindFailed` | Auto-increment +1 up to 5 retries; final fail → "Ports 7878–7882 all occupied, please check" | ✓ |
| 3 | QR token expired (5 min) | `PairingTokenInvalid` | Mac UI regenerates QR; iOS UI: "QR expired, please rescan" | ✓ |
| 4 | iOS reconnects but its `device_id` is no longer in the Mac's trusted store (Mac side forgot the device, or `devices.json` was reset) | `DeviceNotTrusted` | iOS: "Pairing invalidated, please rescan" + return to PairingPage | ✓ |
| 5 | iOS scans a second QR while Mac is `Paired` | `PairingStateMismatch` | Mac UI: "Existing paired device (A); continue will replace" confirmation; iOS: "Mac is already paired with another device" | ✓ |
| 6 | WS dropped (phone lock / network switch) | `Disconnected` | Mac UI returns to "Paired, awaiting reconnect"; iOS top banner "Reconnecting… (attempt N)" | ✓ |
| 7 | A CLI probe hangs | `CliProbeTimeout` (5s) | Affected row shows "status: error (timeout)"; other CLIs render normally | ✓ |
| 8 | `~/Library/Application Support/minos/devices.json` corrupt | `StoreCorrupt` | Mac UI: "Pairing state file corrupt; backed up as `.bak`. Please re-pair" + "View logs" | ✓ (fail-safe reset) |
| 9 | iOS Keychain access denied | `StoreIo` | iOS UI: "Cannot access Keychain; please grant in Settings" + deeplink | ✓ |
| 10 | jsonrpsee server panic (internal request error) | `RpcCallFailed` | iOS UI: toast "Server error, please retry"; Mac side does **not** auto-restart RPC server in MVP (avoid restart loops masking real bugs) | ✓ (toast only) |

Explicitly out of MVP failure handling: offline-LLM fallback, MITM detection, replay protection (meaningless without E2EE), multi-Mac/multi-iPhone conflict resolution (single-pair forced; #5 covers replacement), automated diagnostic wizard. MVP provides a "Copy diagnostics" button that copies the last 200 log lines to clipboard.

---

## 8. Testing Strategy

**Default**: business-logic crates (`minos-domain` / `minos-protocol` / `minos-pairing` / `minos-cli-detect` / `minos-transport`) follow TDD — failing test first, then implementation. FFI shim crates (`minos-ffi-uniffi` / `minos-ffi-frb`) only get build-and-shape smoke checks. UI layers cover key interactions with widget/view tests; coverage is not chased to 100%.

### 8.1 Rust matrix

| Layer | Test type | Tools | Key targets |
|---|---|---|---|
| `minos-domain` | Unit + serde golden | `serde_json` round-trip; `tests/golden/*.json` | Type invariants; JSON schema does not drift |
| `minos-protocol` | Unit + schema golden | jsonrpsee mock helpers | `pair` / `list_clis` / `health` request and response shapes pinned by golden files |
| `minos-pairing` | Table-driven state machine + property tests | `rstest` + `proptest` | All legal `Unpaired→AwaitingPeer→Paired` paths; illegal transitions return `PairingStateMismatch`; `generate_qr_payload` token entropy property (1000 random draws no duplicates) |
| `minos-cli-detect` | Unit (subprocess injected via trait) | `CommandRunner` trait + mock impls | which not found; bin exists but `--version` times out; bin exists but exits nonzero; version-string parsing variants |
| `minos-transport` | Integration (in-process loopback) | tokio test runtime; `tokio_tungstenite` server+client in same test | WS upgrade handshake; token mismatch → 401; reconnect backoff sequence (mock time) |
| `minos-daemon` / `minos-mobile` | Integration (in-process full stack) | The transport tests above + fake `PairingStore` | "pair → list_clis → disconnect" full pipeline in a single `#[tokio::test]`; runs in < 1s |
| `minos-ffi-uniffi` | Build smoke + UDL shape | `cargo test` + grep generated types | No business tests |
| `minos-ffi-frb` | Build smoke + frb-generated shape | Same | No business tests |

Rust E2E lives at `crates/minos-daemon/tests/e2e.rs` and exercises start-server → fake-mobile → pair → list_clis → shutdown. No FFI involved; this is the pre-FFI confidence anchor.

### 8.2 Swift matrix

| Type | Tools | Scope |
|---|---|---|
| Unit (Domain/Application) | XCTest + Swift Testing | Mock `DaemonHandle` via a protocol shim around the UniFFI type to enable injection; cover `AppState` reducer logic |
| View | SwiftUI Preview snapshots (no third-party lib) | `MenuBarView` / `QRSheet` / `CLIListView` rendered in 3 states without crashing |
| Smoke | Manual checklist (§8.4) | |

No snapshot-testing library in MVP (high maintenance cost vs payoff at this scale).

### 8.3 Dart / Flutter matrix

| Type | Tools | Scope |
|---|---|---|
| Unit (application/domain) | `flutter_test` + `mocktail` | Mock `MobileClient` (frb-generated interface); test Riverpod providers' state transitions |
| Widget | `flutter_test` `WidgetTester` wrapped in `ShadApp` | `PairingPage` scan event → state change; `HomePage` renders all three `ConnectionState` variants; shadcn components render in dark + light themes without errors |
| Integration | `integration_test` package | **Not** in MVP — requires real device + real Mac and CI cannot run it. Scheduled for P1.5. |
| Smoke | Manual checklist (§8.4) | |

`riverpod_lint` runs in widget tests too: provider overrides must use `overrideWith` (catches manual patching).

### 8.4 MVP smoke acceptance checklist (hard gate before tagging v0.1.0)

```
□ Mac: Tailscale installed, logged in, 100.x IP visible
□ iPhone: Tailscale installed, logged in, 100.x IP visible, can ping Mac
□ Mac: Minos.app installed via dmg, launched from Applications, MenuBar icon visible
□ Click "Show QR" in menu → QRSheet appears, QR visible
□ iOS: Minos installed via TestFlight, launched, PairingPage shown
□ Scan QR → within 5s, PairingPage navigates to HomePage showing "Connected to fannnzhang's MacBook"
□ HomePage shows codex/claude/gemini rows (installed → path + version; missing → status: missing)
□ Restart iOS app → reconnects automatically without re-scan
□ Restart Mac app → iPhone reconnects within 30s (backoff range)
□ `tailscale down` on Mac (network drop) → iPhone shows "Reconnecting" banner; restoring brings it back automatically
□ Click "Forget this device" on Mac → iPhone immediately shows "Pairing revoked, please re-scan"
```

11 boxes ticked = MVP complete.

### 8.5 CI (GitHub Actions, minimal)

`.github/workflows/ci.yml` with three parallel jobs:

| Job | Runner | Steps |
|---|---|---|
| `rust` | `ubuntu-latest` (cross-platform builds happen in release pipeline, not CI) | `cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo deny check` (license / advisory scan) |
| `dart` | `ubuntu-latest` | `flutter pub get`; `dart format --set-exit-if-changed`; `dart analyze --fatal-infos`; `flutter test`; `dart run custom_lint` |
| `swift` | `macos-14` | `swiftlint --strict` (lint only; Xcode build is slow on CI and signing is unavailable in MVP — kept local) |

Out of MVP CI: Xcode device/simulator builds; Flutter iOS/Android builds; UniFFI/frb cross-platform artifact builds; release packaging. All deferred to the P1.5 release pipeline spec.

---

## 9. Tooling and Developer Ergonomics

### 9.1 Pinned files at repository root

| File | Content (essentials) | Purpose |
|---|---|---|
| `rust-toolchain.toml` | `channel = "stable"`; `components = ["rustfmt","clippy","rust-src"]`; `targets = ["aarch64-apple-darwin","x86_64-apple-darwin","aarch64-apple-ios","aarch64-apple-ios-sim"]` | Track latest stable; targets pre-installed via rustup |
| `apps/mobile/pubspec.yaml` | `environment: { flutter: ">=3.41.0 <4.0.0", sdk: "^3.6.0" }` | Pin Flutter and Dart SDK |
| `.tool-versions` (asdf) | `nodejs`, `flutter`, `rust` | Optional asdf user convenience |
| `.swiftformat` | width 120, `--swiftversion 5.10` | Style consistency |
| `.swiftlint.yml` | `disabled_rules: [identifier_name]`; `opt_in_rules: [empty_count, force_unwrapping]`; `included: ["apps/macos"]` | Strict but practical |
| `.gitattributes` | `* text=auto eol=lf` for Rust/Swift/Dart files | Avoid CRLF mix on Windows checkouts |

### 9.2 `cargo xtask` command catalog

`xtask/` is a regular binary crate. `.cargo/config.toml` aliases `cargo xtask` for ergonomics.

| Command | Action |
|---|---|
| `cargo xtask gen-uniffi` | Run `uniffi-bindgen generate --library target/.../libminos_ffi_uniffi.dylib --language swift --out-dir apps/macos/Minos/Generated/` |
| `cargo xtask gen-frb` | Run `flutter_rust_bridge_codegen generate` (config in `flutter_rust_bridge.yaml`) |
| `cargo xtask build-macos` | Build `minos-ffi-uniffi` for `aarch64-apple-darwin` and `x86_64-apple-darwin`; `lipo` into `target/xcframework/MinosCore.xcframework` |
| `cargo xtask build-ios` | Build `minos-ffi-frb` for `aarch64-apple-ios` and `aarch64-apple-ios-sim`; output `target/ios/`; frb's build hook reads from there |
| `cargo xtask check-all` | Sequential: `cargo fmt --check` → `cargo clippy --workspace -- -D warnings` → `cargo test --workspace` → `(cd apps/mobile && dart format --set-exit-if-changed && dart analyze && flutter test)` → `swiftlint --strict apps/macos`. Single command for local pre-push and CI |
| `cargo xtask bootstrap` | Install `uniffi-bindgen`, `flutter_rust_bridge_codegen`, `cargo-deny`, `cargo-audit`; in `apps/mobile/` run `flutter pub get` and `dart run build_runner build` (Riverpod codegen) |

`xtask/src/main.rs` stays under ~200 lines for MVP. Rationale for xtask over Makefile/justfile: Rust-typed, cross-platform, no shell-flavor pitfalls.

### 9.3 Pre-commit / pre-push hooks

**Not in MVP.** CI covers `check-all`. Pre-commit hooks slow contributor velocity and skipping them creates inconsistency. Reconsider in a future ADR if the contributor count grows.

### 9.4 Logging conventions

| Boundary | Library | Sink | `name_prefix` |
|---|---|---|---|
| Rust daemon (Mac process) | `tracing` + `mars_xlog::XlogLayer` | `~/Library/Logs/Minos/` | `daemon` |
| Rust core (iOS process) | `tracing` + `mars_xlog::XlogLayer` | iOS app `Documents/Minos/Logs/` | `mobile-rust` |
| Swift app | `OSLog` subsystem `ai.minos.macos` | Console.app + Unified log | — |
| Flutter app | `package:xlog` (peterich-rs/xlog Dart package) | Same iOS Documents directory | `mobile-flutter` |
| Decoder | `third_party/mars/.../decode_mars_nocrypt_log_file.py` (referenced from README) | — | — |

Single-writer constraint: same `(name_prefix, log_dir)` pair allows only one writer. Mac and iOS each have multiple distinct prefixes to permit Rust and language-host loggers to coexist in the same directory.

Standard log fields (every cross-boundary log carries at least one): `device_id`, `peer_device_id`, `rpc_method`, `pairing_state`.

### 9.5 ADRs

`docs/adr/` uses MADR 4.0; format `NNNN-slug.md`; sections `Context / Decision / Consequences / Alternatives Rejected`.

```
docs/adr/
├── 0001-monorepo-layout.md
├── 0002-mobile-stack-flutter-frb.md
├── 0003-rust-clean-arch-deviation.md
├── 0004-jsonrpc2-over-ws.md
├── 0005-no-e2ee-in-mvp.md
└── 0006-logging-with-mars-xlog.md
```

All six are written alongside this spec.

---

## 10. Out of Scope (MVP)

| Item | Phase | Rationale |
|---|---|---|
| End-to-end encryption (X25519 + Ed25519 + AES-GCM) | P2 | Tailscale provides transport security; MVP threat model accepts that |
| Cloud relay (self-hosted or third-party) | P3 | Tailscale already enables remote interconnect |
| Push notifications (APNs / FCM) | P2 | Pointless before agent runtime emits asynchronous tasks |
| Multi-Mac / multi-iPhone pairing | P2 | Trusted store is already an array; UI cap of 1 in MVP |
| Android validation | P1.5 | Flutter project keeps `android/` directory; only acceptance is omitted |
| LaunchAgent autostart | P1.5 | Manual launch sufficient for MVP |
| Agent execution (codex `app-server`, claude/gemini PTY) | **P1** (next spec) | Channel-only validation in MVP; agent runtime is the next product step |
| Streaming markdown / chat UI | P1 | Coupled to agent execution; nothing to stream in MVP |
| Workspace git ops (commit / push / branch) | P2 | Mirrors remodex; after agent runtime |
| Workspace snapshot / revert | P2 | Same |
| Remote file attachment | P2 | Same |
| Approval hooks (sensitive op confirmation on phone) | P2 | "Sensitive op" only meaningful after agent runtime exists |
| Multi-agent workflow / orchestration (Goal-D B band) | P3 | Single-agent first |
| Team / multi-tenant (Goal-D C band) | P4+ | Server side required; long horizon |
| Browser web client (slock.ai's original surface) | P4+ | Dedicated web UI + WS client; long horizon |
| Telemetry / usage reporting | P3 | Requires privacy policy and opt-in UI first |
| Auto-updater (Sparkle for Mac) | P2 | TestFlight / DMG-manual sufficient for MVP |

### 10.1 MVP design hooks (visible shapes, no implementations)

| Hook | Shape | Effort to fill in later |
|---|---|---|
| `trait Agent { fn name() -> AgentName; async fn stream_events() -> impl Stream<Item = AgentEvent>; }` | Defined in `minos-domain`, zero impls | P1 adds `JsonRpcAgent` (codex) and `PtyAgent` (claude/gemini) |
| `enum AgentEvent { TokenChunk, ToolCall, Reasoning, ToolResult, Done, ... }` | Fully defined in `minos-domain` | No enum changes; only producers added |
| `subscribe_events()` JSON-RPC subscription | Declared via `jsonrpsee::subscription`; MVP server returns "not implemented" | P1 implements server side and stream forwarding |
| Encrypted `PairingStore` | Trait abstracted; MVP impl is plain JSON; trait reserves `seal/open` (default = passthrough) | P2 adds X25519 / Keychain-encrypted backend |
| `ConnectionState::Reconnecting` substate | Already in enum | Implemented in MVP |
| Mac "Copy diagnostics" button | Implemented in MVP (last 200 log lines to clipboard) | — |
| Mobile Tailscale-readiness `precheck()` | MVP: query macOS / iOS system API to verify 100.x IP availability | — |

---

## 11. Roadmap and Future Spec Pointers

Each future spec runs through the same cycle: brainstorm → spec → plan → execute.

Phase tags in parentheses are annotations only; actual filenames are the slug to the right of the tag.

```
docs/superpowers/specs/
├── minos-architecture-and-mvp-design.md     ← this spec
├── codex-app-server-integration.md          (P1)
├── pty-agent-claude-gemini.md               (P1)
├── streaming-chat-ui.md                     (P1)
├── launchd-and-android.md                   (P1.5)
├── release-pipeline.md                      (P1.5)
├── end-to-end-encryption.md                 (P2)
├── git-and-workspace-ops.md                 (P2)
└── cloud-relay-and-push.md                  (P3)
```

Filenames are date-free per project convention.

---

## 12. ADR Index

| # | Topic |
|---|---|
| 0001 | Monorepo layout: Cargo workspace + `apps/` + `crates/` (vs per-platform top-level, vs polyrepo) |
| 0002 | Mobile stack: Flutter + flutter_rust_bridge (vs Telegram-fork twin native) |
| 0003 | Rust Clean Arch deviation: crate-bordered hexagonal (vs four-layer onion folders) |
| 0004 | Wire protocol: JSON-RPC 2.0 over WebSocket (vs CBOR / protobuf) |
| 0005 | No E2EE in MVP: Tailscale-only transport security |
| 0006 | Logging: `mars-xlog` from peterich-rs/xlog-rs (dogfooding our own logger) |
