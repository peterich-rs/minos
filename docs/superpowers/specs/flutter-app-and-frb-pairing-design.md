# Minos · Flutter App + frb Pairing Bring-up — Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-22 |
| Owner | fannnzhang |
| Parent spec | `docs/superpowers/specs/minos-architecture-and-mvp-design.md` |
| Target plan | `docs/superpowers/plans/03-flutter-app-and-frb-pairing.md` (to be written) |

---

## 1. Context

Plans 01 and 02 delivered the Rust workspace, the macOS `MenuBarExtra` app, and the UniFFI bridge. `cargo xtask check-all` is green; the macOS app can generate and display a pairing QR. The iOS side of the pipeline remains empty — `apps/mobile/` contains only a `.gitkeep`, and `minos-ffi-frb` is a stub that exists solely to reserve the crate name.

This spec scaffolds the iOS Flutter application, wires `minos-mobile::MobileClient` across the `flutter_rust_bridge` (frb v2) boundary, and drives one end-to-end pairing flow: **real iPhone scans the macOS-rendered QR code, completes the `pair` JSON-RPC over a Tailscale-backed WebSocket, and surfaces the `Connected` state in the Flutter UI**.

The scope is deliberately narrower than the architecture spec's "MVP iOS app". The `list_clis` round-trip, auto-reconnect loop, Keychain-backed `PairingStore`, and the "Forget this Mac" affordance are all deferred to a Tier B spec so that this plan stays focused on proving the FFI + WebSocket plumbing.

---

## 2. Goals

### 2.1 Tier A (this spec)

1. Scaffold a Flutter project under `apps/mobile/` with iOS + Android targets (only iOS is validated; `android/` is kept intact for P1.5).
2. Fill in `minos-ffi-frb` with frb v2 `#[frb(...)]` annotations over `minos-mobile::MobileClient`, exposing a minimal API surface to Dart.
3. Implement a two-page Flutter UI (`PairingPage` + `HomePage`) and the supporting Riverpod state, backed by `shadcn_ui` components and `mobile_scanner` QR capture.
4. Persistently pass real-device smoke checklist §8.4 items 1–5 from the parent spec: Mac-side Tailscale / QR / MenuBar ready; iOS Tailscale ready; iOS scan → `Connected to {MacName}` within 5 s.
5. Extend `cargo xtask check-all` and CI so Dart analyze / format / test plus frb-codegen-drift checks run automatically.

### 2.2 Non-goals (explicit Tier B / later)

- Dart consumption of `list_clis`; rendering CLI rows on `HomePage`.
- Auto-reconnect loop on the iOS side (1 s → 30 s exponential backoff).
- `flutter_secure_storage`-backed Dart `PairingStore` implementation through a frb callback.
- "Forget this Mac" UI and the corresponding Mac-side revocation propagation.
- English / i18n surface in the UI (en strings are already present in Rust's localization table and remain latent).
- Android real-device validation; TestFlight / notarization / DMG publishing; Xcode iOS builds in CI.

---

## 3. Tech Stack & Pinned Versions

| Layer | Choice | Version pin policy |
|---|---|---|
| Flutter SDK | `>=3.41.0 <4.0.0` | `pubspec.yaml environment` |
| Dart SDK | `^3.6.0` | Same |
| flutter_rust_bridge | `^2.0.0` (resolved to current `2.x` at scaffold time) | Same; tracked loosely per MVP spec §3 |
| flutter_riverpod | `^2.5.0` + `riverpod_annotation` `^2.3.0` + `riverpod_generator` `^2.4.0` + `riverpod_lint` `^2.3.0` + `custom_lint` `^0.6.0` | Same |
| shadcn_ui | `^0.30.0` (latest at scaffold) | Same |
| mobile_scanner | `^5.0.0` | Same |
| permission_handler | `^11.0.0` | Same |
| path_provider | `^2.1.0` | Same |
| mocktail | `^1.0.0` | Dev-only |
| iOS deployment target | iOS 16 (inherited from MVP spec §3) | `apps/mobile/ios/Runner.xcodeproj` post-scaffold patch |
| Bundle ID | `ai.minos.mobile` | Mirrors macOS `ai.minos.macos` |

**Version drift policy**: Values above are accurate as of 2026-04-22. If `flutter pub add` or `cargo add` resolves higher minor versions at scaffold time, prefer the resolved version unless compilation fails.

---

## 4. Architecture

### 4.1 iOS process topology

```
  iPhone (Tailscale 100.64.0.42)
  ┌──────────── Minos.app (Flutter + Rust, single process) ────────────┐
  │ Dart / Flutter                                                     │
  │  ┌─ presentation/  (ShadApp root + _Router + 3 pages)              │
  │  ├─ application/   (4 Riverpod providers)                          │
  │  ├─ domain/        (MinosCore protocol + display extensions)       │
  │  └─ infrastructure/(MinosCore facade — sole owner of frb handle)   │
  │                             │                                      │
  │                             │ frb v2 generated bindings            │
  │  ┌──────────────────────────▼─────────────────────────┐            │
  │  │ lib/src/rust/  (generated; checked in)              │           │
  │  └──────────────────────────┬─────────────────────────┘            │
  │                             │ dart:ffi + frb ABI                   │
  │  ┌──────────────────────────▼─────────────────────────┐            │
  │  │ libminos_ffi_frb.a  (staticlib, iOS)                │           │
  │  │   #[frb(opaque / mirror)] wrappers                  │           │
  │  └──────────────────────────┬─────────────────────────┘            │
  │                             │ in-process Rust calls                │
  │  ┌──────────────────────────▼─────────────────────────┐            │
  │  │ minos-mobile (tokio runtime)                        │           │
  │  │   MobileClient + InMemoryPairingStore               │           │
  │  └──────────────────────────┬─────────────────────────┘            │
  └─────────────────────────────┼──────────────────────────────────────┘
                                │ JSON-RPC 2.0 / WebSocket
                                │ over Tailscale WireGuard tunnel
                 ┌──────────────▼──────────────┐
                 │ macOS Minos.app (100.64.0.10)│
                 │ minos-daemon WS server :7878 │
                 └──────────────────────────────┘
```

### 4.2 FFI boundary discipline

The hexagonal border from spec §4.3 is preserved:

- **`minos-mobile` does not depend on `flutter_rust_bridge`.** It stays a pure composition root.
- **`minos-ffi-frb` is the adapter.** It is the only crate with frb-specific annotations (`#[frb(opaque)]`, `#[frb(mirror)]`, `StreamSink<T>`).
- **Dart does not consume frb types directly.** A `MinosCore` facade (and its `MinosCoreProtocol` abstract class) interposes, so Riverpod providers and unit tests depend on the local protocol — not on generated frb classes. This mirrors plan 02's `SubscriptionHandle` / `DaemonDriving` pattern on the Swift side.

### 4.3 FFI data shapes (precise list)

| Direction | Shape | Rationale |
|---|---|---|
| Dart → Rust | `String self_name` (construct); `String qr_json` (pair); `String log_dir` (init logging) | Raw strings cross frb cleanly; avoids shipping `QrPayload` or `Arc<dyn PairingStore>` |
| Rust → Dart | Mirrored `ConnectionState`, `PairResponse`, `MinosError`, `ErrorKind`, `Lang` | Dart reasons about variants natively |
| Rust → Dart (stream) | `StreamSink<ConnectionState>` → Dart `Stream<ConnectionState>` | Bridges the existing `tokio::sync::watch::Receiver` without leaking watch semantics to Dart |

---

## 5. Rust-side Components

### 5.1 `minos-domain` additions

None required. The existing `MinosError`, `ErrorKind`, `Lang`, and `ConnectionState` already cover every case Tier A needs. A malformed scanned QR is represented as `MinosError::StoreCorrupt { path: "qr_payload", message }` — semantically "a payload we expected to deserialize was invalid". This avoids growing the error enum during a UI-scoped plan.

### 5.2 `minos-mobile` additions

Two FFI-friendly constructors / entry points on `MobileClient`:

```rust
// crates/minos-mobile/src/client.rs — new methods on impl MobileClient

/// FFI-friendly constructor. Uses an in-memory PairingStore — suitable for
/// plan 03 where Dart does not implement the store. A future constructor
/// `new_with_dart_store` will be added when Keychain persistence lands.
#[must_use]
pub fn new_with_in_memory_store(self_name: String) -> Self {
    Self::new(Arc::new(crate::InMemoryPairingStore::new()), self_name)
}

/// Scan / paste accepts raw JSON. We deserialize inside Rust so `QrPayload`
/// never crosses the FFI boundary.
#[allow(clippy::missing_errors_doc)]
pub async fn pair_with_json(&self, qr_json: String) -> Result<PairResponse, MinosError> {
    let qr: QrPayload = serde_json::from_str(&qr_json).map_err(|e| MinosError::StoreCorrupt {
        path: "qr_payload".into(),
        message: e.to_string(),
    })?;
    self.pair_with(qr).await
}
```

Existing `events_stream(&self) -> watch::Receiver<ConnectionState>` is reused by the frb adapter — `minos-mobile` is unchanged with respect to the stream API.

### 5.3 `minos-ffi-frb` as adapter

`crates/minos-ffi-frb/` transitions from the placeholder in `lib.rs` to a real adapter with the following surface (exact macro syntax is determined by frb v2's supported attributes at implementation time; the semantics below are binding):

```rust
// crates/minos-ffi-frb/src/api/minos.rs — new file; frb codegen's rust_input
use flutter_rust_bridge::{frb, StreamSink};
use minos_domain::{ConnectionState, ErrorKind, Lang, MinosError};
use minos_protocol::PairResponse;

#[frb(mirror(ConnectionState))]
pub enum _ConnectionState { Disconnected, Pairing, Connected, Reconnecting }

#[frb(mirror(PairResponse))]
pub struct _PairResponse { pub ok: bool, pub mac_name: String }

#[frb(mirror(ErrorKind))]
pub enum _ErrorKind { /* 11 variants mirrored 1:1 */ }

#[frb(mirror(Lang))]
pub enum _Lang { Zh, En }

// MinosError is mirrored via frb(mirror); variant-level mirror lets Dart
// pattern-match on which failure occurred.

#[frb(opaque)]
pub struct MobileClient(minos_mobile::MobileClient);

impl MobileClient {
    #[frb(sync)]
    pub fn new(self_name: String) -> Self {
        Self(minos_mobile::MobileClient::new_with_in_memory_store(self_name))
    }

    pub async fn pair_with_json(&self, qr_json: String) -> Result<PairResponse, MinosError> {
        self.0.pair_with_json(qr_json).await
    }

    #[frb(sync)]
    pub fn current_state(&self) -> ConnectionState {
        self.0.current_state()
    }

    /// Bridges the watch::Receiver to a Dart Stream. Terminates when the
    /// sink closes.
    pub fn subscribe_state(&self, sink: StreamSink<ConnectionState>) {
        let mut rx = self.0.events_stream();
        tokio::spawn(async move {
            if sink.add(*rx.borrow_and_update()).is_err() {
                return;
            }
            while rx.changed().await.is_ok() {
                if sink.add(*rx.borrow()).is_err() {
                    break;
                }
            }
        });
    }
}

/// Initialize logging from the Dart-supplied sandbox path (resolved via
/// path_provider).
pub fn init_logging(log_dir: String) -> Result<(), MinosError> {
    minos_mobile::logging::init(std::path::Path::new(&log_dir))
}

/// Localization bridge — Dart computes `ErrorKind` from a mirrored
/// `MinosError` variant locally, then calls this to get the user-facing
/// string. Mirrors `crates/minos-ffi-uniffi::kind_message`.
#[frb(sync)]
pub fn kind_message(kind: ErrorKind, lang: Lang) -> String {
    kind.user_message(lang).to_string()
}
```

`Cargo.toml` additions for `minos-ffi-frb`:

```toml
[dependencies]
flutter_rust_bridge = "2"
tokio = { version = "1", features = ["rt"] }
minos-mobile = { path = "../minos-mobile" }
minos-domain  = { path = "../minos-domain" }
minos-protocol = { path = "../minos-protocol" }

[lib]
crate-type = ["staticlib", "cdylib"]  # staticlib for iOS (App Store), cdylib for Android
```

---

## 6. Dart / Flutter Components

### 6.1 Project scaffold

```bash
flutter create \
  --org ai.minos --project-name minos \
  --platforms ios,android \
  --template app \
  apps/mobile
```

Post-scaffold patches:
- `apps/mobile/ios/Runner/Info.plist`: add `NSCameraUsageDescription = "Minos 需要使用相机扫描 Mac 上的配对二维码"` (zh default); iOS deployment target set to 16.
- Delete `flutter create`'s counter-app `main.dart`, `test/widget_test.dart`, and `home_page.dart` defaults.
- Set Bundle ID in Xcode project to `ai.minos.mobile`.

### 6.2 Directory structure

```
apps/mobile/
├── pubspec.yaml
├── analysis_options.yaml          # incl. riverpod_lint + custom_lint
├── flutter_rust_bridge.yaml       # frb config (see §10.2)
├── ios/Runner/Info.plist
├── android/                       # scaffolded, not further validated
├── lib/
│   ├── main.dart
│   ├── src/rust/                  # frb generated artifact; checked in
│   ├── infrastructure/
│   │   ├── minos_core.dart        # frb facade, sole owner of MobileClient
│   │   └── app_paths.dart         # path_provider wrapper
│   ├── application/
│   │   └── minos_providers.dart   # 4 Riverpod providers (codegen)
│   ├── domain/
│   │   ├── minos_core_protocol.dart   # abstract class consumed by providers
│   │   └── minos_error_display.dart   # ErrorKind computation + userMessage helper
│   └── presentation/
│       ├── app.dart                # ShadApp + _Router
│       ├── pages/
│       │   ├── pairing_page.dart
│       │   ├── home_page.dart
│       │   └── permission_denied_page.dart
│       └── widgets/
│           ├── qr_scanner_view.dart
│           └── debug_paste_qr_sheet.dart   # kDebugMode-gated
└── test/
    └── unit/
        ├── pairing_controller_test.dart
        └── minos_error_display_test.dart
        # test/widget/ and test/integration/ deliberately absent in Tier A
        # per the logic-only unit-test convention (see §9.2)
```

### 6.3 Riverpod providers (all codegen via `@riverpod`)

| Provider | Shape | Depends on | Responsibility |
|---|---|---|---|
| `minosCoreProvider` | `@Riverpod(keepAlive: true)` returning `MinosCoreProtocol` | — | Holds the `MinosCore` instance. Default body `throw UnimplementedError` — must be overridden in `main()` via `overrideWithValue`. |
| `connectionStateProvider` | `@Riverpod(keepAlive: true)` returning `Stream<ConnectionState>` | `minosCore` | Exposes `MobileClient.subscribeState()`; `_Router` reads it. |
| `cameraPermissionProvider` | `@riverpod` AsyncNotifier | — | Wraps `permission_handler`: `.check()`, `.request()`, `.openSettings()`. |
| `pairingControllerProvider` | `@riverpod` `AsyncNotifier<PairResponse?>` (initial `AsyncData(null)`) | `minosCore` | `submit(String qrJson)` drives `loading → data(PairResponse) / error(MinosError)`. UI uses `ref.listen` for toast, `ref.watch` for overlay, `HomePage` reads `.valueOrNull` for the `mac_name`. |

### 6.4 Pages & widgets

- **`app.dart`** — `ShadApp.materialApp(themeMode: ThemeMode.system, …)` with both light and dark `ShadThemeData` defined; home is `_Router`.
- **`_Router`** watches `connectionStateProvider`:
  - `Disconnected | Pairing | Reconnecting` → `PairingPage` (with `Pairing` state rendering a loading overlay).
  - `Connected` → `HomePage`.
- **`PairingPage`**:
  - On mount: `ref.read(cameraPermissionProvider.notifier).check()`.
  - Permission states route to `QrScannerView` (granted) or `PermissionDeniedPage` (denied / permanentlyDenied).
  - `kDebugMode` appends a floating action button that opens `DebugPasteQrSheet` — a `ShadSheet` with a multiline `TextField` and submit button that calls `pairingController.submit(pasted)`.
- **`QrScannerView`** wraps `MobileScanner`; `onDetect` calls `pairingController.submit(firstBarcodeRaw)`.
- **`PermissionDeniedPage`** — short explanation + "打开设置" button that calls `permission_handler.openAppSettings()`.
- **`HomePage`** — Tier A is intentionally minimal: a `ShadCard` titled "已连接" with subtitle `${response.macName}`, read from `ref.watch(pairingControllerProvider).valueOrNull`. Non-null is invariant because `_Router` only routes to `HomePage` when state is `Connected`, which is reached only after `pair_with_json` resolves. No further actions, no device id footer (would require exposing `device_id` across FFI — out of Tier A scope).

### 6.5 `MinosCore` facade + protocol shim

```dart
// lib/domain/minos_core_protocol.dart
abstract class MinosCoreProtocol {
  Future<PairResponse> pairWithJson(String qrJson);
  Stream<ConnectionState> get states;
  ConnectionState get current;
}

// lib/infrastructure/minos_core.dart
class MinosCore implements MinosCoreProtocol {
  MinosCore._(this._rust);

  final MobileClient _rust;            // frb-generated opaque handle

  static Future<MinosCore> init({required String selfName, required String logDir}) async {
    await RustLib.init();
    await api.initLogging(logDir: logDir);
    final client = MobileClient(selfName: selfName);
    return MinosCore._(client);
  }

  @override
  Future<PairResponse> pairWithJson(String qrJson) =>
      _rust.pairWithJson(qrJson: qrJson);

  @override
  Stream<ConnectionState> get states => _rust.subscribeState();

  @override
  ConnectionState get current => _rust.currentState();
}
```

`PairResponse` caching is delegated to `pairingControllerProvider` rather than living on `MinosCore`; the controller's `AsyncValue<PairResponse?>` is the single source of truth that `HomePage` reads. Keeping `MinosCore` a thin pass-through avoids duplicating state across two Riverpod consumers.

Widget / unit tests mock `MinosCoreProtocol` — never the frb-generated `MobileClient`. This keeps the generated layer untouched by test mutations.

---

## 7. Data Flow

### 7.1 App launch

```
main() async
  ├─ WidgetsFlutterBinding.ensureInitialized()
  ├─ docsDir = await getApplicationDocumentsDirectory()
  ├─ logDir  = '${docsDir}/Minos/Logs'
  ├─ core    = await MinosCore.init(selfName: 'iPhone', logDir: logDir)  // hardcoded Tier A; device_info_plus is Tier B
  └─ runApp(ProviderScope(
       overrides: [minosCoreProvider.overrideWithValue(core)],
       child: MinosApp(),
     ))
```

### 7.2 QR scan → `Connected`

```
[PairingPage]
  user scans QR (or pastes in kDebugMode)
    ↓
  pairingControllerProvider.notifier.submit(qrJson)
    ↓
  minosCore.pairWithJson(qrJson)
    ↓ frb dart:ffi
  Rust: serde_json::from_str::<QrPayload>(qrJson)?
        MobileClient.pair_with(qr)
          state_tx.send(Pairing)           ← _Router shows loading overlay
          WsClient::connect(ws://{host}:{port})   ← Tailscale WireGuard tunnel
          MinosRpcClient::pair(PairRequest{device_id, name, token})
          // macOS daemon validates token, transitions state to Paired
          state_tx.send(Connected)         ← _Router renders HomePage
        Ok(PairResponse{ok, mac_name})
    ↑ Future<PairResponse>
  pairingController state: AsyncData(PairResponse{ok, mac_name})
  _Router jumps to HomePage on state==Connected; HomePage reads
    ref.watch(pairingControllerProvider).valueOrNull → subtitle ${mac_name}
```

### 7.3 Debug-only paste fallback

For simulator iteration (no camera) during development:

1. Developer runs `cargo run -p minos-daemon` on the macOS host — the CLI binary prints `pairing_qr: {...JSON...}` to stdout.
2. Developer copies the JSON.
3. On iOS simulator: open `PairingPage` → `kDebugMode` floating action button → "粘贴 QR JSON" sheet → paste → submit.
4. `pairingController.submit` takes the identical code path as the camera, so Tier A can iterate without signing to a real device.

`kDebugMode` is a compile-time `const` in Flutter; the `debug_paste_qr_sheet.dart` tree and its trigger are dead-code-eliminated in release builds.

---

## 8. Error Handling

### 8.1 Rust → Dart mapping

- frb v2 `#[frb(mirror(MinosError))]` maps `Result<T, MinosError>` to a throwing Dart async call. Dart catches via sealed-class hierarchy (`MinosErrorConnectFailed`, `MinosErrorPairingTokenInvalid`, …).
- `ErrorKind` and `Lang` are mirrored so Dart can pattern-match locally and construct the `kind_message(kind, lang)` FFI call.
- The existing zh + en strings in `ErrorKind::user_message` (spec §7.1) are the **single source of truth**; Dart does **not** re-hardcode copy.

### 8.2 Dart-side UI text

```dart
// lib/domain/minos_error_display.dart
extension MinosErrorDisplay on MinosError {
  ErrorKind get kind => switch (this) {
    MinosErrorBindFailed() => ErrorKind.bindFailed,
    MinosErrorConnectFailed() => ErrorKind.connectFailed,
    MinosErrorDisconnected() => ErrorKind.disconnected,
    MinosErrorPairingTokenInvalid() => ErrorKind.pairingTokenInvalid,
    MinosErrorPairingStateMismatch() => ErrorKind.pairingStateMismatch,
    MinosErrorDeviceNotTrusted() => ErrorKind.deviceNotTrusted,
    MinosErrorStoreIo() => ErrorKind.storeIo,
    MinosErrorStoreCorrupt() => ErrorKind.storeCorrupt,
    MinosErrorCliProbeTimeout() => ErrorKind.cliProbeTimeout,
    MinosErrorCliProbeFailed() => ErrorKind.cliProbeFailed,
    MinosErrorRpcCallFailed() => ErrorKind.rpcCallFailed,
  };

  String userMessage([Lang lang = Lang.zh]) =>
      RustLib.instance.api.kindMessage(kind: kind, lang: lang);
}
```

UI surfacing:
- `PairingPage` uses `ref.listen<AsyncValue<void>>(pairingControllerProvider, ...)`: on `AsyncError<MinosError>`, calls `ShadToaster.of(context).show(ShadToast.destructive(description: Text(err.userMessage())))`.
- No hardcoded zh strings in UI code for error paths — every display string routes through `userMessage()`.
- Tier A defaults to `Lang.zh`; en access is latent.

### 8.3 Tier A failure modes that must be handled

| # | Trigger | Rust error | Dart UX |
|---|---|---|---|
| 1 | iOS Tailscale off / not signed in (Mac 100.x unreachable) | `ConnectFailed` | ShadToast with `userMessage()` |
| 2 | Scanned code is not JSON / not a `QrPayload` | `StoreCorrupt { path: "qr_payload", .. }` | ShadToast "本地配对状态损坏,已备份;请重新配对" (literal — room for Tier B to specialize the copy) |
| 3 | QR token expired (> 5 min) | `PairingTokenInvalid` | ShadToast "二维码已过期,请重新扫描" |
| 4 | Mac is already paired with another device | `PairingStateMismatch` | ShadToast "已存在配对设备,请确认替换" |
| 5 | Camera permission permanently denied | *(Dart-side; not a `MinosError`)* | `PermissionDeniedPage` with "打开设置" CTA |
| 6 | RPC handler errors server-side | `RpcCallFailed` | ShadToast "服务端错误,请稍后重试" |

Error mode #2 inherits a literal that was written for persisted-state corruption and reads slightly awkwardly for "scanned wrong QR". Tier B should introduce a dedicated `ErrorKind::QrPayloadMalformed` and a matching entry in the localization table; left alone for Tier A to avoid touching the domain error enum mid-UI-plan.

---

## 9. Testing Strategy

### 9.1 Rust matrix

| Crate | Test | Tool |
|---|---|---|
| `minos-mobile` | New unit: `new_with_in_memory_store()` returns usable instance | `cargo test -p minos-mobile` |
| `minos-mobile` | New unit: `pair_with_json("not json")` → `StoreCorrupt` variant | Same |
| `minos-mobile` | Existing E2E `tests/e2e.rs` (`mobile_pairs_with_daemon_and_lists_clis`) — unchanged, remains the pre-FFI confidence anchor | Same |
| `minos-ffi-frb` | Build smoke: `cargo check -p minos-ffi-frb --target aarch64-apple-ios` | `xtask check-all` |
| `minos-ffi-frb` | Codegen drift: `cargo xtask gen-frb` then `git diff --exit-code` | `xtask check-all` + CI |

### 9.2 Dart matrix

Per the project-wide testing convention (unit tests cover logic only; UI / widget / functional tests are integration and deferred), Tier A ships **unit tests only** — logic layers that can be validated without widget trees or real I/O.

| Type | File | Scope |
|---|---|---|
| Unit | `test/unit/pairing_controller_test.dart` | Mock `MinosCoreProtocol` via mocktail; verify `submit` drives `loading → data(PairResponse)` on success, `loading → error(MinosError)` on `pairWithJson` throwing; verify idempotent re-submit from error state |
| Unit | `test/unit/minos_error_display_test.dart` | Every `MinosError` variant → correct `ErrorKind` mapping; `userMessage(Lang.zh)` and `userMessage(Lang.en)` return non-empty strings |

**Deferred to integration-test phase** (explicitly not in Tier A plan):
- `pairing_page_test.dart` (widget test exercising permission branches + scanner mount + `kDebugMode` paste sheet)
- `home_page_test.dart` (widget test verifying `已连接 {macName}` rendering)
- `integration_test/` end-to-end scenarios (already deferred per MVP spec §8.3 to P1.5)

The real-device smoke gate (§9.3) is the only functional-level verification Tier A provides; it is manual, not automated.

### 9.3 Real-device smoke gate

MVP spec §8.4 items 1–5 are the done bar:

```
□ Mac: Tailscale 已装 + 登录,100.x IP 可见
□ iPhone: Tailscale 已装 + 登录,100.x IP 可见,可 ping Mac
□ Mac: Minos.app 已安装并启动,MenuBar 图标可见
□ Mac: 点击 "Show QR" → QRSheet 出现,二维码可见
□ iOS: Minos 通过 Xcode 直装真机,打开后进入 PairingPage,扫码后 5 秒内跳转 HomePage 并显示 "已连接 {MacName}"
```

Items 6–11 (reconnect, forget, CLI rows, diagnostics) explicitly belong to Tier B.

Smoke artifacts: record the iOS `Documents/Minos/Logs/mobile-rust.*.xlog` of a successful run; attach to the closing commit message.

---

## 10. Tooling & Developer Ergonomics

### 10.1 `cargo xtask` command updates

| Command | Change | Action |
|---|---|---|
| `xtask gen-frb` | New | Invoke `flutter_rust_bridge_codegen generate` using `flutter_rust_bridge.yaml` |
| `xtask build-ios` | New | `cargo build --target aarch64-apple-ios --target aarch64-apple-ios-sim -p minos-ffi-frb --release` |
| `xtask bootstrap` | Modify | Append `(cd apps/mobile && flutter pub get && dart run build_runner build --delete-conflicting-outputs)` |
| `xtask check-all` | Modify | Append Dart/Flutter leg: `(cd apps/mobile && dart format --set-exit-if-changed && dart analyze --fatal-infos && flutter test && dart run custom_lint)`; plus a `gen-frb → git diff --exit-code` drift check |

### 10.2 frb codegen workflow

`flutter_rust_bridge.yaml` at the repository root:

```yaml
# Minimum-viable config; refer to frb v2 docs for additional keys
rust_input: crates/minos-ffi-frb/src/api/**/*.rs
rust_root: crates/minos-ffi-frb
dart_output: apps/mobile/lib/src/rust
rust_output: crates/minos-ffi-frb/src/frb_generated.rs
```

Regeneration trigger: any change to `crates/minos-ffi-frb/src/api/` or to mirrored types' public shape. `check-all`'s drift guard turns "forgot to regen" into a CI failure.

### 10.3 Generated artifact check-in policy

- **Checked in**: `apps/mobile/lib/src/rust/**` and `crates/minos-ffi-frb/src/frb_generated.rs`.
- **Rationale**: CI's Dart job runs on `ubuntu-latest` and should not need a Rust toolchain just to `dart analyze`; committing the generated Dart keeps the CI surface small. This is the same choice plan 02 made for UniFFI-generated Swift (per commit `886e18a feat(xtask): detect uniffi-bindgen-swift codegen drift via replace_required asserts`).
- **Safety net**: the `gen-frb → git diff --exit-code` step in `check-all` means any divergence between committed artifact and current source blocks merge.

### 10.4 CI updates

`.github/workflows/ci.yml`:

| Job | Change |
|---|---|
| `dart` (ubuntu-latest) | Previously trivial (empty `apps/mobile`); now actually runs `flutter pub get`, `dart format`, `dart analyze --fatal-infos`, `flutter test`, `dart run custom_lint` against a real Flutter project |
| `linux` (Rust) | Append `cargo check -p minos-ffi-frb` |
| `frb-drift` step | Append to the Rust job (requires toolchain): install `flutter_rust_bridge_codegen`, run `cargo xtask gen-frb`, `git diff --exit-code` |
| `swift` (macos-15) | Unchanged. Tier A does not add iOS Xcode builds to CI (deferred per MVP spec §8.5) |

---

## 11. Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | frb v2 `#[frb(mirror)]` cannot fully express `MinosError` (variants with structured fields) | Medium | Dart loses structured errors | Fall back to a flattened mirror enum in `minos-ffi-frb` (variant + `String context`) while keeping the full enum for UniFFI |
| 2 | `StreamSink` cancellation does not abort the `tokio::spawn` task, leaking one task per pair attempt | Medium | Slow memory growth during repeated dev loops | Use `sink.add().is_err()` as termination signal; verify with `tokio-console` during bring-up |
| 3 | `mobile_scanner` struggles on older iPhones / low-power mode — slow decode | Low | Poor scan UX; still passes Tier A | Observe during real-device smoke; fall back to `qr_code_scanner` only if the failure is reproducible |
| 4 | iOS Tailscale not authorized yet when the app first launches → `ConnectFailed` without guidance | Medium | User confusion on first run | `MinosError::ConnectFailed` localized copy already covers this; a richer pre-check is Tier B |
| 5 | Flutter 3.41 + frb 2.x + native-assets build chain has known iOS edge cases | Medium | `flutter build ios` fails on first attempt | First plan-03 phase must include a "empty project builds ios" milestone before adding business code |
| 6 | `NSCameraUsageDescription` copy flagged during future App Store review | Low (not submitting in Tier A) | Irrelevant until release spec | Use clear, functional copy; revisit in release-pipeline spec |

---

## 12. Out of Scope (recap)

| Item | Target |
|---|---|
| Dart `list_clis` consumption + CLI list on `HomePage` | `ios-mvp-completion-design.md` (Tier B) |
| Auto-reconnect loop with exponential backoff | Tier B |
| Keychain-backed Dart `PairingStore` via frb callback | Tier B |
| "Forget this Mac" UI + Mac-side revocation wiring | Tier B |
| English / i18n language picker | Future i18n spec |
| Android real-device validation | P1.5 `launchd-and-android.md` |
| iOS Xcode build in CI; TestFlight / notarization / DMG | P1.5 `release-pipeline.md` |
| `subscribe_events` Dart consumption (agent event stream) | P1 `codex-app-server-integration.md` / `streaming-chat-ui.md` |

---

## 13. Delivery Preview (non-binding; plan 03 is canonical)

| Phase | Deliverable | Validation gate |
|---|---|---|
| P0 | `minos-mobile` adds `new_with_in_memory_store`, `pair_with_json`; tests green | `cargo test -p minos-mobile` green |
| P1 | `minos-ffi-frb` adapter populated; `flutter_rust_bridge.yaml` committed | `cargo check -p minos-ffi-frb --target aarch64-apple-ios` + `cargo xtask gen-frb` succeed with no drift |
| P2 | `apps/mobile/` scaffold + `pubspec.yaml` + `Info.plist` + empty UI builds iOS | `flutter build ios --simulator --no-codesign` succeeds |
| P3 | Dart layering: 4 providers + 3 pages + `MinosCoreProtocol` | `dart analyze --fatal-infos` + `flutter test` (widget suite) green |
| P4 | xtask + CI + ADR 0008 landed | `cargo xtask check-all` locally green; CI green |
| P5 | Real-device smoke | §8.4 items 1–5 all ticked; xlog snapshot attached to commit |

Each phase is one commit and one reviewer gate per the "Batch tasks before review" working agreement.

---

## 14. File Change Summary

```
Added:
├── flutter_rust_bridge.yaml
├── apps/mobile/**                                    (full Flutter scaffold)
├── apps/mobile/ios/Runner/Info.plist                 (NSCameraUsageDescription)
├── apps/mobile/lib/src/rust/**                       (frb generated; checked in)
├── apps/mobile/lib/…                                 (infrastructure/application/domain/presentation + tests)
├── crates/minos-ffi-frb/src/api/minos.rs             (frb mirrors + MobileClient wrapper)
├── crates/minos-ffi-frb/src/frb_generated.rs         (frb generated; checked in)
└── docs/adr/0008-frb-v2-boundary-and-generated-artifact-policy.md

Modified:
├── .github/workflows/ci.yml                          (dart job fleshed out; frb-drift step)
├── Cargo.toml                                        (confirm minos-ffi-frb crate-type list)
├── crates/minos-ffi-frb/Cargo.toml                   (add flutter_rust_bridge, tokio, minos-* deps)
├── crates/minos-ffi-frb/src/lib.rs                   (replace placeholder with real module tree)
├── crates/minos-mobile/src/client.rs                 (add new_with_in_memory_store, pair_with_json + tests)
├── xtask/src/main.rs                                 (gen-frb, build-ios, bootstrap, check-all updates)
└── docs/superpowers/plans/01-rust-core-and-monorepo-scaffold.md  (forward pointer only; optional)
```

---

## 15. ADR Index (delta)

| # | Topic |
|---|---|
| 0008 | frb v2 boundary: opaque / mirror choices; generated artifact check-in policy; raw-JSON QR payload at FFI boundary (vs mirroring `QrPayload`); `minos-mobile` does not depend on `flutter_rust_bridge` |
