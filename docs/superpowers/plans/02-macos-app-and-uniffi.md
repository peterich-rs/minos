# Minos · macOS App + UniFFI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans`. Execute **one phase per subagent, one validation gate per phase, and one commit per phase**. This document is intentionally phase-oriented; do not fall back to task-by-task micro-commits.

**Goal:** Fully wire the UniFFI shim over `minos-daemon::DaemonHandle` and ship a SwiftUI `MenuBarExtra` macOS 14+ app (`ai.minos.macos`) that surfaces boot state, pairing-QR display, Forget-paired-device affordance, and today-log Finder reveal — backed by logic-only Swift unit tests and a CI job on `macos-15`. The plan ends when `cargo xtask check-all` passes end-to-end (Rust + Swift legs) and `xcodebuild -scheme Minos build` produces `Minos.app` with no warnings.

**Architecture:** Plan 02 still follows the design spec's layered split and dependency order: Rust domain/pairing/daemon groundwork first, then the UniFFI shim, then build/codegen tooling, then the Swift app, then Swift logic tests, then CI/docs rollout.

**Tech Stack:**
- Rust stable channel (inherited from `rust-toolchain.toml`)
- UniFFI 0.31 (proc-macro mode) + `uniffi-bindgen-swift`
- Swift 5.10 + SwiftUI (macOS 14+) + AppKit + CoreImage + Observation
- XcodeGen 2.x + SwiftLint (brew)
- XCTest
- GitHub Actions `macos-15`

**Reference spec:** Implements `docs/superpowers/specs/macos-app-and-uniffi-design.md`.

**Working directory note:** This plan runs on `main` alongside plan 01's history; single-developer repo. No worktree isolation required.

**Version drift policy:** Versions listed here are accurate as of 2026-04-21. If `cargo add` / `brew install` resolves to a higher minor version when executed, prefer the resolved version unless compilation fails.

---

## File structure (target end-state)

```text
minos/
├── .github/workflows/ci.yml                        [modified: add swift job]
├── .gitignore                                      [modified: Xcode + Brewfile.lock]
├── README.md                                       [modified: status update]
├── apps/
│   └── macos/
│       ├── Brewfile
│       ├── .swiftlint.yml
│       ├── project.yml
│       ├── Minos/
│       │   ├── MinosApp.swift
│       │   ├── Info.plist
│       │   ├── Generated/
│       │   ├── Presentation/
│       │   ├── Application/
│       │   ├── Domain/
│       │   ├── Infrastructure/
│       │   └── Resources/Assets.xcassets/
│       └── MinosTests/
├── crates/
│   ├── minos-domain/
│   ├── minos-pairing/
│   ├── minos-daemon/
│   └── minos-ffi-uniffi/
├── docs/adr/
│   └── 0007-xcodegen-for-macos-project.md
└── xtask/
    └── src/main.rs
```

---

## Current checkpoint

- Historical groundwork through **Phase E** is already implemented in the repo: `minos-domain` `ErrorKind`, UniFFI feature-gated derives, `minos-daemon` FFI-friendly refactor, `Subscription` callback bridge, `start_autobind`, `current_trusted_device`, and daemon-side UniFFI exports.
- The repo should be treated as **stopped at Task 11.5 close-out**, not beyond it. Current worktree evidence shows the `minos-pairing` custom-type bridge is in progress, but the required feature-gated validation and commit have not been cleanly landed yet.
- The first unfinished work after that close-out is the real `minos-ffi-uniffi` shim. `crates/minos-ffi-uniffi/src/lib.rs` still needs to move from the sentinel surface to the full exported bridge.
- From this checkpoint onward, execute at **phase granularity only**.

### Historical phase status

| Phase | Status | Outcome |
|---|---|---|
| A | Landed | `minos-domain` exposes `ErrorKind` and routes user-facing localization through it. |
| B | Landed | FFI-visible `minos-domain` types carry optional `uniffi` derives and per-crate scaffolding. |
| C | Landed | `minos-pairing` exports `QrPayload` / `TrustedDevice` under optional `uniffi`. |
| D | Landed | `minos-daemon` is FFI-friendly: `Arc<Inner>`, `stop(&self)`, `host/port`, `start_autobind`, `subscribe`, `current_trusted_device`, `logging::today()`. |
| E | Landed | `DaemonHandle`, `Subscription`, and `ConnectionStateObserver` are exported under the daemon `uniffi` feature. |
| C follow-up / Task 11.5 | In progress | `minos-pairing` still needs the custom-type bridge to be validated, committed, and treated as the starting point for the remaining work. |

---

## Phase dependency graph

```text
Historical path already landed:
  Phase A -> Phase B -> Phase C -> Phase D -> Phase E

Current execution path from this checkpoint:
  Task 11.5 close-out
    -> Phase F  Rust UniFFI bridge completion
    -> Phase G  Build / codegen / Xcode scaffold
    -> Phase H  Swift app implementation
    -> Phase I  Swift logic tests + local green gate
    -> Phase J  CI + docs rollout
```

### Phase execution rules

1. One implementation subagent owns one phase end-to-end.
2. A phase is not done until its listed validation commands pass.
3. Do not split a phase into multiple commits unless validation exposes a narrow repair inside that same phase.
4. The design spec remains the source of truth for behavior, layering, and UI scope; this plan optimizes execution order and commit boundaries.
5. If a phase needs a small adjacent config/doc change to make its own gate pass, keep that change in the same phase commit.

---

## Phase F · Rust UniFFI bridge completion

**Goal:** Close out the unfinished Rust-side bridge so Swift-facing bindings exist on a stable exported surface.

**Scope:**
- Land and validate the `Task 11.5` custom-type bridge in `crates/minos-pairing` and `crates/minos-domain`.
- Replace the `ping()` sentinel in `crates/minos-ffi-uniffi` with the full shim: custom types, remote newtype bridges, logging exports, error-kind message bridge, and daemon/type re-exports.
- Keep the entire Rust-side surface independently buildable before any macOS tooling or Swift app files are added.

**Files likely touched:**
- `crates/minos-pairing/Cargo.toml`
- `crates/minos-pairing/src/lib.rs`
- `crates/minos-domain/src/ids.rs`
- `crates/minos-ffi-uniffi/Cargo.toml`
- `crates/minos-ffi-uniffi/src/lib.rs`

**Preserved constraints:**
- Every crate carrying UniFFI derives keeps feature-gated `uniffi::setup_scaffolding!()` in its own `lib.rs`.
- `custom_type!(T, ...)` still requires a single-ident path, so `DateTime<Utc>` uses a local `DateTimeUtc` alias.
- Cross-crate type bridges keep the `remote` form; `custom_newtype!` is still not valid for remote registrations.
- `PairingToken.0` remains public if the remote lift/lower path needs constructor and field access across crates.

**Validation:**

```bash
cargo build -p minos-pairing --features uniffi
cargo build -p minos-daemon --features uniffi
cargo build -p minos-ffi-uniffi --release
cargo test -p minos-pairing
cargo test -p minos-domain
```

**Commit boundary:**

```bash
git add crates/minos-pairing crates/minos-domain/src/ids.rs crates/minos-ffi-uniffi
git commit -m "feat(uniffi): complete Rust-side Swift bridge and custom-type registration"
```

---

## Phase G · Build / codegen / Xcode scaffold

**Goal:** Make the repo able to build the universal static library, generate Swift bindings, generate the Xcode project, and carry the minimal macOS tooling scaffold in source control.

**Scope:**
- Implement `xtask build-macos`, `gen-uniffi`, `gen-xcode`, `check-all` Swift leg, and `bootstrap` Swift-tool installation.
- Add `.gitignore` entries for generated Xcode and xcframework artifacts.
- Add `apps/macos/Brewfile`, `apps/macos/.swiftlint.yml`, and `apps/macos/project.yml`; remove the `.gitkeep` placeholder.
- Ensure `uniffi-bindgen-swift` is the bindgen binary used by bootstrap/codegen.

**Files likely touched:**
- `xtask/src/main.rs`
- `.gitignore`
- `apps/macos/Brewfile`
- `apps/macos/.swiftlint.yml`
- `apps/macos/project.yml`
- `apps/macos/.gitkeep` (delete)

**Preserved constraints:**
- `apps/macos/project.yml` keeps the original target settings: `LSUIElement`, `ai.minos.macos`, macOS 14.0, `MinosCoreFFI.modulemap`, library search path, and strict concurrency.
- `Minos/Generated/` stays generated and excluded from SwiftLint.
- `build-macos` continues to produce `target/xcframework/libminos_ffi_uniffi.a` as a universal archive.

**Validation:**

```bash
cargo build -p xtask
cargo xtask build-macos
cargo xtask gen-uniffi
cargo xtask gen-xcode
```

**Commit boundary:**

```bash
git add xtask/src/main.rs .gitignore apps/macos/Brewfile apps/macos/.swiftlint.yml apps/macos/project.yml apps/macos/.gitkeep
git commit -m "feat(build): add macOS codegen, XcodeGen, and repo scaffold"
```

---

## Phase H · Swift app implementation

**Goal:** Land a buildable `MenuBarExtra` macOS app with the exact plan-02 UI surface: boot/error state, QR display, Forget flow, and today-log reveal.

**Scope:**
- Create the full four-layer Swift tree under `apps/macos/Minos/`.
- Implement the `DaemonDriving` seam, `ObserverAdapter`, `AppState`, `DaemonBootstrap`, QR rendering, diagnostics reveal, and display helpers.
- Implement `StatusIcon`, `QRSheet`, `MenuBarView`, `MinosApp`, and required app resources / plist.
- Stay within the design spec's UI-per-phase rule: no placeholder views for plan 03/04.

**Files likely touched:**
- `apps/macos/Minos/MinosApp.swift`
- `apps/macos/Minos/Info.plist`
- `apps/macos/Minos/Application/*`
- `apps/macos/Minos/Domain/*`
- `apps/macos/Minos/Infrastructure/*`
- `apps/macos/Minos/Presentation/*`
- `apps/macos/Minos/Resources/Assets.xcassets/*`

**Preserved constraints:**
- `AppState` depends on local `DaemonDriving`, not the generated concrete `DaemonHandle`.
- Error localization still goes through Rust's `ErrorKind` string table; Swift only owns the discriminant switch.
- QR rendering stays CoreImage-based and uses the serialized `QrPayload` JSON bytes.
- The plan-02 UI remains limited to boot-error, unpaired, paired, QR sheet, log reveal, and quit.

**Validation:**

```bash
cargo xtask gen-uniffi
cargo xtask gen-xcode
xcodebuild -project apps/macos/Minos.xcodeproj -scheme Minos -destination 'platform=macOS' -configuration Debug build
swiftlint --strict apps/macos
```

**Commit boundary:**

```bash
git add apps/macos/Minos
git commit -m "feat(macos): add MenuBarExtra app and presentation flow"
```

---

## Phase I · Swift logic tests + local green gate

**Goal:** Add the mock-driven Swift logic test suite and make the local macOS gate pass end-to-end.

**Scope:**
- Add `MockDaemon` and `AppStateTests` under `apps/macos/MinosTests/`.
- Keep tests logic-only; no Preview snapshot, widget, UI, or XCUITest coverage enters this phase.
- Make the app/test/build/lint path green under `cargo xtask check-all`.

**Files likely touched:**
- `apps/macos/MinosTests/TestSupport/MockDaemon.swift`
- `apps/macos/MinosTests/Application/AppStateTests.swift`
- Small adjacent fixes in `apps/macos/Minos/*` or `xtask/src/main.rs` if required by test/build feedback

**Preserved constraints:**
- Test doubles stay behind `DaemonDriving`; the test target should not need to instantiate the real Rust daemon.
- Any fallback seam introduced for `Subscription` or generated-type initialization stays local to testability and does not widen production scope.

**Validation:**

```bash
xcodebuild -project apps/macos/Minos.xcodeproj -scheme MinosTests -destination 'platform=macOS' -configuration Debug test
cargo xtask check-all
```

**Commit boundary:**

```bash
git add apps/macos/MinosTests apps/macos/Minos xtask/src/main.rs
git commit -m "test(macos): add AppState logic suite and green local verification"
```

---

## Phase J · CI + docs rollout

**Goal:** Add the `macos-15` CI lane and finish the plan-02 documentation surface.

**Scope:**
- Add the macOS job to `.github/workflows/ci.yml`.
- Add `docs/adr/0007-xcodegen-for-macos-project.md`.
- Update `README.md` to reflect plan-02 readiness / macOS app availability.
- Keep final wording cleanup to this plan doc in the same phase.

**Files likely touched:**
- `.github/workflows/ci.yml`
- `docs/adr/0007-xcodegen-for-macos-project.md`
- `README.md`
- `docs/superpowers/plans/02-macos-app-and-uniffi.md`

**Validation:**

```bash
cargo xtask check-all
```

**Commit boundary:**

```bash
git add .github/workflows/ci.yml docs/adr/0007-xcodegen-for-macos-project.md README.md docs/superpowers/plans/02-macos-app-and-uniffi.md
git commit -m "ci/docs: add macOS CI and finalize plan-02 docs"
```

---

## Final verification

### Automated gate

```bash
cargo xtask check-all
cargo xtask build-macos
cargo xtask gen-uniffi
cargo xtask gen-xcode
xcodebuild -project apps/macos/Minos.xcodeproj -scheme Minos -destination 'platform=macOS' -configuration Debug build
xcodebuild -project apps/macos/Minos.xcodeproj -scheme MinosTests -destination 'platform=macOS' -configuration Debug test
swiftlint --strict apps/macos
```

### Manual sanity gate

1. Launch `Minos.app` and confirm the menu-bar icon appears.
2. Confirm the unpaired layout shows `显示配对二维码…`.
3. Open the QR sheet and verify a QR renders, refresh works, and close dismisses the sheet.
4. Use `在 Finder 中显示今日日志…` and verify Finder reveals today's `.xlog` file.
5. Use `退出 Minos` and verify the app exits cleanly.
6. Verify `~/Library/Logs/Minos/daemon_YYYYMMDD.xlog` exists and is non-empty.

If any manual sanity check fails, open a follow-up task and do **not** mark plan 02 complete.

---

## Deliverables

| Deliverable | Entry point | Verification |
|---|---|---|
| Rust daemon FFI-friendly surface | `crates/minos-daemon/` | `cargo test -p minos-daemon` |
| UniFFI shim | `crates/minos-ffi-uniffi/src/lib.rs` | `cargo build -p minos-ffi-uniffi --release` |
| Swift bindings codegen | `cargo xtask gen-uniffi` | generated `apps/macos/Minos/Generated/*` |
| XcodeGen project | `apps/macos/project.yml` | `cargo xtask gen-xcode` |
| macOS app | `apps/macos/Minos/MinosApp.swift` | `xcodebuild -scheme Minos build` + manual sanity |
| Swift logic tests | `apps/macos/MinosTests/` | `xcodebuild -scheme MinosTests test` |
| CI lane | `.github/workflows/ci.yml` | GitHub Actions `macos-15` |
| ADR 0007 | `docs/adr/0007-xcodegen-for-macos-project.md` | review |
| README update | `README.md` | review |

Plan 02 is complete only when all phases F through J are committed and the automated + manual verification gates above pass.
