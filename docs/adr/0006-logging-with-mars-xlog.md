# 0006 · Logging with mars-xlog (Dogfooding peterich-rs/xlog-rs)

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-21 |
| Deciders | fannnzhang |

## Context

Minos needs structured logging that flows into:
- Compressed log files on Mac (`~/Library/Logs/Minos/`).
- Compressed log files on iOS (app `Documents/Minos/Logs/`).
- Native console (Console.app on Mac, OSLog on iOS) for live development inspection.
- A decoder pipeline for ops debugging post-incident.
- Both the Rust core and the language hosts (Swift, Dart) with consistent format.

The peterich-rs organization owns `xlog-rs`, a Rust workspace whose release surface is `mars-xlog` — a Rust-native re-implementation of Tencent Mars XLog. It ships:
- `mars-xlog` Rust crate with `tracing-subscriber::Layer` integration (`XlogLayer`).
- `mars-xlog-uniffi` for Swift / Kotlin consumption.
- `packages/xlog` Flutter / Dart package using native_assets + Rust cdylib.
- Dynamic level toggling at runtime (`XlogLayerHandle`).
- mmap async write, zlib compression, and Mars-compatible decoder scripts (`decode_mars_nocrypt_log_file.py` works on output).

## Decision

Adopt `mars-xlog` for both Rust sides (daemon and mobile core) and `packages/xlog` for the Flutter Dart side. Swift continues to use `OSLog` for system-level lifecycle events (it complements rather than competes with mars-xlog).

| Boundary | Library | Sink | `name_prefix` |
|---|---|---|---|
| Rust daemon (Mac process) | `tracing` + `mars_xlog::XlogLayer` | `~/Library/Logs/Minos/` | `daemon` |
| Rust core (iOS process) | `tracing` + `mars_xlog::XlogLayer` | iOS app `Documents/Minos/Logs/` | `mobile-rust` |
| Swift app | `OSLog` subsystem `ai.minos.macos` | Console.app + Unified log | — |
| Flutter app | `package:xlog` (peterich-rs/xlog Dart package) | iOS app `Documents/Minos/Logs/` | `mobile-flutter` |

## Consequences

**Positive — co-evolution feedback loop**
- Both repositories are owned by peterich-rs. Any pain point Minos hits in mars-xlog becomes a direct upstream issue / PR. The feedback latency is essentially zero.
- Minos becomes a real-world reference integration for mars-xlog: Tailscale + WebSocket long-connection + Codex high-frequency streaming output. mars-xlog gets dogfooded under conditions that would be hard to synthesize.
- Coordinated changes can be developed in parallel: switch the workspace temporarily to a path or git-ref override (`[patch.crates-io] mars-xlog = { path = "../xlog-rs/crates/xlog" }`), confirm the integration, publish a new mars-xlog version, switch Minos back to crates.io.

**Positive — engineering**
- Drop-in `tracing::Layer` integration: replacing the originally planned `tracing-appender` setup is a one-line subscriber registration change.
- mmap async write means logs survive even if the app is killed mid-write — relevant for iOS app suspension scenarios.
- Zlib compression keeps log file size manageable on both platforms.
- The decoder pipeline is already proven (Tencent's Mars decoders work).
- iOS / macOS native console semantics are preserved by mars-xlog's design: `printf` / `NSLog` / `OSLog` continue to behave normally.

**Neutral / accepted cost**
- Occasionally a Minos feature will block on a small upstream change in mars-xlog. Accepted because both repositories are under the same organization — turnaround is minimal.
- `mars-xlog` is a new project (low star count, recent release). Mitigated by the direct maintainer relationship.
- Single-writer constraint per `(name_prefix, log_dir)` pair: same directory may have multiple writers only with distinct prefixes. Not a problem for our matrix (each role has its own prefix); explicit constraint to remember when adding new components.

### Operational notes

- When a Minos issue touches the logging layer, open an upstream issue in `xlog-rs` first. Link the Minos issue to the upstream issue.
- When iterating across both repos, use `[patch.crates-io]` in `Cargo.toml` to point mars-xlog at a local checkout. Remove the patch once the upstream change is published and the published version is referenced.
- Document the decoder workflow in the Minos README (how to take a `.xlog` file from a user-submitted bug report and decode it locally).

## Alternatives Rejected

### `tracing-appender` + custom OSLog bridge

The "boring" alternative: `tracing-subscriber` + `tracing-appender` for daily-rolling JSON log files, plus a hand-written OSLog bridge for Swift-side observability.

Rejected:
- Loses mmap, compression, and the Mars decoder pipeline. We would re-invent these poorly.
- Hand-written OSLog bridge is duplicated effort against `mars-xlog-uniffi`'s existing implementation.
- No reciprocal value: nothing flows back to a library we own.

### `OSLog` only (no file logging)

Rejected:
- Long-tail debugging requires file logs: a user reports a bug an hour after it happened, and the OSLog ring buffer is gone.
- OSLog viewer (`log` command, Console.app) is cumbersome for ops scenarios where we want to grep / process structured fields.
