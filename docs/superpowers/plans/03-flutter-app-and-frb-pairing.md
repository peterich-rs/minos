# Minos · Flutter App + frb Pairing Bring-up — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans`. Execute **one phase per subagent, one validation gate per phase, and one commit per phase**. This document is intentionally phase-oriented; do not fall back to task-by-task micro-commits. Dispatch UI-heavy phases (in particular Phase D) to the `frontend-design` specialist rather than the generic implementer. All dispatched subagents run on `opus` — do not auto-downgrade.

**Goal:** Stand up the iOS Flutter app under `apps/mobile/`, wire `minos-mobile::MobileClient` across the `flutter_rust_bridge` v2 boundary, and prove the full pipeline end-to-end by scanning the macOS-rendered pairing QR on a real iPhone and reaching `Connected` within 5 seconds. The plan ends when MVP smoke checklist §8.4 items 1–5 are ticked on hardware, with a captured `mobile-rust.*.xlog` attached to the closing commit.

**Architecture:** Tier A plumbing only. `minos-mobile` gains two FFI-friendly entry points; `minos-ffi-frb` becomes a real adapter with `#[frb(opaque)]` / `#[frb(mirror)]` wrappers over `MobileClient`; `apps/mobile/` scaffolds a Flutter project, bridges to the Rust core through frb v2, and exposes a two-screen UI (pair / home) backed by Riverpod-codegen providers. The project-wide logic-only unit-test rule is honored: no widget tests, no `integration_test`, no Preview snapshots in this plan.

**Tech Stack:**
- Rust stable (inherited from `rust-toolchain.toml`); `tokio` feature `rt` added to `minos-ffi-frb`
- `flutter_rust_bridge` 2.x (new API, proc-macro + `StreamSink`)
- Flutter 3.41.x + Dart 3.6.x; `flutter_riverpod` 2.x + `riverpod_annotation` + `riverpod_generator` + codegen via `build_runner`; `shadcn_ui` at UI root
- `mobile_scanner` for QR capture; `permission_handler` for camera permission flow; `path_provider` for sandbox-aware log directory
- `xlog` (Dart) for Dart-side logs; `mocktail` for unit-test mocks
- Xcode 26.2 (inherited from plan 02); iOS 16 deployment target; Apple Developer personal account for real-device signing

**Reference spec:** Implements `docs/superpowers/specs/flutter-app-and-frb-pairing-design.md`. All behavior, layering, and scope decisions live in the spec; this plan optimizes execution order and commit boundaries.

**Working directory note:** Runs on `main` alongside plans 01 and 02; single-developer repo. No worktree isolation required.

**Version drift policy:** Versions listed here are accurate as of 2026-04-22. If `cargo add` / `flutter pub add` / `brew install` resolves to a higher minor version when executed, prefer the resolved version unless compilation fails.

---

## File structure (target end-state)

```text
minos/
├── .github/workflows/ci.yml                               [modified: dart job fleshed out, frb-drift step, linux adds minos-ffi-frb check]
├── README.md                                              [modified: plan-03 status]
├── Cargo.toml                                             [unchanged: minos-ffi-frb already in workspace]
├── flutter_rust_bridge.yaml                               [new]
├── apps/
│   └── mobile/                                            [new]
│       ├── pubspec.yaml
│       ├── analysis_options.yaml                          [riverpod_lint + custom_lint]
│       ├── ios/Runner/Info.plist                          [NSCameraUsageDescription, iOS 16]
│       ├── ios/Runner.xcodeproj/                          [flutter create output, bundle id ai.minos.mobile]
│       ├── android/                                       [scaffolded, not validated]
│       ├── lib/
│       │   ├── main.dart
│       │   ├── src/rust/                                  [frb generated — checked in]
│       │   ├── infrastructure/{minos_core.dart, app_paths.dart}
│       │   ├── application/minos_providers.dart           [+ .g.dart codegen, checked in]
│       │   ├── domain/{minos_core_protocol.dart, minos_error_display.dart}
│       │   └── presentation/
│       │       ├── app.dart
│       │       ├── pages/{pairing_page, home_page, permission_denied_page}.dart
│       │       └── widgets/{qr_scanner_view, debug_paste_qr_sheet}.dart
│       └── test/unit/{pairing_controller_test, minos_error_display_test}.dart
├── crates/
│   ├── minos-mobile/
│   │   └── src/client.rs                                  [modified: new_with_in_memory_store, pair_with_json + tests]
│   └── minos-ffi-frb/
│       ├── Cargo.toml                                     [modified: + flutter_rust_bridge, tokio, minos-* deps]
│       ├── src/
│       │   ├── lib.rs                                     [modified: replace placeholder with module tree]
│       │   ├── api/minos.rs                               [new]
│       │   └── frb_generated.rs                           [new — frb generated, checked in]
├── docs/
│   └── adr/
│       └── 0008-frb-v2-boundary-and-generated-artifact-policy.md  [new]
├── xtask/
│   └── src/main.rs                                        [modified: gen-frb, build-ios, bootstrap Flutter leg, check-all Flutter leg, drift guard]
└── .gitignore                                             [modified: .dart_tool/, build/, .flutter-plugins*, ephemeral/]
```

---

## Current checkpoint

- Plans 01 and 02 are landed; `cargo xtask check-all` is green; macOS `Minos.app` generates and displays the pairing QR.
- `crates/minos-mobile` has `MobileClient`, `InMemoryPairingStore`, and logging init wired to the domain/protocol/transport layers. `crates/minos-mobile/tests/e2e.rs` exercises the full "pair → list_clis → forget" pipeline in-process and is the pre-FFI confidence anchor; it is not modified by this plan.
- `crates/minos-ffi-frb/src/lib.rs` is a sentinel shim (`minos_ffi_frb_ping`) reserving the crate name; `apps/mobile/` contains only `.gitkeep`.
- `flutter_rust_bridge_codegen` is **not yet installed locally**; Phase E's `bootstrap` update adds the install step, but Phase C also needs it ad-hoc for the initial generation. The plan notes this in Phase C's preparation section.

---

## Phase dependency graph

```text
Plans 01–02 landed.
 -> Phase A  Rust minos-mobile FFI-friendly additions
    -> Phase B  Flutter scaffold (empty iOS shell builds)
       -> Phase C  frb v2 adapter + codegen into apps/mobile/lib/src/rust
          -> Phase D  Dart layering, pairing flow, and unit tests
             -> Phase E  Tooling + CI + ADR 0008
                -> Phase F  Real-device smoke and plan close-out
```

### Phase execution rules

1. One implementation subagent owns one phase end-to-end. UI-heavy phases dispatch to the `frontend-design` specialist.
2. A phase is not done until its listed validation commands pass **and** (where applicable) the required generated artifacts are regenerated and checked in.
3. Do not split a phase into multiple commits unless validation exposes a narrow repair inside that same phase.
4. The design spec remains the source of truth for behavior, layering, and UI scope; this plan optimizes execution order and commit boundaries.
5. If a phase needs a small adjacent config/doc change to make its own gate pass, keep that change in the same phase commit.
6. Never weaken Phase A's test coverage or Phase D's unit-test scope to unblock Phase F; the logic-only unit-test rule (spec §9.2) is absolute.

---

## Phase A · Rust minos-mobile FFI-friendly additions

**Goal:** Add the two FFI-friendly entry points the frb adapter needs, without introducing any frb dependency into `minos-mobile`. Keep the hexagonal border intact.

**Scope:**
- Add `MobileClient::new_with_in_memory_store(self_name: String) -> Self` on the existing `impl MobileClient` block.
- Add `MobileClient::pair_with_json(&self, qr_json: String) -> Result<PairResponse, MinosError>` that internally `serde_json::from_str::<QrPayload>` then delegates to the existing `pair_with`.
- Add unit tests in `crates/minos-mobile/src/client.rs` (or an adjacent module) covering:
  - `new_with_in_memory_store()` returns an instance whose `current_state() == ConnectionState::Disconnected`.
  - `pair_with_json("not json")` returns `Err(MinosError::StoreCorrupt { path: "qr_payload", .. })`.
  - `pair_with_json(valid_json)` is symmetric with `pair_with(parsed_qr)` against a fresh in-process `DaemonHandle` (reuse the `tests/e2e.rs` bring-up helper or add a focused integration test mirroring it).
- Do **not** modify the existing `events_stream` or `pair_with` surface.

**Files likely touched:**
- `crates/minos-mobile/src/client.rs`
- `crates/minos-mobile/tests/e2e.rs` (only if a helper refactor is required to avoid duplication; prefer not to)
- `crates/minos-mobile/Cargo.toml` (only if a dev-dependency addition is required; existing `serde_json` should already be available transitively via `minos-pairing`/`minos-protocol` — verify)

**Preserved constraints:**
- `minos-mobile` does **not** add `flutter_rust_bridge` as a dependency in this phase (or any phase).
- `InMemoryPairingStore` remains in `store.rs`; the new constructor wires it without changing the trait-object-based `MobileClient::new` signature.
- The `StoreCorrupt` variant carries `{ path, message }`; the QR-malformed case uses `path: "qr_payload"` literally. A dedicated `QrPayloadMalformed` variant is out of scope for this plan (spec §8.3 note).

**Validation:**

```bash
cargo fmt --check
cargo clippy -p minos-mobile --all-targets -- -D warnings
cargo test -p minos-mobile
cargo xtask check-all
```

**Commit boundary:**

```bash
git add crates/minos-mobile/src/client.rs crates/minos-mobile/tests crates/minos-mobile/Cargo.toml
git commit -m "feat(minos-mobile): add FFI-friendly constructor and pair_with_json"
```

---

## Phase B · Flutter scaffold (empty iOS shell builds)

**Goal:** Scaffold the Flutter project under `apps/mobile/` with iOS + Android targets, produce a buildable empty iOS shell, and prove `flutter build ios --simulator --no-codesign` succeeds **before** any frb integration lands. This is the Risk §5 fire-break: surface any native_assets / toolchain-level iOS build issues while the blast radius is still a single-page counter app.

**Scope:**
- Run `flutter create --org ai.minos --project-name minos --platforms ios,android --template app apps/mobile` from the repo root.
- Delete the `.gitkeep` placeholder and the auto-generated counter-app: replace `apps/mobile/lib/main.dart` with a minimal `runApp(const MaterialApp(home: Scaffold(body: Center(child: Text('Minos')))));` shell so Phase C can layer frb work on top without fighting the default template.
- Delete the auto-generated `apps/mobile/test/widget_test.dart` (the widget test is both a placeholder and a convention violation per spec §9.2).
- Patch `apps/mobile/ios/Runner/Info.plist`: add `NSCameraUsageDescription = "Minos 需要使用相机扫描 Mac 上的配对二维码"`; confirm (or set) `UILaunchStoryboardName` / `CFBundleDisplayName` as Flutter generated them.
- Set the iOS deployment target to 16.0 in both `ios/Podfile` (`platform :ios, '16.0'`) and the Xcode project's `IPHONEOS_DEPLOYMENT_TARGET`.
- Set the iOS Bundle ID to `ai.minos.mobile` (in the Xcode project's `PRODUCT_BUNDLE_IDENTIFIER`).
- Add root `.gitignore` entries for `apps/mobile/.dart_tool/`, `apps/mobile/build/`, `apps/mobile/ios/Pods/`, `apps/mobile/ios/.symlinks/`, `apps/mobile/ios/Flutter/ephemeral/`, `apps/mobile/ios/Flutter/Flutter.podspec`, `apps/mobile/android/.gradle/`, `apps/mobile/android/local.properties`, `apps/mobile/.flutter-plugins`, `apps/mobile/.flutter-plugins-dependencies`.
- Add `apps/mobile/analysis_options.yaml` with `include: package:flutter_lints/flutter_lints.yaml` as the baseline (Phase D extends it with `custom_lint` + `riverpod_lint`).
- Do **not** add `flutter_rust_bridge`, `riverpod`, `shadcn_ui`, `mobile_scanner`, `permission_handler`, `path_provider`, or `xlog` yet. Phase C adds frb; Phase D adds the rest.

**Files likely touched:**
- `apps/mobile/` (entire tree, new)
- `.gitignore` (modified)

**Preserved constraints:**
- `android/` exists but is not validated beyond what `flutter create` produces.
- `flutter build ios --simulator --no-codesign` must succeed on the scaffold alone; if it fails, the root cause is a toolchain mismatch (CocoaPods version, Xcode command-line tools, Flutter 3.41 quirks) — fix within this phase rather than deferring to Phase C.
- Do **not** run `pod install` with out-of-band environment fixes; if pods fail, surface the error and fix via standard tooling.

**Validation:**

```bash
cd apps/mobile
flutter --version              # confirm 3.41.x
flutter pub get
dart format --set-exit-if-changed .
dart analyze --fatal-infos
flutter build ios --simulator --no-codesign
cd ../..
```

**Commit boundary:**

```bash
git add apps/mobile .gitignore
git commit -m "feat(flutter): scaffold apps/mobile empty iOS shell"
```

---

## Phase C · frb v2 adapter + codegen

**Goal:** Populate `minos-ffi-frb` with the full adapter surface specified in spec §5.3, generate the Dart bindings into `apps/mobile/lib/src/rust/`, and commit both sides so CI's Dart leg runs without needing a Rust toolchain.

**Preparation (not a commit):**
- Install `flutter_rust_bridge_codegen` at the version resolved by Cargo for `flutter_rust_bridge = "2"`: `cargo install flutter_rust_bridge_codegen --version ^2 --locked`. Phase E's `xtask bootstrap` codifies this; Phase C needs it ad-hoc.

**Scope:**
- Update `crates/minos-ffi-frb/Cargo.toml`:
  - Add `flutter_rust_bridge = "2"` (runtime + macros).
  - Add `tokio = { version = "1", features = ["rt"] }` (needed by the `subscribe_state` spawn).
  - Add path deps: `minos-mobile = { path = "../minos-mobile" }`, `minos-domain = { path = "../minos-domain" }`, `minos-protocol = { path = "../minos-protocol" }`.
  - Confirm `[lib] crate-type = ["staticlib", "cdylib"]` (staticlib for iOS App Store policy per spec §5.3).
- Replace `crates/minos-ffi-frb/src/lib.rs` with a real module tree that declares `pub mod api;` and includes `pub use frb_generated::*;` gated on the `frb_generated` module existing after codegen. Remove the `minos_ffi_frb_ping` sentinel and the `_link_minos_mobile` placeholder.
- Create `crates/minos-ffi-frb/src/api/minos.rs` with exactly the surface from spec §5.3:
  - `#[frb(mirror(ConnectionState))]` on a private shadow enum with the four variants.
  - `#[frb(mirror(PairResponse))]` on a private shadow struct with `ok: bool, mac_name: String`.
  - `#[frb(mirror(ErrorKind))]` on a private shadow enum with all 11 variants (from `minos-domain/src/error.rs:ErrorKind`, 1:1).
  - `#[frb(mirror(Lang))]` on a private shadow enum with `Zh, En`.
  - `#[frb(mirror(MinosError))]` covering every variant including those with structured fields (`BindFailed { addr, message }`, `ConnectFailed { url, message }`, `Disconnected { reason }`, `PairingTokenInvalid`, `PairingStateMismatch { actual: PairingState }`, `DeviceNotTrusted { device_id }`, `StoreIo { path, message }`, `StoreCorrupt { path, message }`, `CliProbeTimeout { bin, timeout_ms }`, `CliProbeFailed { bin, message }`, `RpcCallFailed { method, message }`). If `PairingStateMismatch.actual: PairingState` cannot be mirrored cleanly, fall back to a flattened mirror (replace `PairingState` with its `Debug` string) per Risk §1 — keep the full structured enum available to UniFFI unchanged.
  - `#[frb(opaque)] pub struct MobileClient(minos_mobile::MobileClient);`
  - Impl on `MobileClient`:
    - `#[frb(sync)] pub fn new(self_name: String) -> Self` delegating to `minos_mobile::MobileClient::new_with_in_memory_store(self_name)`.
    - `pub async fn pair_with_json(&self, qr_json: String) -> Result<PairResponse, MinosError>` delegating.
    - `#[frb(sync)] pub fn current_state(&self) -> ConnectionState` delegating.
    - `pub fn subscribe_state(&self, sink: StreamSink<ConnectionState>)` spawning a `tokio::spawn` that reads `events_stream()` (a `watch::Receiver<ConnectionState>`) and forwards via `sink.add(...)`; terminates when `sink.add(...).is_err()`.
  - Free functions:
    - `pub fn init_logging(log_dir: String) -> Result<(), MinosError>` wrapping `minos_mobile::logging::init(Path::new(&log_dir))`.
    - `#[frb(sync)] pub fn kind_message(kind: ErrorKind, lang: Lang) -> String` delegating to `kind.user_message(lang).to_string()`.
- Create `flutter_rust_bridge.yaml` at the repo root:
  ```yaml
  rust_input: crates/minos-ffi-frb/src/api/**/*.rs
  rust_root: crates/minos-ffi-frb
  dart_output: apps/mobile/lib/src/rust
  rust_output: crates/minos-ffi-frb/src/frb_generated.rs
  ```
- Run `flutter_rust_bridge_codegen generate` from the repo root. The command writes `crates/minos-ffi-frb/src/frb_generated.rs` and populates `apps/mobile/lib/src/rust/` with `frb_generated.dart` plus an `api/minos.dart` (exact names per frb v2 output).
- Verify everything compiles: `cargo check -p minos-ffi-frb` (host target) and `cargo check -p minos-ffi-frb --target aarch64-apple-ios` (iOS target).
- Update `apps/mobile/pubspec.yaml` to add the single frb dependency: `flutter_rust_bridge: ^2.0.0` (version resolved at add-time). Do not add any other deps in this phase.
- Update `apps/mobile/lib/main.dart` minimally to call `await RustLib.init()` before `runApp` (the rest of Dart wiring is Phase D). The home screen stays the simple `Text('Minos')` placeholder.
- Verify `flutter build ios --simulator --no-codesign` still succeeds with the frb static library linked in. **If the build fails here, it is the Risk §5 materialization** — troubleshoot and resolve within this phase; do not defer.

**Files likely touched:**
- `crates/minos-ffi-frb/Cargo.toml`
- `crates/minos-ffi-frb/src/lib.rs`
- `crates/minos-ffi-frb/src/api/minos.rs`
- `crates/minos-ffi-frb/src/frb_generated.rs` (generated)
- `apps/mobile/lib/src/rust/**` (generated)
- `apps/mobile/pubspec.yaml`
- `apps/mobile/lib/main.dart`
- `flutter_rust_bridge.yaml`

**Preserved constraints:**
- `minos-mobile` stays free of `flutter_rust_bridge` — only `minos-ffi-frb` links against it.
- Generated files (both `crates/minos-ffi-frb/src/frb_generated.rs` and `apps/mobile/lib/src/rust/**`) are checked in. Drift guard lands in Phase E.
- `QrPayload` does not appear in any frb signature; raw `String` carries QR JSON across the FFI boundary.
- `Lang` default in the adapter's `kind_message` signature is **not** specified — Dart callers pass `Lang.zh` explicitly.

**Validation:**

```bash
# Adapter + Dart bindings compile
cargo build -p minos-ffi-frb
cargo check -p minos-ffi-frb --target aarch64-apple-ios
cargo check -p minos-ffi-frb --target aarch64-apple-ios-sim

# frb codegen is idempotent (no diff after regeneration)
flutter_rust_bridge_codegen generate
git diff --exit-code apps/mobile/lib/src/rust crates/minos-ffi-frb/src/frb_generated.rs

# Flutter still builds with frb linked
cd apps/mobile
flutter pub get
flutter build ios --simulator --no-codesign
cd ../..

# Full local workspace gate
cargo xtask check-all   # may still have gaps until Phase E lands; accept per-leg greens
```

**Commit boundary:**

```bash
git add crates/minos-ffi-frb apps/mobile/lib/src/rust apps/mobile/pubspec.yaml apps/mobile/lib/main.dart flutter_rust_bridge.yaml
git commit -m "feat(ffi-frb): populate frb v2 adapter with MobileClient surface"
```

---

## Phase D · Dart layering, pairing flow, and unit tests

**Goal:** Build the full Dart layer stack (infrastructure → application → domain → presentation) with `MinosCoreProtocol` shim, four Riverpod-codegen providers, three screens, and the two unit-test files. On a real device, scanning a macOS-rendered QR results in `HomePage` showing "已连接 {MacName}" within 5 seconds.

**Dispatch note:** This phase is the UI-heavy work — dispatch to the `frontend-design` specialist agent per project convention. Avoid generic implementers for widget composition and Riverpod wiring.

**Scope:**
- Update `apps/mobile/pubspec.yaml`:
  ```yaml
  dependencies:
    flutter:
      sdk: flutter
    flutter_rust_bridge: ^2.0.0
    flutter_riverpod: ^2.5.0
    riverpod_annotation: ^2.3.0
    shadcn_ui: ^0.30.0
    mobile_scanner: ^5.0.0
    permission_handler: ^11.0.0
    path_provider: ^2.1.0
    xlog: ^0.1.0
  dev_dependencies:
    flutter_test:
      sdk: flutter
    build_runner: ^2.4.0
    riverpod_generator: ^2.4.0
    riverpod_lint: ^2.3.0
    custom_lint: ^0.6.0
    mocktail: ^1.0.0
    flutter_lints: ^5.0.0
  ```
  Resolve versions at add-time with `flutter pub add`; the `^` ranges above are starting points.
- Update `apps/mobile/analysis_options.yaml` to enable `riverpod_lint` and `custom_lint` plugins (per `riverpod_lint` README).
- Create `apps/mobile/lib/domain/minos_core_protocol.dart` as an abstract class with exactly three members: `Future<PairResponse> pairWithJson(String qrJson)`, `Stream<ConnectionState> get states`, `ConnectionState get current`. Import `PairResponse` and `ConnectionState` from the frb-generated barrel.
- Create `apps/mobile/lib/domain/minos_error_display.dart` as an extension on `MinosError` exposing:
  - `ErrorKind get kind` — pure Dart pattern-match, 11 arms, delegating each variant to the matching `ErrorKind.<variant>`.
  - `String userMessage([Lang lang = Lang.zh])` — calls the frb-generated `kindMessage(kind: kind, lang: lang)` and returns its result.
  Do not hardcode any zh / en strings on the Dart side.
- Create `apps/mobile/lib/infrastructure/app_paths.dart` with a `Future<String> logDirectory()` helper that calls `getApplicationDocumentsDirectory()` and appends `/Minos/Logs`, creating the directory if absent.
- Create `apps/mobile/lib/infrastructure/minos_core.dart` with the `MinosCore` class implementing `MinosCoreProtocol` exactly as specified in spec §6.5 (private constructor, `init({selfName, logDir}) async` factory that runs `RustLib.init()`, calls the frb `initLogging(logDir: logDir)`, constructs a `MobileClient`, returns a wrapped instance; the three protocol methods delegate).
- Create `apps/mobile/lib/application/minos_providers.dart` with four `@riverpod` providers per spec §6.3 table:
  - `minosCoreProvider` — `@Riverpod(keepAlive: true)` returning `MinosCoreProtocol`; body throws `UnimplementedError` until overridden.
  - `connectionStateProvider` — `@Riverpod(keepAlive: true)` returning `Stream<ConnectionState>`, reads `ref.watch(minosCoreProvider).states`.
  - `cameraPermissionProvider` — `@riverpod` AsyncNotifier exposing `check()`, `request()`, `openSettings()` using `permission_handler`'s `Permission.camera`.
  - `pairingControllerProvider` — `@riverpod` AsyncNotifier with state `PairResponse?`, default `build()` returning `null`, method `submit(String qrJson)` that sets loading, calls `ref.read(minosCoreProvider).pairWithJson(qrJson)`, and assigns `AsyncData(response)` on success or `AsyncError(e, st)` on a `MinosError` catch.
- Run `dart run build_runner build --delete-conflicting-outputs` inside `apps/mobile/` to produce `minos_providers.g.dart`; commit the generated file.
- Create `apps/mobile/lib/presentation/app.dart`:
  - `MinosApp` widget wrapping `ShadApp(themeMode: ThemeMode.system, theme: ShadThemeData.light(...), darkTheme: ShadThemeData.dark(...), home: const _Router())` using Shad's default color scheme (no custom palette in Tier A).
  - `_Router` widget reads `ref.watch(connectionStateProvider)` and routes: `Connected` → `HomePage`; otherwise → `PairingPage`. The stream's loading/error states fall through to `PairingPage` so first-launch before any value is still pairable.
- Create `apps/mobile/lib/presentation/pages/pairing_page.dart`:
  - On mount: call `ref.read(cameraPermissionProvider.notifier).check()`; then request if denied.
  - Render:
    - `PermissionStatus.permanentlyDenied` → `PermissionDeniedPage`.
    - `PermissionStatus.granted` → `QrScannerView` wrapped in a `ShadCard` with short instructional copy.
    - Loading / denied-pending-request → `ShadProgress` indicator.
  - `ref.listen<AsyncValue<PairResponse?>>(pairingControllerProvider, (_, next) { ... })`: on `AsyncError`, call `ShadToaster.of(context).show(ShadToast.destructive(description: Text((next.error as MinosError).userMessage())))`.
  - `kDebugMode` gated: a `FloatingActionButton.extended(label: '粘贴 QR JSON', ...)` that opens a `showShadSheet` with a multiline `TextField` and submit button calling `pairingControllerProvider.notifier.submit(pasted)`.
- Create `apps/mobile/lib/presentation/pages/home_page.dart` per spec §6.4: `ShadCard` titled "已连接" with subtitle `${response.macName}` read from `ref.watch(pairingControllerProvider).valueOrNull`. No actions.
- Create `apps/mobile/lib/presentation/pages/permission_denied_page.dart`: centered explanation + `ShadButton` that calls `permission_handler.openAppSettings()`.
- Create `apps/mobile/lib/presentation/widgets/qr_scanner_view.dart`: wraps `MobileScanner(onDetect: (capture) { final raw = capture.barcodes.firstOrNull?.rawValue; if (raw != null) ref.read(pairingControllerProvider.notifier).submit(raw); })`. Handle the `MobileScannerController` lifecycle explicitly (start on mount, stop on dispose).
- Create `apps/mobile/lib/presentation/widgets/debug_paste_qr_sheet.dart`: sheet body extracted from the PairingPage's `kDebugMode` block for readability; entire file is tree-shaken from release builds because the only call site is `kDebugMode`-gated.
- Update `apps/mobile/lib/main.dart` to:
  - `WidgetsFlutterBinding.ensureInitialized()`.
  - Resolve `logDir` via `app_paths.logDirectory()`.
  - `await MinosCore.init(selfName: 'iPhone', logDir: logDir)`.
  - `runApp(ProviderScope(overrides: [minosCoreProvider.overrideWithValue(core)], child: const MinosApp()))`.
- Create `apps/mobile/test/unit/pairing_controller_test.dart`: use `mocktail` to fake `MinosCoreProtocol`; test three scenarios:
  1. `submit(valid)` transitions `AsyncData(null)` → `AsyncLoading` → `AsyncData(PairResponse(...))`.
  2. `submit(invalid)` where the mock throws a `MinosError.storeCorrupt` transitions to `AsyncError<MinosError>`.
  3. A second `submit` after an error clears the error and follows the success path.
  Override `minosCoreProvider` via `ProviderScope.overrides` with the fake in a `ProviderContainer` (no widget tree).
- Create `apps/mobile/test/unit/minos_error_display_test.dart`: for every `MinosError` variant (use representative field values), assert `error.kind` matches the expected `ErrorKind` and assert `error.userMessage(Lang.zh)` / `error.userMessage(Lang.en)` return non-empty strings.
- Ensure `flutter test` passes; keep `test/widget/` and `test/integration/` absent per spec §9.2.

**Files likely touched:**
- `apps/mobile/pubspec.yaml`, `apps/mobile/analysis_options.yaml`, `apps/mobile/lib/main.dart`
- `apps/mobile/lib/domain/{minos_core_protocol,minos_error_display}.dart`
- `apps/mobile/lib/infrastructure/{minos_core,app_paths}.dart`
- `apps/mobile/lib/application/{minos_providers.dart, minos_providers.g.dart}`
- `apps/mobile/lib/presentation/app.dart`
- `apps/mobile/lib/presentation/pages/{pairing_page,home_page,permission_denied_page}.dart`
- `apps/mobile/lib/presentation/widgets/{qr_scanner_view,debug_paste_qr_sheet}.dart`
- `apps/mobile/test/unit/{pairing_controller_test,minos_error_display_test}.dart`

**Preserved constraints:**
- Unit tests stay logic-only; no `WidgetTester`, no `integration_test`, no Preview snapshots, no `ShadApp`-wrapped widget tests.
- Riverpod providers are codegen-based; `riverpod_lint` is enabled and must pass.
- No hardcoded zh / en strings for error copy; every surfaced message routes through `userMessage()`.
- `pairingControllerProvider` is the single source of truth for `PairResponse`; `MinosCore` does not cache the response.
- `MobileClient` (frb-generated) is never imported outside `lib/infrastructure/minos_core.dart`; all other Dart files depend on `MinosCoreProtocol`.
- The UI covers only Tier A features: scan / paste-in-debug / connected card / permission-denied page. No CLI list, no reconnect banner, no forget affordance.

**Validation:**

```bash
cd apps/mobile
flutter pub get
dart run build_runner build --delete-conflicting-outputs
dart format --set-exit-if-changed .
dart analyze --fatal-infos
flutter test
dart run custom_lint
flutter build ios --simulator --no-codesign
cd ../..
```

**Commit boundary:**

```bash
git add apps/mobile
git commit -m "feat(flutter): wire Riverpod layers, pairing flow, and unit tests"
```

---

## Phase E · Tooling, CI, ADR 0008

**Goal:** Land the `xtask` updates, CI updates, ADR 0008, and the frb-drift guard. After this phase, a fresh clone can run `cargo xtask bootstrap && cargo xtask check-all` and everything is green without manual setup.

**Scope:**
- Extend `xtask/src/main.rs` with four new / modified subcommands (one subcommand implementation per function; clippy workspace lint caps functions at 100 lines so extract helpers as needed; ~800 lines total is the current realistic budget after the Flutter / drift-guard additions). Historical "under ~400 lines" guidance is superseded because UniFFI-side tooling already pushed the file past that before Phase E):
  - `gen-frb`: shells out to `flutter_rust_bridge_codegen generate` using `flutter_rust_bridge.yaml` at the repo root. Fails loudly if the binary isn't on PATH (point the user at `cargo xtask bootstrap`).
  - `build-ios`: runs `cargo build --target aarch64-apple-ios --target aarch64-apple-ios-sim -p minos-ffi-frb --release`.
  - `bootstrap` (modify): after the existing UniFFI/Swift tool install, install `flutter_rust_bridge_codegen` at the resolved `flutter_rust_bridge` major version and run `(cd apps/mobile && flutter pub get && dart run build_runner build --delete-conflicting-outputs)` to prime codegen artifacts.
  - `check-all` (modify): after the existing Rust + Swift legs, append a Flutter leg: `(cd apps/mobile && fvm flutter pub get && fvm dart format --set-exit-if-changed . && fvm flutter analyze --fatal-infos && fvm flutter test)`. Note: `dart run custom_lint` is NOT part of the leg because Phase D selected `riverpod_lint 3.x` which ships as a native analysis_server_plugin (picked up by `flutter analyze`); legacy `custom_lint` is not installed. Also append a drift guard: `gen-frb` followed by `git diff --exit-code apps/mobile/lib/src/rust crates/minos-ffi-frb/src/frb_generated.rs` AND a `git ls-files --others --exclude-standard` check over the same paths so new untracked generated files also fail the gate.
- Update `.github/workflows/ci.yml`:
  - `dart` job (ubuntu-latest): flesh out now that `apps/mobile/` is non-empty. Install Flutter via `subosito/flutter-action@v2` pinned to `3.41.x`; cache pub. Run `flutter pub get`, `dart run build_runner build --delete-conflicting-outputs` (verifies codegen goes through cleanly), `dart format --set-exit-if-changed .`, `dart analyze --fatal-infos`, `flutter test --exclude-tags ffi` (the `ffi`-tagged tests require the host dylib and run on the macOS lane via `cargo xtask check-all`). `dart run custom_lint` is intentionally omitted — `riverpod_lint 3.x` ships as a native analysis_server_plugin.
  - `linux` (Rust) job: append `cargo check -p minos-ffi-frb`.
  - Add a new `frb-drift` step at the end of the `linux` job (or as a dedicated job, whichever is simpler to cache): install `flutter_rust_bridge_codegen` at the pinned major version, run it, then `git diff --exit-code`. If it fails, the PR author forgot to `cargo xtask gen-frb`.
  - `swift` job: unchanged; no iOS Xcode build added (spec §10.4 explicit).
- Author `docs/adr/0008-frb-v2-boundary-and-generated-artifact-policy.md` following MADR 4.0 shape (`Context / Decision / Consequences / Alternatives Rejected`). Cover: the `minos-mobile` / `minos-ffi-frb` dependency split; the `#[frb(opaque)]` / `#[frb(mirror)]` choice; the raw-JSON-at-boundary decision for `QrPayload`; the check-in-generated-artifacts decision and the drift guard that backs it; why this matches plan 02's UniFFI decision for consistency. Explicitly reject: letting `minos-mobile` depend on `flutter_rust_bridge`; having Dart construct `QrPayload`; .gitignoring generated bindings.

**Files likely touched:**
- `xtask/src/main.rs`
- `.github/workflows/ci.yml`
- `docs/adr/0008-frb-v2-boundary-and-generated-artifact-policy.md`

**Preserved constraints:**
- `cargo xtask check-all` remains a single command covering the entire local gate. Don't fork into per-language sub-commands.
- CI stays on ubuntu-latest for the Dart leg; no iOS runners added (deferred to P1.5 release pipeline per spec §10.4).
- Pin Flutter version in CI to match `apps/mobile/pubspec.yaml environment`.
- Do not add secret-dependent signing / TestFlight steps.

**Validation:**

```bash
cargo xtask bootstrap
cargo xtask check-all
cargo xtask gen-frb
git diff --exit-code apps/mobile/lib/src/rust crates/minos-ffi-frb/src/frb_generated.rs
```

**Commit boundary:**

```bash
git add xtask/src/main.rs .github/workflows/ci.yml docs/adr/0008-frb-v2-boundary-and-generated-artifact-policy.md
git commit -m "ci/docs: add Flutter leg, frb drift guard, and ADR 0008"
```

---

## Phase F · Real-device smoke and plan close-out

**Goal:** Prove the entire pipeline on hardware by running MVP smoke checklist §8.4 items 1–5, archive an `.xlog` artifact as evidence, and finalize README + plan-03 documentation.

**Scope:**
- On the Mac host: confirm Tailscale is signed in and `tailscale ip -4` returns a `100.x` address; launch `Minos.app`; verify the MenuBar icon; open the inline QR popover.
- On the iPhone (real device, not simulator): confirm Tailscale is signed in; confirm it can reach the Mac's `100.x`.
- Connect the iPhone to the Mac via USB; open `apps/mobile/ios/Runner.xcodeproj` in Xcode; enable signing with the user's Apple Developer personal team; set the iPhone as the run destination; run the app.
- On first launch: grant camera permission in the system prompt.
- Scan the QR displayed by `Minos.app`. Within 5 seconds, the Flutter app should navigate from `PairingPage` to `HomePage` and display "已连接 {MacName}".
- Retrieve the `mobile-rust.*.xlog` from the iOS app sandbox (`Devices and Simulators` → `Minos` → `Download container`) and copy it to a temporary location. Rename to `mobile-rust-first-pair.xlog`.
- Tick each §8.4 item in the plan (this file) by replacing `□` with `✅` and adding a date stamp.
- Update `README.md`: add a short paragraph under the current status section noting that plan 03 is complete and the iOS pairing flow works end-to-end; link to this plan file.
- Attach the log file evidence in the commit message body (just the path / size; do not commit the log itself into the repo).

**Failure handling:**
- If any §8.4 item fails, open a follow-up task **and do not mark plan 03 complete**. Common failures and expected root causes:
  - Tailscale not signed in on iPhone → `MinosError.connectFailed`; fix device setup, retry.
  - QR expired (> 5 min) → `MinosError.pairingTokenInvalid`; click Mac-side "重新生成", retry.
  - Camera permission denied → verify `Info.plist` string; reset simulator/device permissions via Settings.
  - `flutter build ios --codesign` fails with provisioning profile issues → check Apple Developer team in Xcode Signing & Capabilities.
- The smoke gate is the **only** functional-level verification in this plan. If it fails, fix the underlying bug; do **not** introduce automated UI/widget/integration tests as a workaround (that decision belongs to a future integration-test-phase plan).

**Files likely touched:**
- `docs/superpowers/plans/03-flutter-app-and-frb-pairing.md` (this file — tick the smoke boxes, add the closing section)
- `README.md`

**Preserved constraints:**
- The logic-only unit-test rule remains absolute — do not add widget or integration tests to "help debug" failures.
- Do not add Android-specific smoke steps; Android validation is explicitly P1.5.

**Validation:**

1. Smoke checklist §8.4 items 1–5 all ticked in this plan document.
2. `mobile-rust-first-pair.xlog` file exists outside the repo, size > 0.
3. `cargo xtask check-all` green on `main`.

**Commit boundary:**

```bash
git add docs/superpowers/plans/03-flutter-app-and-frb-pairing.md README.md
git commit -m "chore(plan-03): close Tier A with real-device smoke artifacts"
```

---

## Final verification

### Automated gate

```bash
cargo xtask bootstrap
cargo xtask check-all
cargo xtask gen-frb
git diff --exit-code apps/mobile/lib/src/rust crates/minos-ffi-frb/src/frb_generated.rs
cd apps/mobile && flutter build ios --simulator --no-codesign && cd ../..
```

### Manual sanity gate (spec §8.4 items 1–5)

```
□ Mac: Tailscale 已装 + 登录,`tailscale ip -4` 返回 100.x
□ iPhone: Tailscale 已装 + 登录,100.x 可见,可 ping Mac 的 100.x
□ Mac: Minos.app 启动,MenuBar 图标可见
□ Mac: 点击 "Show QR" → 二维码出现
□ iOS: Minos 通过 Xcode 直装真机,打开后进入 PairingPage,扫码后 5 秒内 HomePage 显示 "已连接 {MacName}"
```

Plan 03 is complete only when all six phases are committed, automated gates pass, and the manual smoke gate is ticked (replace □ with ✅ and date-stamp) in this document.

---

## Deliverables

| Deliverable | Entry point | Verification |
|---|---|---|
| Rust FFI-friendly extensions | `crates/minos-mobile/src/client.rs` | `cargo test -p minos-mobile` |
| frb v2 adapter | `crates/minos-ffi-frb/src/api/minos.rs` | `cargo build -p minos-ffi-frb` + `cargo check -p minos-ffi-frb --target aarch64-apple-ios` |
| Dart bindings | `apps/mobile/lib/src/rust/` (generated, checked in) | `cargo xtask gen-frb` + `git diff --exit-code` |
| Flutter iOS shell | `apps/mobile/` | `flutter build ios --simulator --no-codesign` |
| Dart layering | `apps/mobile/lib/{infrastructure,application,domain,presentation}/` | `dart analyze --fatal-infos` + `flutter test` + `dart run custom_lint` |
| Unit tests | `apps/mobile/test/unit/` | `flutter test` |
| xtask updates | `xtask/src/main.rs` | `cargo xtask check-all` |
| CI lane updates | `.github/workflows/ci.yml` | GitHub Actions green on ubuntu `dart` + `linux` jobs |
| ADR 0008 | `docs/adr/0008-frb-v2-boundary-and-generated-artifact-policy.md` | review |
| README status update | `README.md` | review |
| Real-device smoke evidence | `mobile-rust-first-pair.xlog` referenced in closing commit body | file exists, size > 0 |

Plan 03 closes Tier A of the iOS bring-up. Tier B (list_clis consumption, auto-reconnect, Keychain-backed `PairingStore`, "Forget this Mac") lives in a separate `ios-mvp-completion-design.md` spec and its own plan.
