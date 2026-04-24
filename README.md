# Minos

Native macOS status-bar app + Flutter mobile client + shared Rust core for remote AI-coding control. Drive `codex` / `claude` / `gemini` on a Mac from a paired phone. The macOS app (plan 05) talks to the `minos-relay` WSS broker behind Cloudflare Access; iOS / Flutter is still on the Tier A Tailscale pipeline until plan 06 ports it.

## Status

Plans 01–04 are ready in-repo.

- **Plan 02** — macOS MenuBarExtra app, UniFFI bridge, XcodeGen project spec, Swift logic tests, macOS CI lane.
- **Plan 03** — Flutter iOS app under `apps/mobile/` with `flutter_rust_bridge` v2 bindings over `minos-mobile::MobileClient`, Riverpod-codegen state layer, `shadcn_ui` UI, `mobile_scanner` QR capture, Dart-side `mars-xlog` via `peterich-rs/xlog-rs`, and the pair-over-Tailscale pipeline. Tier A scope: iOS scans macOS QR → `pair` JSON-RPC → WebSocket connected. `cargo xtask check-all` covers Rust + Swift + Flutter legs with an frb codegen drift guard. Real-device smoke (MVP spec §8.4 items 1–5) is the last gate and is driven manually — see `docs/superpowers/plans/03-flutter-app-and-frb-pairing.md` §Phase F.
- **Plan 04** — `codex app-server` loopback integration via `minos-agent-runtime`, daemon-side `subscribe_events` streaming plus `start_agent` / `send_user_message` / `stop_agent` RPCs, and a debug-build macOS menubar Agent segment for maintainer smoke testing.
- **Plan 05** (this branch) — Mac app migrated from Tailscale P2P to `minos-relay` WSS client. Tailscale code (`tailscale.rs`, `WsServer`, port-retry autobind) removed; CF Service Token onboarding via two-field Keychain sheet; backend URL baked at compile time via `option_env!("MINOS_BACKEND_URL")`; connection state split into `RelayLinkState` + `PeerState` (two orthogonal axes); new dev bin `cargo run -p minos-mobile --bin fake-peer --features cli` for end-to-end smoke without iOS. iOS / Flutter remain on the legacy stack until plan 06 ports them onto the relay.

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
# Show resolved paths, the local-state.json location, and the compile-time
# relay backend URL (overridable at build time via MINOS_BACKEND_URL).
cargo run -p minos-daemon -- doctor

# Start the daemon against the relay. Needs a reachable relay — boot a
# local one first with `cargo run -p minos-relay`, or point to a hosted
# one at build time. Pass `--print-qr` to mint a pairing QR once the
# link is up.
cargo run -p minos-daemon -- start --print-qr

# Inspect what the library would use on macOS without the CLI overrides.
cargo run -p minos-daemon -- --platform-paths doctor
```

CF Service Token credentials come from `CF_ACCESS_CLIENT_ID` /
`CF_ACCESS_CLIENT_SECRET` for the CLI; the macOS app reads them from the
Keychain (written via the in-app Settings sheet).

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

During development without a real device: iOS Tier A still uses the pre-relay Tailscale pair flow, so spin up the legacy minos-daemon stack on that side. The Mac-side relay flow has its own dev bin instead — see `cargo run -p minos-mobile --bin fake-peer --features cli`, which drives the relay end-to-end without an iPhone.

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
