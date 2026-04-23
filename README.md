# Minos

Native macOS status-bar app + Flutter mobile client + shared Rust core for remote AI-coding control. Drive `codex` / `claude` / `gemini` on a Mac from a paired phone over Tailscale.

## Status

Plans 01–04 are ready in-repo.

- **Plan 02** — macOS MenuBarExtra app, UniFFI bridge, XcodeGen project spec, Swift logic tests, macOS CI lane.
- **Plan 03** — Flutter iOS app under `apps/mobile/` with `flutter_rust_bridge` v2 bindings over `minos-mobile::MobileClient`, Riverpod-codegen state layer, `shadcn_ui` UI, `mobile_scanner` QR capture, Dart-side `mars-xlog` via `peterich-rs/xlog-rs`, and the pair-over-Tailscale pipeline. Tier A scope: iOS scans macOS QR → `pair` JSON-RPC → WebSocket connected. `cargo xtask check-all` covers Rust + Swift + Flutter legs with an frb codegen drift guard. Real-device smoke (MVP spec §8.4 items 1–5) is the last gate and is driven manually — see `docs/superpowers/plans/03-flutter-app-and-frb-pairing.md` §Phase F.
- **Plan 04** — `codex app-server` loopback integration via `minos-agent-runtime`, daemon-side `subscribe_events` streaming plus `start_agent` / `send_user_message` / `stop_agent` RPCs, and a debug-build macOS menubar Agent segment for maintainer smoke testing.

Tier B (list_clis in Dart, auto-reconnect, Keychain-backed pairing store, "Forget this Mac") lives in a future `ios-mvp-completion-design.md` spec.

## Roadmap

The next P1 surface is the streaming chat UI and the mobile-side consumer for the landed agent RPC/event stream. Until that exists, the macOS app exposes debug-build-only menubar controls to start Codex, send a test ping, and stop the session locally.

See `docs/superpowers/specs/minos-architecture-and-mvp-design.md` for the overall product design, `docs/superpowers/specs/flutter-app-and-frb-pairing-design.md` for the iOS Tier A design, and `docs/superpowers/plans/` for execution plans.

## Quick start (development)

```bash
# Bootstrap dev tools.
# On macOS this also installs xcodegen and swiftlint from apps/macos/Brewfile.
cargo xtask bootstrap

# Run all checks.
# On macOS this includes UniFFI/XcodeGen generation, xcodebuild, MinosTests,
# and swiftlint in addition to the Rust workspace checks.
cargo xtask check-all
```

## macOS app

The macOS app lives in `apps/macos/` and uses XcodeGen plus UniFFI-generated Swift bindings.

```bash
# Build the universal Rust static library used by Xcode.
cargo xtask build-macos

# Regenerate Swift bindings and the Xcode project.
cargo xtask gen-uniffi
cargo xtask gen-xcode

# Open the generated project in Xcode.
open apps/macos/Minos.xcodeproj
```

## Rust daemon CLI

For faster Rust-side validation, `minos-daemon` now has a direct CLI entrypoint.
By default, the CLI keeps its runtime files under `~/.minos/` so ad hoc testing
doesn't mix with the macOS app's platform-native paths.

```bash
# Show resolved paths and the current Tailscale discovery result.
cargo run -p minos-daemon -- doctor

# Start the daemon without Tailscale, using an ephemeral loopback port.
cargo run -p minos-daemon -- start --bind 127.0.0.1:0 --print-qr

# Inspect what the library would use on macOS without the CLI overrides.
cargo run -p minos-daemon -- --platform-paths doctor
```

On this repository's current MVP path, production pairing still assumes a
Tailscale `100.x.y.z` address between the Mac and the phone.

## Mobile app (iOS)

The Flutter app lives in `apps/mobile/`. Flutter is pinned to `3.41.6` via `apps/mobile/.fvmrc` and managed through [fvm](https://fvm.app).

```bash
# First-time: bootstrap prepares flutter_rust_bridge_codegen, iOS rustup targets,
# runs `fvm flutter pub get`, and primes Riverpod codegen.
cargo xtask bootstrap

# Regenerate the Dart ↔ Rust frb bindings after changing minos-ffi-frb.
cargo xtask gen-frb

# Build iOS staticlibs (device + simulator).
cargo xtask build-ios

# Open the iOS project in Xcode (requires an Apple Developer team for real-device signing).
open apps/mobile/ios/Runner.xcodeproj
```

During development without a real device, start `cargo run -p minos-daemon -- start --print-qr` on the Mac, copy the printed QR JSON, and paste it in the iOS Simulator via the `kDebugMode`-only FAB on the pairing page.

## Repository layout

```
crates/    Rust workspace (9 crates: domain, protocol, pairing, cli-detect,
           transport, daemon, mobile, ffi-uniffi, ffi-frb)
apps/      macOS (SwiftUI/UniFFI, XcodeGen-managed) and mobile (Flutter/frb)
xtask/     Build / codegen orchestration in Rust
docs/      Specs (`docs/superpowers/specs/`) and ADRs (`docs/adr/`)
```

## License

MIT — see `LICENSE`.
