# Minos · macOS App + UniFFI — Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-21 |
| Owner | fannnzhang |
| Parent spec | `docs/superpowers/specs/minos-architecture-and-mvp-design.md` |
| Implements | Plan 02 of the MVP roadmap (§11 of parent spec) |
| Predecessor | Plan 01 — Rust core + monorepo scaffold (`docs/superpowers/plans/01-rust-core-and-monorepo-scaffold.md`) |

---

## 1. Context

Plan 01 delivered the nine Rust crates compiling, `mars-xlog` logging wired on the daemon side, an in-process E2E test exercising `pair → list_clis → disconnect`, and a `minos-ffi-uniffi` **skeleton** (a single `ping()` sentinel). `apps/macos/` is an empty `.gitkeep` placeholder.

Plan 02 turns that skeleton into a shippable macOS `MenuBarExtra` app, backed by a fully-wired UniFFI shim over `minos-daemon::DaemonHandle`. After plan 02 lands, a user can install `Minos.app`, see a status-bar icon, click "Show QR," and view a valid pairing QR rendered in the UI — all without any iOS / Flutter client existing (that is plan 03's responsibility).

This document is the design contract plan 02 implements. It only covers the macOS side; the mobile side is plan 03.

---

## 2. Goals and Non-Goals

### 2.1 In scope

1. `minos-ffi-uniffi` fully wired — `DaemonHandle` + domain types + `MinosError` + async + event stream visible to Swift through UniFFI 0.31 proc-macro mode.
2. `cargo xtask gen-uniffi` produces Swift sources into `apps/macos/Minos/Generated/`; `cargo xtask build-macos` produces a universal static library at `target/xcframework/libminos_ffi_uniffi.a` (arm64 + x86_64 via `lipo`).
3. `apps/macos/Minos.app` — SwiftUI `MenuBarExtra` app, no Dock icon (`LSUIElement = true`), macOS 13+, bundle ID `ai.minos.macos`.
4. Four-layer folder structure under `apps/macos/Minos/` (Presentation / Application / Domain / Infrastructure) matching parent spec §5.3.
5. Logic-only Swift unit tests in `apps/macos/MinosTests/` (XCTest), depending on a local `DaemonDriving` protocol to keep tests decoupled from UniFFI's generated concrete type.
6. `cargo xtask check-all` green end-to-end — Rust fmt / clippy / test plus `xcodegen generate` → `xcodebuild build` → `swiftlint --strict`.
7. XcodeGen-managed project (`apps/macos/project.yml` checked in, generated `.xcodeproj` gitignored).

### 2.2 Out of scope (explicit deferrals)

| Item | Deferred to |
|---|---|
| iOS / Flutter / frb | Plan 03 |
| Real Tailscale device-to-device smoke (MVP 11-box checklist) | After plan 03 |
| Code signing, notarization, DMG, Sparkle auto-update | P1.5 release pipeline |
| LaunchAgent autostart | P1.5 |
| `minos-mobile::KeychainPairingStore` callback-interface wiring | Plan 03 |
| CLI list view / forget-device UI / multi-device view | Plan 03 or plan 04 (when backing data actually exists) |
| Agent runtime (codex `app-server`, claude/gemini PTY) | Plan 04+ |
| SwiftUI Preview snapshot / widget tests / UI automation | Deferred to integration-test phase (not this plan) |

### 2.3 Testing philosophy (binding rule)

Unit tests across Rust, Swift, and (future) Kotlin in this project cover **logic only**. UI, widget, Preview-snapshot, and end-to-end functional tests count as integration tests and are not written in plan 02. The sole test target in `apps/macos/MinosTests/` is logic-layer (`AppState` reducer-style transitions through a mocked `DaemonDriving` protocol).

### 2.4 UI-per-phase rule (binding rule)

Plan 02's UI contains **only** views and controls for states and actions that actually fire in plan 02. No placeholder views for features arriving in plan 03/04. Later phases will freely reshape / rewrite the plan 02 UI; no preservation tax.

Reachable states in plan 02 on a fresh machine (empty `devices.json`):
- `ConnectionState::Disconnected` (boot default)
- `ConnectionState::Pairing` (after user clicks "Show QR")
- Boot-error state (Tailscale not ready, port conflict, etc.)

Unreachable in plan 02 without a real mobile client: `Connected`, `Reconnecting`. These states compile (the enum is visible) but the plan-02 UI does not render a dedicated view for them — the status header just displays them generically using `ConnectionState.displayLabel`. If a stale `devices.json` is present, Pair state may show "Paired · awaiting peer" statically; no Forget-device UI is shipped.

---

## 3. Assumptions from Plan 01

Plan 02 treats the following as already delivered and stable:

- `minos-domain`: `DeviceId`, `PairingToken`, `AgentName/Status/Descriptor`, `ConnectionState`, `PairingState`, `Lang`, `MinosError` — all Serde-round-trippable, with golden tests.
- `minos-pairing`: `Pairing` state machine, `PairingStore` trait, `QrPayload`, `ActiveToken`, `generate_qr_payload`, `QR_TOKEN_TTL = 5min`.
- `minos-cli-detect`: `CommandRunner` port, `RealCommandRunner`, `detect_all`.
- `minos-transport`: `WsServer`, `WsClient`, exponential backoff.
- `minos-daemon`: `DaemonHandle` with `start(cfg)` / `pairing_qr` / `current_state` / `events_stream` (watch::Receiver) / `addr` (SocketAddr) / `discover_tailscale_ip` (instance method) / `forget_device` / `stop` (consuming self); `FilePairingStore`; `logging::init()` + `log_dir()` + `set_debug`.
- `xtask`: `check-all` + `bootstrap` implemented; `gen-uniffi` / `gen-frb` / `build-macos` / `build-ios` stubs.
- `minos-ffi-uniffi`: skeleton with `ping()` sentinel and `uniffi.toml` pointing at `module_name = "MinosCore"`.
- `apps/macos/.gitkeep`, `apps/mobile/.gitkeep` — empty placeholders.
- CI: `.github/workflows/ci.yml` running `cargo xtask check-all` on `ubuntu-latest` (currently only covers Rust; Swift leg added by plan 02).

---

## 4. Architecture

```
┌────────────── apps/macos/Minos.app (single process) ───────────────┐
│ Swift / SwiftUI — four layers under apps/macos/Minos/              │
│                                                                    │
│  Presentation                                                      │
│    ├─ MenuBarView        state header + 3 menu items + quit        │
│    ├─ QRSheet            modal, CoreImage-rendered QR              │
│    └─ StatusIcon         SF Symbol + color, by ConnectionState     │
│                                                                    │
│  Application                                                       │
│    ├─ AppState (@Observable)   connectionState, currentQr,         │
│    │                           bootError, displayError             │
│    └─ ObserverAdapter          ConnectionStateObserver → @MainActor │
│                                                                    │
│  Domain                                                            │
│    ├─ ConnectionState+Display  .displayLabel / .iconName / .color  │
│    └─ MinosError+Display       .userMessage(lang) passthrough      │
│                                                                    │
│  Infrastructure                                                    │
│    ├─ DaemonBootstrap          initLogging + startAutobind + inject │
│    ├─ QRCodeRenderer           QrPayload → CGImage via CIFilter    │
│    └─ DiagnosticsReveal        Finder-reveal today's .xlog         │
│                                                                    │
│                          UniFFI async + callback interface         │
│  ┌──────────────────────────────▼──────────────────────────────┐   │
│  │  libminos_ffi_uniffi.a  (universal arm64 + x86_64 staticlib) │   │
│  │    re-exports → minos-daemon::DaemonHandle (full surface)    │   │
│  └──────────────────────────────┬──────────────────────────────┘   │
│                                │                                    │
│  minos-daemon (tokio, in-process)                                  │
│    ├─ Phase 0 surgery (see §5.1)                                   │
│    ├─ plan 01 transport / pairing / cli-detect                     │
│    └─ mars-xlog writer → ~/Library/Logs/Minos/daemon_*.xlog        │
│                                                                    │
│  WS bind 100.x.y.z:7878..=7882 (awaits peer; unreachable in 02)    │
└────────────────────────────────────────────────────────────────────┘
```

### 4.1 Process model

Single macOS process. SwiftUI scene and tokio runtime coexist via UniFFI. The tokio runtime is initialized lazily inside `DaemonHandle::start_autobind` (spawn-blocking-safe; UniFFI's async support handles the Swift `await` bridge).

### 4.2 Dependency injection

`AppState` depends on a Swift protocol `DaemonDriving` (declared in `Application/DaemonDriving.swift`), not on the UniFFI-generated `DaemonHandle` concrete type. `DaemonHandle` conforms via an extension under `Infrastructure/DaemonHandle+DaemonDriving.swift`. Tests inject a `MockDaemon` that implements the protocol directly.

`AppState` is an `@Observable` reference type (not a `@MainActor` isolated actor) so SwiftUI Views observe its stored properties directly. All writes to `AppState` happen on `@MainActor` via the `ObserverAdapter`.

### 4.3 Logging

Two independent channels:

- **Rust side** (`minos-daemon::logging`) writes mars-xlog binary files to `~/Library/Logs/Minos/daemon_YYYYMMDD.xlog`. `DaemonBootstrap` calls the UniFFI-exported `initLogging()` once at app start.
- **Swift side** uses Apple's `OSLog` with subsystem `ai.minos.macos`, categories per layer (`bootstrap`, `appState`, `view`). Logs surface in Console.app.

No cross-channel bridging. User diagnostics export is Finder-reveal of the raw `.xlog` file (see §7.4).

### 4.4 QR rendering

`QRCodeRenderer` takes a `QrPayload` (UniFFI-generated Swift struct) and renders a QR image via `CIFilter.qrCodeGenerator()`. The encoded payload is the JSON serialization of `QrPayload` (same bytes Rust would send on the wire). No external SPM dependency.

---

## 5. Components

### 5.1 Phase 0 · `minos-daemon` FFI-friendly refactor

| # | Current (plan 01) | Target (plan 02 Phase 0) | Rationale |
|---|---|---|---|
| 1 | `pub struct DaemonHandle { server: Option<WsServer>, … fields }` | `pub struct DaemonHandle { inner: Arc<DaemonInner> }` — all fields live inside `Arc<DaemonInner>`; `DaemonHandle` becomes a transparent wrapper | UniFFI `#[uniffi::Object]` requires `&self`-only methods on `Arc<Self>` |
| 2 | `pub async fn stop(mut self)` — consumes self | `pub async fn stop(&self)` — takes `WsServer` out of an internal `Mutex<Option<WsServer>>`, then awaits shutdown | `consume self` cannot be exported |
| 3 | `pub fn addr(&self) -> SocketAddr` | `pub fn host(&self) -> String` + `pub fn port(&self) -> u16`; also keep `pub(crate) fn addr(&self) -> SocketAddr` for tests | `SocketAddr` is not a UniFFI primitive |
| 4 | `pub async fn discover_tailscale_ip(&self) -> Option<String>` — instance method | Promote to module-level: `pub async fn minos_daemon::discover_tailscale_ip() -> Option<String>` (free function, same body) | Caller needs it **before** `start`, so it cannot depend on `&self` |
| 5 | (none) | **New**: `pub async fn DaemonHandle::start_autobind(mac_name: String) -> Result<Arc<Self>, MinosError>` — internally calls `discover_tailscale_ip()`, loops through ports 7878..=7882 calling `start(cfg)` until one succeeds; maps all-failed case to `MinosError::BindFailed { addr: "100.x.y.z:7878-7882", message: "all ports occupied" }` | Spec §7.4 failure #2 (port retry) belongs in Rust, not Swift |
| 6 | `pub fn events_stream(&self) -> watch::Receiver<ConnectionState>` | Keep (Rust internal). **New**: `pub fn subscribe(&self, observer: Arc<dyn ConnectionStateObserver>) -> Arc<Subscription>` — spawns a tokio task that does `tokio::select!` between `rx.changed()` and a `oneshot::Receiver<()>` (cancellation); each `changed()` → `observer.on_state(*rx.borrow())` | UniFFI cannot directly surface Tokio `watch::Receiver` |

New Rust types in `minos-daemon`:

```rust
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct Subscription {
    cancel_tx: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl Subscription {
    pub fn cancel(&self) {
        if let Some(tx) = self.cancel_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait ConnectionStateObserver: Send + Sync {
    fn on_state(&self, state: ConnectionState);
}
```

Also new (for diagnostics export — see §7.4):

```rust
// minos-daemon/src/logging.rs
pub fn today() -> Result<PathBuf, MinosError> {
    // 1. Flush active mars-xlog writer so today's buffered entries hit disk
    if let Some(handle) = HANDLE.get() {
        handle.flush();  // API exact name resolved in implementation
    }
    // 2. Compute path: log_dir().join(format!("daemon_{YYYYMMDD}.xlog"))
    //    Format matches mars-xlog naming convention.
    // 3. If file does not exist (no logs written today), return StoreIo error.
}
```

### 5.2 `minos-ffi-uniffi` — full wiring

Responsibilities:
- `uniffi::setup_scaffolding!()` (already in place)
- `uniffi::custom_newtype!(DeviceId, Uuid)` + `uniffi::custom_type!(Uuid, String, { lower, try_lift })` — `DeviceId(Uuid)` crosses as a Swift struct wrapping a String UUID
- `uniffi::custom_newtype!(PairingToken, String)` — single-String newtype
- `uniffi::custom_type!(DateTime<Utc>, SystemTime, { lower, try_lift })` — brings `TrustedDevice.paired_at` across (Swift sees `Date`)
- Re-export the subset of `minos-daemon` free functions:

```rust
#[uniffi::export]
pub fn init_logging() -> Result<(), MinosError> { minos_daemon::logging::init() }

#[uniffi::export]
pub fn set_debug(enabled: bool) { minos_daemon::logging::set_debug(enabled) }

#[uniffi::export]
pub fn today_log_path() -> Result<String, MinosError> {
    minos_daemon::logging::today().map(|p| p.to_string_lossy().to_string())
}

#[uniffi::export]
pub async fn discover_tailscale_ip() -> Option<String> {
    minos_daemon::discover_tailscale_ip().await
}
```

Remove the `ping()` sentinel. The previous `build.rs` remains a no-op (UniFFI proc-macro mode does not need a build.rs-driven UDL pass).

### 5.3 UniFFI derive rollout (feature-gated on-type)

Feature flag `uniffi` added to the three crates that host FFI-visible types. No wrapper types — derives live on the source types directly, off by default.

```toml
# crates/minos-domain/Cargo.toml
[features]
uniffi = ["dep:uniffi"]
[dependencies]
uniffi = { workspace = true, optional = true }
```

Same pattern in `minos-pairing` and `minos-daemon`. `minos-ffi-uniffi/Cargo.toml` enables the feature on all three path deps:

```toml
minos-domain  = { path = "../minos-domain",  features = ["uniffi"] }
minos-pairing = { path = "../minos-pairing", features = ["uniffi"] }
minos-daemon  = { path = "../minos-daemon",  features = ["uniffi"] }
```

**Types receiving `#[cfg_attr(feature = "uniffi", derive(…))]` in plan 02** (full DaemonHandle surface — one-shot rollout, `forget_device` / agent types exported too even though plan 02 UI does not consume them; simpler than re-opening the FFI layer in plan 03):

| Crate | Type | UniFFI derive |
|---|---|---|
| `minos-domain` | `DeviceId` | via `custom_newtype!` in shim (no derive on type) |
| `minos-domain` | `AgentName` | `Enum` |
| `minos-domain` | `AgentStatus` | `Enum` |
| `minos-domain` | `AgentDescriptor` | `Record` |
| `minos-domain` | `ConnectionState` | `Enum` |
| `minos-domain` | `PairingState` | `Enum` |
| `minos-domain` | `Lang` | `Enum` |
| `minos-domain` | `ErrorKind` (new in plan 02, see §7.2) | `Enum` |
| `minos-domain` | `MinosError` | `Error` (struct-shaped, not `flat_error`) |
| `minos-pairing` | `PairingToken` | via `custom_newtype!` in shim |
| `minos-pairing` | `QrPayload` | `Record` |
| `minos-pairing` | `TrustedDevice` | `Record` |
| `minos-daemon` | `DaemonHandle` | `Object` |
| `minos-daemon` | `Subscription` | `Object` |
| `minos-daemon` | `ConnectionStateObserver` | callback trait (`with_foreign`) |

`DaemonConfig` stays unannotated — it is not reachable through the plan 02 Swift API (Swift only calls `start_autobind`).

### 5.4 `xtask` additions

| Command | Implementation |
|---|---|
| `cargo xtask build-macos` | `cargo build -p minos-ffi-uniffi --release --target aarch64-apple-darwin` + same for `x86_64-apple-darwin`; `lipo -create …/libminos_ffi_uniffi.a -output target/xcframework/libminos_ffi_uniffi.a`; validate with `lipo -info` (must report both arches) |
| `cargo xtask gen-uniffi` | `cargo build -p minos-ffi-uniffi --release` (host arch, produces `libminos_ffi_uniffi.dylib`); then `uniffi-bindgen-swift generate --library <dylib> --out-dir apps/macos/Minos/Generated/ --module-name MinosCore`; produces `MinosCore.swift`, `MinosCoreFFI.h`, `MinosCoreFFI.modulemap` |
| `cargo xtask gen-xcode` *(new)* | `xcodegen generate --spec apps/macos/project.yml`; generates `apps/macos/Minos.xcodeproj` (gitignored) |
| `cargo xtask check-all` | Append Swift leg: `gen-uniffi` → `gen-xcode` → `xcodebuild -scheme Minos -destination "platform=macOS" -configuration Debug build` → `swiftlint --strict apps/macos` |
| `cargo xtask bootstrap` | Append `brew bundle` with a Brewfile (`xcodegen`, `swiftlint`); `cargo install uniffi-bindgen-swift --locked` |

Check order in `check-all` matters: Rust tests run first; if Rust is broken, Swift leg is skipped to save local iteration time.

### 5.5 `apps/macos/project.yml` (XcodeGen)

```yaml
name: Minos
options:
  bundleIdPrefix: ai.minos
  deploymentTarget:
    macOS: "13.0"
settings:
  base:
    SWIFT_VERSION: "5.10"
    MACOSX_DEPLOYMENT_TARGET: "13.0"
    ENABLE_HARDENED_RUNTIME: NO
    CODE_SIGN_IDENTITY: "-"
    SWIFT_STRICT_CONCURRENCY: complete
targets:
  Minos:
    type: application
    platform: macOS
    sources:
      - Minos
    resources:
      - Minos/Resources/Assets.xcassets
    info:
      path: Minos/Info.plist
      properties:
        LSUIElement: true
        CFBundleDisplayName: Minos
        CFBundleShortVersionString: "0.1.0"
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
    sources: [MinosTests]
    dependencies:
      - target: Minos
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: ai.minos.macos.tests
```

Generated `apps/macos/Minos.xcodeproj` is added to `.gitignore`.

### 5.6 Swift folder layout

```
apps/macos/
├─ project.yml                                (XcodeGen)
├─ Brewfile                                    (xcodegen, swiftlint)
├─ .swiftlint.yml                              (excludes Minos/Generated/)
├─ Minos/
│  ├─ MinosApp.swift                           @main, MenuBarExtra scene
│  ├─ Info.plist
│  ├─ Generated/                               (gitignored) UniFFI output
│  ├─ Presentation/
│  │  ├─ MenuBarView.swift
│  │  ├─ QRSheet.swift
│  │  └─ StatusIcon.swift
│  ├─ Application/
│  │  ├─ AppState.swift
│  │  ├─ DaemonDriving.swift                   protocol
│  │  └─ ObserverAdapter.swift
│  ├─ Domain/
│  │  ├─ ConnectionState+Display.swift
│  │  └─ MinosError+Display.swift
│  ├─ Infrastructure/
│  │  ├─ DaemonBootstrap.swift
│  │  ├─ DaemonHandle+DaemonDriving.swift      extension
│  │  ├─ QRCodeRenderer.swift
│  │  └─ DiagnosticsReveal.swift
│  └─ Resources/
│     └─ Assets.xcassets/
│        ├─ AppIcon.appiconset/                placeholder SF Symbol-derived
│        └─ AccentColor.colorset/
└─ MinosTests/
   ├─ Application/
   │  └─ AppStateTests.swift
   └─ TestSupport/
      └─ MockDaemon.swift                      implements DaemonDriving
```

### 5.7 MenuBar dropdown (final UI surface for plan 02)

```
┌──────────────────────────────────────────────────┐
│  [icon]  Minos                                   │
│                                                  │
│  {ConnectionState.displayLabel(lang: .zh)}       │ ← status header
│  (dim gray sublabel: "{host}:{port}" when ready) │
├──────────────────────────────────────────────────┤
│  显示配对二维码…                                  │ opens QRSheet
│  在 Finder 中显示今日日志…                        │ NSWorkspace reveal
├──────────────────────────────────────────────────┤
│  退出 Minos                                      │ NSApp.terminate
└──────────────────────────────────────────────────┘
```

`StatusIcon` maps `ConnectionState`:

| State | SF Symbol | Tint |
|---|---|---|
| Disconnected | `bolt.circle` | secondary |
| Pairing | `bolt.circle.fill` | accent blue |
| Connected | `bolt.circle.fill` | system green |
| Reconnecting { attempt } | `bolt.circle.fill` | system orange |
| Boot error present | `bolt.circle.trianglebadge.exclamationmark` | system red |

---

## 6. Data Flow

### 6.1 Boot

```
MinosApp.init()
 └─ Task { await DaemonBootstrap.bootstrap(appState) }

DaemonBootstrap.bootstrap:
 1. try? initLogging()                                // Rust xlog up
 2. Logger.app.info("boot start")
 3. let daemon = try await DaemonHandle.startAutobind(macName: hostName())
       // Rust: discover_tailscale_ip → bind 7878..=7882 → state_tx.send(Disconnected)
 4. let adapter = ObserverAdapter { state in
       Task { @MainActor in appState.connectionState = state }
    }
    let sub = daemon.subscribe(observer: adapter)
 5. await MainActor.run {
       appState.daemon = daemon         // DaemonDriving-conforming
       appState.subscription = sub
       appState.connectionState = daemon.currentState()
    }
 Exception path: any `throw` → `await MainActor.run { appState.bootError = e }`
```

`MinosApp.body` renders `MenuBarExtra` immediately; content renders "正在启动…" until `appState.connectionState != nil || appState.bootError != nil`.

### 6.2 Show QR

```
User taps "显示配对二维码…"
 └─ AppState.showQr()
     ├─ let qr = try await daemon.pairingQr()
     │     // Rust: Unpaired → AwaitingPeer, ActiveToken::fresh, state_tx.send(Pairing)
     ├─ self.currentQr = qr
     └─ self.isQrSheetPresented = true

// concurrently, observer callback fires:
observer.on_state(.pairing) → appState.connectionState = .pairing → StatusIcon re-tints

QRSheet body renders:
 ├─ QRCodeRenderer.image(for: qr)   CIFilter.qrCodeGenerator
 ├─ Image(decorative: cgImage, scale: 1)
 ├─ Text("有效期 5 分钟 · 在手机上扫描")
 ├─ Text("{qr.host}:{qr.port}")       dim gray debug info
 ├─ Button("重新生成") → AppState.regenerateQr()   same path as showQr
 └─ Button("关闭") → isQrSheetPresented = false
```

5-minute token TTL: a SwiftUI `TimelineView(.periodic(from: issuedAt, by: 1))` drives an overlay "二维码已过期" after 5 minutes, with a "重新生成" call-to-action. No automatic rotation in plan 02; the user must click.

### 6.3 Connection state propagation

```
Rust code path issuing state_tx.send(new_state)    (pair flow, stop, forget, etc.)
 └─ watch::Receiver.changed() resolves
    └─ subscribe() task: tokio::select! arm → observer.on_state(new_state)
       └─ UniFFI cross → Swift ObserverAdapter.onState(new_state)
          └─ Task { @MainActor in appState.connectionState = new_state }
             └─ SwiftUI @Observable → MenuBarView rerenders
                ├─ StatusIcon (symbol + tint)
                └─ state header label
```

### 6.4 Reveal logs in Finder

```
User taps "在 Finder 中显示今日日志…"
 └─ AppState.revealTodayLog()
     ├─ let path = try todayLogPath()        // UniFFI free function → Rust logging::today()
     ├─ let url = URL(fileURLWithPath: path)
     └─ NSWorkspace.shared.activateFileViewerSelecting([url])

Error path (file does not yet exist → StoreIo):
 └─ appState.displayError = e
    └─ MenuBarView renders ShadToast-style inline banner for 3 seconds
```

### 6.5 Quit

```
User taps "退出 Minos"
 └─ AppState.shutdown() async
     ├─ await daemon?.stop()         // stops WsServer, state_tx.send(Disconnected)
     ├─ subscription?.cancel()       // oneshot, stops observer task
     └─ NSApp.terminate(nil)
```

### 6.6 Boot-failure recovery

`appState.bootError != nil` triggers MenuBarView's error branch:

```
┌──────────────────────────────────────────────────┐
│  [⚠]  Minos · 启动失败                            │
│                                                  │
│  {bootError.userMessage(lang: .zh)}              │
│                                                  │
│  ▼ 详情                                          │
│  {bootError.description}                         │
│                                                  │
│  [重试]                                          │
├──────────────────────────────────────────────────┤
│  在 Finder 中显示今日日志…                        │
│  退出 Minos                                      │
└──────────────────────────────────────────────────┘
```

"重试" calls `DaemonBootstrap.bootstrap(appState)` again; on success the error branch dismisses. "显示配对二维码…" is hidden in this state.

---

## 7. Error Handling (cross-FFI)

### 7.1 Rust → Swift `MinosError` mapping

`MinosError` annotated `#[uniffi::Error]` with **struct-shaped** variants (not `flat_error`) so Swift sees associated values — the UI can display `bindFailed.addr` or `disconnected.reason` when useful:

```swift
enum MinosError: Error {
    case bindFailed(addr: String, message: String)
    case connectFailed(url: String, message: String)
    case disconnected(reason: String)
    case pairingTokenInvalid
    case pairingStateMismatch(actual: PairingState)
    case deviceNotTrusted(deviceId: String)
    case storeIo(path: String, message: String)
    case storeCorrupt(path: String, message: String)
    case cliProbeTimeout(bin: String, timeoutMs: UInt64)
    case cliProbeFailed(bin: String, message: String)
    case rpcCallFailed(method: String, message: String)
}
```

### 7.2 `MinosError.userMessage(lang:)` bridge

UniFFI's `#[uniffi::Error]` variants can be thrown and caught across FFI but **cannot be passed back into Rust as function arguments** — the `Error` derive gives the enum Swift's `Error` conformance, not a value-passing shape. To keep the localized strings single-sourced on the Rust side, plan 02 introduces a parallel payload-free enum `ErrorKind` that mirrors `MinosError`'s discriminants, and moves the string table to `ErrorKind::user_message`:

```rust
// minos-domain/src/error.rs (added alongside MinosError)
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    BindFailed, ConnectFailed, Disconnected,
    PairingTokenInvalid, PairingStateMismatch,
    DeviceNotTrusted, StoreIo, StoreCorrupt,
    CliProbeTimeout, CliProbeFailed, RpcCallFailed,
}

impl ErrorKind {
    pub fn user_message(self, lang: Lang) -> &'static str {
        // 22 entries (11 variants × 2 langs) — verbatim moved from the
        // existing MinosError::user_message string table
    }
}

impl MinosError {
    pub fn kind(&self) -> ErrorKind { /* 11 trivial matches, no payload */ }
    pub fn user_message(&self, lang: Lang) -> &'static str {
        self.kind().user_message(lang)
    }
}
```

UniFFI shim exposes a free function that accepts the payload-free `ErrorKind`:

```rust
// minos-ffi-uniffi/src/lib.rs
#[uniffi::export]
pub fn kind_message(kind: ErrorKind, lang: Lang) -> String {
    kind.user_message(lang).to_string()
}
```

Swift resolves the localized string via a trivial discriminant-only switch:

```swift
// Domain/MinosError+Display.swift
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
        MinosCore.kindMessage(kind: kind, lang: lang)
    }
}
```

Only the `MinosError → ErrorKind` switch is duplicated across Rust and Swift; the **string table stays single-sourced** in Rust's `ErrorKind::user_message`. Adding a new `MinosError` variant requires:
1. Add the variant to `MinosError`
2. Add the corresponding variant to `ErrorKind`
3. Add one arm to `MinosError::kind`
4. Add two arms (zh + en) to `ErrorKind::user_message`
5. Add one arm to Swift's `MinosError.kind` switch

### 7.3 UI error policy

Three buckets, each with a deliberate display:

| Trigger | Swift handler | Display |
|---|---|---|
| `bootstrap()` throws | `appState.bootError = e` | Full error branch in MenuBarView (§6.6) |
| `showQr()` / `revealTodayLog()` / UI-invoked async throws | `appState.displayError = e` | Inline banner 3s, then auto-dismiss |
| observer callback — not applicable; callbacks are infallible | — | — |

### 7.4 Errors plan 02 cannot trigger

The following `MinosError` variants are declared (Swift sees the case) but not exercised in plan 02 flows:

- `ConnectFailed`, `Disconnected`, `DeviceNotTrusted`, `RpcCallFailed` (mobile-side / reconnect)
- `StoreCorrupt` (needs a real session having written `devices.json` first)
- `CliProbeTimeout`, `CliProbeFailed` (CLI probing happens inside `list_clis`, which only a mobile peer invokes)

These are not filtered out of the Swift mapping — users may trigger them by loading stale fixtures. UI handlers default to the displayError banner.

---

## 8. Testing Strategy

### 8.1 Rust tests (Phase 0 additions only; plan 01 tests untouched)

| Crate | New tests | Technique |
|---|---|---|
| `minos-daemon` | `start_autobind` iterates 7878..=7882 and returns first successful addr; `start_autobind` returns `BindFailed` when all 5 occupied | `#[tokio::test]` + pre-bind decoy `TcpListener`s to occupy ports |
| `minos-daemon` | `stop(&self)` idempotent — calling twice does not panic | `#[tokio::test]` |
| `minos-daemon` | `Subscription::cancel` stops observer task (watcher confirms no further `on_state` after cancel) | `#[tokio::test]` with test double implementing `ConnectionStateObserver` |
| `minos-daemon` | `logging::today()` returns a path; path exists after emitting at least one log record | `#[test]` with `MINOS_LOG_DIR` tempdir |

### 8.2 UniFFI shim tests

`minos-ffi-uniffi` has no logic to unit-test. Build-phase verification only:
- `cargo build -p minos-ffi-uniffi --release` succeeds (covered by `check-all`)
- `cargo xtask gen-uniffi` produces a `MinosCore.swift` that contains strings `"public class DaemonHandle"`, `"public enum ConnectionState"`, `"public struct QrPayload"`, `"public protocol ConnectionStateObserver"` (simple grep smoke in `xtask`)

### 8.3 Swift logic tests (`MinosTests/Application/AppStateTests.swift`)

Covered scenarios (no UI assertions, no XCUITest):

| Scenario | Setup | Assertion |
|---|---|---|
| Observer callback updates `connectionState` | `MockDaemon` whose `subscribe` stores the observer; test drives `observer.on_state(.connected)` | `appState.connectionState == .connected` |
| `showQr()` success | `MockDaemon.pairingQr` returns a fixture `QrPayload` | `appState.currentQr != nil`, `appState.isQrSheetPresented == true` |
| `showQr()` throws | `MockDaemon.pairingQr` throws `MinosError.storeIo(…)` | `appState.displayError != nil`, `appState.currentQr == nil` |
| `regenerateQr()` | Same path as showQr | second `currentQr` differs from first |
| `bootError` hides showQr affordance | `appState.bootError = .bindFailed(…)` | `appState.canShowQr == false` |
| `shutdown()` calls `daemon.stop()` and `subscription.cancel()` | `MockDaemon` records calls | Both call-counts == 1 |

Swift unit-test target does **not** import or link the UniFFI static lib directly — `DaemonDriving` + `MockDaemon` make it self-contained. `MinosCore` is still linked because XcodeGen places both under the same target graph; tests simply never instantiate UniFFI types.

### 8.4 `xtask check-all` Swift leg

```
gen-uniffi
  └─ validate Generated/ contains expected public symbols (grep smoke)
gen-xcode
xcodebuild -scheme Minos -destination 'platform=macOS' -configuration Debug build
xcodebuild -scheme MinosTests -destination 'platform=macOS' test
swiftlint --strict apps/macos
```

If any step fails, `check-all` exits nonzero.

### 8.5 CI additions

`.github/workflows/ci.yml` adds a second job on `macos-14`:

```yaml
swift:
  runs-on: macos-14
  steps:
    - uses: actions/checkout@v4
    - run: brew bundle --file=apps/macos/Brewfile
    - run: cargo install uniffi-bindgen-swift --locked
    - run: cargo xtask build-macos
    - run: cargo xtask gen-uniffi
    - run: cargo xtask gen-xcode
    - run: xcodebuild -scheme Minos -destination "platform=macOS" -configuration Debug build
    - run: xcodebuild -scheme MinosTests -destination "platform=macOS" test
    - run: swiftlint --strict apps/macos
```

No code signing, no Xcode-simulator build (not applicable on macOS), no app bundling step.

### 8.6 Done criteria

Plan 02 is "done" when all of the following are true **simultaneously**:

1. `cargo xtask check-all` green on a fresh clone (after `cargo xtask bootstrap`)
2. `cargo xtask build-macos` produces `target/xcframework/libminos_ffi_uniffi.a` with `lipo -info` reporting `arm64 x86_64`
3. `cargo xtask gen-uniffi` produces `apps/macos/Minos/Generated/{MinosCore.swift, MinosCoreFFI.h, MinosCoreFFI.modulemap}` with no errors
4. `xcodebuild -scheme Minos build` produces `Minos.app` with no warnings (excluding UniFFI-generated code warnings, which are allowlisted via SwiftLint excludes)
5. `xcodebuild -scheme MinosTests test` passes all `AppStateTests`
6. Manual sanity on maintainer workstation: app launches, MenuBar icon appears, all 3 non-quit menu items functional, Finder opens on reveal, Quit exits cleanly, `~/Library/Logs/Minos/daemon_YYYYMMDD.xlog` is non-empty

Items 1–5 are automated and CI-enforced. Item 6 is a pre-merge check documented in the plan's closing checklist; it is not scripted.

---

## 9. Tooling Notes

### 9.1 `uniffi-bindgen-swift`

Plan 01 pinned UniFFI 0.31 (see `minos-ffi-uniffi/Cargo.toml`). The matching bindgen tool is `uniffi-bindgen-swift` (a separate crate packaged by the UniFFI org). `cargo xtask bootstrap` installs it from crates.io:

```bash
cargo install uniffi-bindgen-swift --locked
```

Command invocation:

```bash
uniffi-bindgen-swift \
    --library target/release/libminos_ffi_uniffi.dylib \
    --module-name MinosCore \
    --out-dir apps/macos/Minos/Generated/
```

Note that `uniffi-bindgen-swift` operates on a built dylib (library-based generation), not the UDL file. `cargo xtask gen-uniffi` builds the library first.

### 9.2 XcodeGen + swiftlint pinning

`apps/macos/Brewfile`:

```ruby
brew "xcodegen"
brew "swiftlint"
```

`apps/macos/.swiftlint.yml` excludes `Minos/Generated/` (UniFFI-generated code is allowed to fail every lint rule).

### 9.3 Swift toolchain

Swift 5.10 via the Xcode 15.4+ toolchain on macOS 14+. No SwiftPM dependencies; all functionality from the stdlib + SwiftUI + AppKit (`NSWorkspace`) + CoreImage.

---

## 10. Out of Scope (explicit, reiterated)

| Item | Why deferred | Target phase |
|---|---|---|
| iOS app + frb shim | Separate tech stack, symmetric plan | Plan 03 |
| Real pair-end-to-end across Tailscale | Needs mobile client | Post-plan-03 |
| Forget-device UI | No trusted device exists in plan 02 alone | Plan 03 |
| CLI list view | No peer to trigger `list_clis` | Plan 03 or 04 |
| Multi-device management view | Single-pair enforced in MVP | P2 |
| Code signing, notarization, DMG | Not a dev-tree concern in MVP | P1.5 release |
| Sparkle auto-update | Same | P1.5 |
| LaunchAgent autostart | Manual launch sufficient for MVP | P1.5 |
| SwiftUI Preview snapshot tests, widget tests, XCUITest | Ruled out by testing-philosophy rule | (eventual integration-test phase) |
| Agent runtime (codex app-server, PTY, chat UI) | Huge scope | P1 |

---

## 11. Open Questions (resolved before plan-write)

None remaining. Six questions were posed and resolved during brainstorming:

1. Plan 02 scope boundaries → answered §2.
2. Xcode project management → XcodeGen (§5.5).
3. Event-stream FFI shape → callback interface + Subscription (§5.1).
4. Daemon FFI-friendly refactor → 6 changes at Phase 0 (§5.1).
5. UniFFI derive placement → on original types with `uniffi` feature flag (§5.3).
6. Build output + timing → universal static `.a` + explicit `xtask` pre-step (§5.4).

Plus three follow-ups:
- UI scope per phase: plan 02 UI is narrow (no CLIListView, no DevicesView, no Forget button) — §2.4.
- Diagnostics export: Finder-reveal the raw `.xlog` file via `today_log_path()` — §5.1 / §6.4.
- `DaemonDriving` protocol shim for Swift test mocking — §4.2.

---

## 12. File Inventory (what plan 02 creates / modifies)

**New files:**

```
crates/minos-daemon/src/subscription.rs                      Subscription + ConnectionStateObserver
apps/macos/project.yml                                       XcodeGen spec
apps/macos/Brewfile                                          xcodegen, swiftlint
apps/macos/.swiftlint.yml                                    excludes Generated/
apps/macos/Minos/MinosApp.swift                              @main
apps/macos/Minos/Info.plist
apps/macos/Minos/Presentation/MenuBarView.swift
apps/macos/Minos/Presentation/QRSheet.swift
apps/macos/Minos/Presentation/StatusIcon.swift
apps/macos/Minos/Application/AppState.swift
apps/macos/Minos/Application/DaemonDriving.swift
apps/macos/Minos/Application/ObserverAdapter.swift
apps/macos/Minos/Domain/ConnectionState+Display.swift
apps/macos/Minos/Domain/MinosError+Display.swift
apps/macos/Minos/Infrastructure/DaemonBootstrap.swift
apps/macos/Minos/Infrastructure/DaemonHandle+DaemonDriving.swift
apps/macos/Minos/Infrastructure/QRCodeRenderer.swift
apps/macos/Minos/Infrastructure/DiagnosticsReveal.swift
apps/macos/Minos/Resources/Assets.xcassets/                  AppIcon + AccentColor
apps/macos/MinosTests/Application/AppStateTests.swift
apps/macos/MinosTests/TestSupport/MockDaemon.swift
docs/adr/0007-xcodegen-for-macos-project.md                  one-page justification
```

**Modified files:**

```
.gitignore                                                   add apps/macos/Minos.xcodeproj and Brewfile.lock.json
.github/workflows/ci.yml                                     add swift job on macos-14
Cargo.toml                                                   (none unless uniffi bump needed)
crates/minos-domain/Cargo.toml                               add uniffi feature
crates/minos-domain/src/{agent,connection,ids,pairing_state}.rs  add cfg_attr derives
crates/minos-domain/src/error.rs                             add ErrorKind enum + string-table move; MinosError derives uniffi::Error
crates/minos-pairing/Cargo.toml                              add uniffi feature
crates/minos-pairing/src/{store,token}.rs                    add cfg_attr derives (QrPayload/TrustedDevice Records; PairingToken via custom_newtype in shim)
crates/minos-daemon/Cargo.toml                               add uniffi feature
crates/minos-daemon/src/lib.rs                               export subscription module
crates/minos-daemon/src/handle.rs                            Phase 0 refactor (Arc<Inner>, stop(&self), host/port, start_autobind)
crates/minos-daemon/src/tailscale.rs                         hoist discover_ip to pub free fn
crates/minos-daemon/src/logging.rs                           add today()
crates/minos-ffi-uniffi/src/lib.rs                           remove ping, add custom_types, re-export free functions
crates/minos-ffi-uniffi/Cargo.toml                           enable uniffi features on path deps
xtask/src/main.rs                                            implement build-macos / gen-uniffi / gen-xcode; extend check-all
xtask/src/bootstrap.rs (or equivalent)                       add brew bundle + uniffi-bindgen-swift install
README.md                                                    update "Status" to reflect plan 02 readiness
```

**Deleted:**

```
(nothing — plan 01 structure is stable)
```

---

## 13. ADR

One new ADR accompanies plan 02:

- `docs/adr/0007-xcodegen-for-macos-project.md` — why XcodeGen over hand-authored `.xcodeproj` or Tuist for this repo.
