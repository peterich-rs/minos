# 0001 · Monorepo Layout

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-21 |
| Deciders | fannnzhang |

## Context

Minos is a polyglot project: a Rust workspace (multiple crates), a SwiftUI macOS status-bar app, and a Flutter mobile app. The Rust workspace must serve both the Mac side (UniFFI to Swift) and the iOS side (frb to Dart). Two FFI crates are physically required because UniFFI and frb cannot share a single cdylib (panic-handling and runtime initialization conflict).

The repository is fresh. Layout choice now sets the cost of every future build, CI workflow, contributor onboarding, and platform addition.

## Decision

A flat monorepo with two top-level siblings, `crates/` and `apps/`, plus `xtask/`, `docs/`, `scripts/`, and `.github/`. The Cargo workspace lives at the repository root.

```
minos/
├── Cargo.toml                      # workspace
├── rust-toolchain.toml
├── crates/
│   ├── minos-domain/
│   ├── minos-protocol/
│   ├── minos-pairing/
│   ├── minos-cli-detect/
│   ├── minos-transport/
│   ├── minos-daemon/
│   ├── minos-mobile/
│   ├── minos-ffi-uniffi/
│   └── minos-ffi-frb/
├── xtask/
├── apps/
│   ├── macos/                      # Xcode project (MenuBarExtra app)
│   └── mobile/                     # Flutter project
├── docs/
│   ├── superpowers/specs/
│   └── adr/
├── scripts/                        # bootstrap shell only
└── .github/workflows/
```

## Consequences

**Positive**
- Single Cargo workspace gives one `Cargo.lock`, one `target/`, and `cargo {fmt,clippy,test} --workspace` works without flags.
- `apps/` is open-ended: future `apps/linux/`, `apps/windows/`, `apps/cli/`, `apps/web/` slot in without restructure.
- Conventional shape for Rust + native polyglot projects (Bitwarden, Signal-Server). Low onboarding friction for contributors familiar with Rust ecosystem.
- `xtask` keeps build orchestration in Rust (typed, cross-platform) instead of shell.

**Neutral**
- The Xcode project sits inside the repo at `apps/macos/`; Xcode tolerates this fine but expects relative paths in the build scripts under `BuildSupport/`.
- frb v2 generates `apps/mobile/rust_builder/`; the Rust source lives at `crates/minos-ffi-frb/`. Bridged via `flutter_rust_bridge.yaml` `rust_input` setting — one line of config, no friction beyond.

## Alternatives Rejected

### Per-platform top-level (`core/` + `desktop/` + `mobile/`)

```
minos/
├── core/                # Cargo workspace inside
├── desktop/macos/
├── mobile/
└── docs/
```

Rejected:
- Every cargo command needs `cd core/` or `--manifest-path core/Cargo.toml`. Small papercut compounds across hundreds of invocations daily.
- "desktop" is ambiguous — does it include only macOS, or future Linux/Windows? Reorganizing later is more disruptive than starting flat.
- Real benefit (per-platform team isolation) is moot at single-developer scale.

### Polyrepo with submodules

`peterich-rs/minos-core` + `peterich-rs/minos-mac` + `peterich-rs/minos-mobile` linked via git submodules or version-pinned releases.

Rejected:
- Three repositories means three CI configurations and three release cadences. Premature optimization at MVP scale.
- Cross-repo changes require a version-pin dance for every refactor that crosses repo lines (which is most of them while the architecture is still stabilizing).
- Single-repo `cargo` workspace ergonomics are forfeited.
