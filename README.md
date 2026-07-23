# Minos

Native macOS status-bar app + Flutter mobile client + shared Rust core for remote AI-coding control. Drive `codex` / `claude` / `gemini` on a Mac from a paired phone.

## Status

Minos is moving from MVP delivery into formal development.

- Historical planning docs under `docs/superpowers/` have been retired.
- The active backend architecture source of truth is `docs/backend-formal-development.md`.
- Current shipped surfaces are the hosted Rust backend, the macOS host app / daemon, the Flutter mobile client, and the browser-admin web client.

## Roadmap

The next backend-facing work is to replace the remaining MVP runtime seams with the formal design in `docs/backend-formal-development.md`, while keeping the existing product scope: account auth, host pairing, agent sessions, approvals, conversations, and projects.

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

# Print a one-shot runtime status snapshot after the relay link comes up.
cargo run -p minos-daemon -- status

# Mint a fresh pairing QR as JSON without leaving a long-running daemon
# process behind.
cargo run -p minos-daemon -- pairing-qr

# List every paired mobile/account row currently attached to this host.
cargo run -p minos-daemon -- peers

# Detect the host-side CLI agents Minos can launch.
cargo run -p minos-daemon -- list-clis

# Inspect host skills for one workspace, or toggle one explicit skill path.
cargo run -p minos-daemon -- host-skills --workspace ~/dev/my-repo
cargo run -p minos-daemon -- set-host-skill ~/.codex/skills/my-skill enable

# Forget the first paired row, or target one explicitly with --device-id.
cargo run -p minos-daemon -- forget-peer
cargo run -p minos-daemon -- forget-peer --device-id <mobile-device-uuid>

# Read persisted session summaries or one session snapshot from the local store.
cargo run -p minos-daemon -- sessions --limit 20
cargo run -p minos-daemon -- thread <thread-id>
cargo run -p minos-daemon -- history <thread-id>

# Run one local codex prompt against the current repo and stream the reply.
cargo run -p minos-daemon -- run "Summarize this workspace"
cargo run -p minos-daemon -- run --thread <thread-id> "Continue this session"

# Keep a local codex session open for multiple turns.
cargo run -p minos-daemon -- chat --workspace ~/dev/my-repo

# Re-attach to a persisted local session.
cargo run -p minos-daemon -- chat --thread <thread-id>

# Start the daemon against the relay. Needs a reachable relay — boot a
# local one first with `cargo run -p minos-relay`, or point to a hosted
# one at build time. Pass `--print-qr` to mint a pairing QR once the
# link is up.
cargo run -p minos-daemon -- start --print-qr

# Inspect what the library would use on macOS without the CLI overrides.
cargo run -p minos-daemon -- --platform-paths doctor
```

`device-secret` persistence is file-backed under `~/.minos/secrets/` (or
`$MINOS_HOME/secrets/`). The daemon no longer mirrors this value into the macOS
Keychain, so pairing should not trigger login-keychain password prompts. When
you pass `--minos-home`, the CLI now forwards that override into the daemon's
SQLite/workspaces runtime instead of only affecting `local-state.json`.

Agent teamwork uses the `minos-teamwork-mcp` sidecar. Runtime lookup checks
`MINOS_TEAMWORK_MCP_BIN`, then a sibling binary next to the running executable,
then `PATH`; if no executable is found, MCP injection is skipped and the agent
session continues without teamwork tools. For source builds, either build the
sidecar with `cargo build -p minos-chat-store --bin minos-teamwork-mcp` or set
`MINOS_TEAMWORK_MCP_BIN=/abs/path/to/minos-teamwork-mcp`.

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
just dev-mobile-android        # debug + flutter run on an Android device/emulator
just build-mobile-ios Release  # production-flavoured build
just build-mobile-android      # Android release APK
```

Direct `flutter run`, `flutter build`, and Xcode IDE Build/Run now
self-bootstrap the Rust FFI compile through `just` via Cargokit, so
`.env.local` is still loaded before `option_env!` is evaluated. Prefer the
public `just` recipes for normal work because they also run the project-level
validation and documented build flags. See ADR 0018.

During development without a real device: the Mac-side relay flow has a dev
bin — see `just smoke-fake-peer register` (or `pair` / `smoke-session`),
which drives the relay end-to-end without an iPhone.

## Mobile login + agent session

The current account-based login + agent session flow is:

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
`docs/backend-formal-development.md` for the backend runtime direction
and `docs/adr/0018-just-config-pipeline.md` for the build-through-just
policy.

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

## Web app

The browser admin client lives in `apps/web/`.

```bash
# First-time:
cd apps/web
cp .env.example .env.local
pnpm install

# Run locally:
just dev-web

# Verify:
just check-web
```

Set `VITE_MINOS_BACKEND_URL` when the backend is not running on `http://127.0.0.1:8787`.

## Repository layout

```
crates/    Rust workspace (9 crates: domain, protocol, pairing, cli-detect,
           transport, daemon, mobile, ffi-uniffi, ffi-frb)
apps/      macOS (SwiftUI/UniFFI, XcodeGen-managed) and mobile (Flutter/frb)
          plus the standalone web admin client (React/Vite)
xtask/     Build / codegen orchestration in Rust
docs/      Active architecture docs plus ADRs and operations runbooks
```

## License

MIT — see `LICENSE`.
