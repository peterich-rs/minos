# Minos

Native macOS status-bar app + Flutter mobile client + shared Rust core for remote AI-coding control. Drive `codex` / `claude` / `gemini` on a Mac from a paired phone. The macOS app (plan 05) talks to the `minos-relay` WSS broker behind Cloudflare Access; iOS / Flutter is still on the Tier A Tailscale pipeline until plan 06 ports it.

## Status

Plans 01–04 are ready in-repo.

- **Plan 02** — macOS MenuBarExtra app, UniFFI bridge, XcodeGen project spec, Swift logic tests, macOS CI lane.
- **Plan 03** — Flutter iOS app under `apps/mobile/` with `flutter_rust_bridge` v2 bindings over `minos-mobile::MobileClient`, Riverpod-codegen state layer, `shadcn_ui` UI, `mobile_scanner` QR capture, Dart-side `mars-xlog` via `peterich-rs/xlog-rs`, and the pair-over-Tailscale pipeline. Tier A scope: iOS scans macOS QR → `pair` JSON-RPC → WebSocket connected. `cargo xtask check-all` covers Rust + Swift + Flutter legs with an frb codegen drift guard. Real-device smoke (MVP spec §8.4 items 1–5) is the last gate and is driven manually — see `docs/superpowers/plans/03-flutter-app-and-frb-pairing.md` §Phase F.
- **Plan 04** — `codex app-server` loopback integration via `minos-agent-runtime`, daemon-side `subscribe_events` streaming plus `start_agent` / `send_user_message` / `stop_agent` RPCs, and a debug-build macOS menubar Agent segment for maintainer smoke testing.
- **Plan 05** (this branch) — Mac app migrated from Tailscale P2P to `minos-relay` WSS client. Tailscale code (`tailscale.rs`, `WsServer`, port-retry autobind) removed; Cloudflare Access Service Tokens are supplied from client environment/build configuration instead of user-entered Keychain forms; backend URL is baked at compile time via `option_env!("MINOS_BACKEND_URL")`; connection state split into `RelayLinkState` + `PeerState` (two orthogonal axes); new dev bin `cargo run -p minos-mobile --bin fake-peer --features cli` supports end-to-end smoke without iOS.

Tier B (list_clis in Dart, auto-reconnect, Keychain-backed pairing store, "Forget this Mac") lives in a future `ios-mvp-completion-design.md` spec.

## Roadmap

The next P1 surface is the streaming chat UI and the mobile-side consumer for the landed agent RPC/event stream. Until that exists, the macOS app exposes debug-build-only menubar controls to start Codex, send a test ping, and stop the session locally.

See `docs/superpowers/specs/minos-architecture-and-mvp-design.md` for the overall product design, `docs/superpowers/specs/flutter-app-and-frb-pairing-design.md` for the iOS Tier A design, and `docs/superpowers/plans/` for execution plans.

## Quick start (development)

```bash
# Bootstrap dev tools.
# On macOS this also installs xcodegen and swiftlint from apps/macos/Brewfile.
cargo xtask bootstrap

# Configure runtime/build env loaded by just.
cp .env.example .env.local

# Run all checks.
# On macOS this includes UniFFI/XcodeGen generation, xcodebuild, MinosTests,
# and swiftlint in addition to the Rust workspace checks.
just check
```

## macOS app

The macOS app lives in `apps/macos/` and uses XcodeGen plus UniFFI-generated Swift bindings.

```bash
# Build the app through Xcode with .env.local loaded by just.
just build-macos Debug

# Regenerate Swift bindings and the Xcode project.
cargo xtask gen-uniffi
cargo xtask gen-xcode

# Open the generated project in Xcode.
open apps/macos/Minos.xcodeproj
```

The generated Xcode project calls back into `just` before compiling the app
target, so Xcode IDE Build/Run loads `.env.local` before Rust evaluates
`option_env!`. A post-build phase patches the built app's `Info.plist` with the
same runtime relay values for Finder/Xcode launches.

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

# Open the iOS workspace in Xcode (requires an Apple Developer team for real-device signing).
open apps/mobile/ios/Runner.xcworkspace
```

For a real-device install that still launches from the Home Screen after you
force-quit it, the public recipes are:

```bash
just dev-mobile-ios            # debug + flutter run hot-reload
just build-mobile-ios Release  # production-flavoured build
```

Direct `flutter run`, `flutter build`, and Xcode IDE Build/Run now
self-bootstrap the Rust FFI compile through `just` via Cargokit, so
`.env.local` is still loaded before `option_env!` is evaluated. Prefer the
public `just` recipes for normal work because they also run the project-level
validation and documented build flags. See
`docs/superpowers/specs/unified-config-pipeline-design.md` and ADR 0018.

During development without a real device: the Mac-side relay flow has a dev
bin — see `just smoke-fake-peer register` (or `pair` / `smoke-session`),
which drives the relay end-to-end without an iPhone.

## Mobile login + agent session

Plan 08 (`docs/superpowers/plans/08-mobile-auth-and-agent-session.md` + 08a/08b) introduced account-based login and the `start_agent` dispatch surface to the mobile client. End-to-end the flow is:

1. **Register or log in** — the iOS client (or `fake-peer`) calls `POST /v1/auth/register` or `/v1/auth/login` against the backend, which returns an access + refresh token tuple plus an `account_id`.
2. **Pair** — once authenticated, the iPhone scans the Mac's QR (v2 payload), POSTs `/v1/pairing/consume` with the bearer, and persists the freshly minted `DeviceSecret`. Same-device subsequent runs re-use the secret; switching accounts on a previously-paired device drops the pairing automatically (`MinosCore._onAuthLanded`).
3. **`start_agent`** — the iPhone opens an authenticated `/devices` WebSocket, then forwards `minos_start_agent` (and follow-up `minos_send_user_message`) to the Mac via `Envelope::Forward`. The daemon replies with a `session_id`; live `EventKind::UiEventMessage` frames stream back over the same socket.

### Local setup

```sh
# 1. Install the task runner (one-time):
brew install just  # or: cargo install just

# 2. Configure environment (one-time):
cp .env.example .env.local
# Edit .env.local: at minimum set MINOS_BACKEND_URL and MINOS_JWT_SECRET.
# Generate a JWT secret: openssl rand -hex 32

# 3. Run the backend:
just backend

# 4. Smoke a fake peer (in another terminal):
just smoke-fake-peer register

# 5. Build the mobile app:
just build-mobile-ios Release
```

All documented build and run commands go through `just`. The macOS Xcode
project and mobile Cargokit scripts also call back into `just` internally, so
IDE launches and direct Flutter builds still load `.env.local` for the Rust
compile instead of silently baking localhost.

`minos-backend` requires `MINOS_JWT_SECRET` (32+ bytes) at startup;
`just backend` enforces it before invoking cargo. See
`docs/superpowers/specs/unified-config-pipeline-design.md` for the
config pipeline design and `docs/adr/0018-just-config-pipeline.md` for
the policy decision.

### Dev smoke without an iPhone

`just smoke-fake-peer <kind>` wraps `cargo run -p minos-mobile --bin
fake-peer --features cli -- <kind> --backend "$MINOS_BACKEND_URL"`:

- `pair` — login-or-register + pair-only; tails inbound frames until the socket closes.
- `register` — strict register + pair; surfaces `EmailTaken` instead of falling through to login.
- `smoke-session` — full register-or-login → pair → `start_agent` loop; tails `UiEventFrame`s on stderr until interrupted.

For per-subcommand flags (e.g. `--email`, `--password`, `--token`,
`--prompt`), invoke the bin directly:

```bash
cargo run -p minos-mobile --bin fake-peer --features cli -- smoke-session \
    --backend "$MINOS_BACKEND_URL" \
    --email fan+smoke@example.com \
    --password Sup3rSecret! \
    --token <token-from-mac-qr> \
    --prompt "Hello from fake-peer" \
    --device-name "Fan's fake iPhone"
```

The in-process e2e regression for the same path lives in `crates/minos-mobile/tests/e2e_register_login_dispatch_start_agent.rs`.

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
