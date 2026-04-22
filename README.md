# Minos

Native macOS status-bar app + Flutter mobile client + shared Rust core for remote AI-coding control. Drive `codex` / `claude` / `gemini` on a Mac from a paired phone over Tailscale.

## Status

Plan 02 is ready in-repo: the macOS MenuBarExtra app, UniFFI bridge, XcodeGen project spec, Swift logic tests, and macOS CI lane are all wired. The Flutter/mobile side remains later work. See `docs/superpowers/specs/minos-architecture-and-mvp-design.md` for the overall product design and `docs/superpowers/plans/` for execution plans.

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
