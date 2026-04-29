# Minos Mobile

Flutter shell for the Minos mobile client.

## Build & run

All commands go through `just` from the workspace root. See the workspace
README for one-time setup (`cp .env.example .env.local`).

```sh
# Production iOS build (Release configuration).
just build-mobile-ios Release

# Hot-reload dev workflow on a simulator or attached device.
just dev-mobile-ios

# Android stub (placeholder; not currently part of the supported surface).
just build-mobile-android
```

Direct `flutter run` and Xcode IDE Build/Run are still wired through the same
env path: Cargokit's Rust build script re-enters `just` before invoking cargo,
so `.env.local` is loaded before `option_env!` is evaluated. Prefer the public
recipes above for normal work because they include the project-level checks and
documented flags.

## Configuration

`MINOS_BACKEND_URL` and `CF_ACCESS_CLIENT_*` are baked at build time
from `.env.local` (workspace root). The Rust FFI reads them via
`option_env!`; the Dart layer reads CF Access via `String.fromEnvironment`
which `flutter run` populates with `--dart-define` (the just recipe wires
both paths from the same `.env.local`).

iOS Keychain (`flutter_secure_storage`) holds only Minos business state:
`device_id`, `device_secret`, `account_id`, refresh tokens — never the
backend URL or CF Access tokens.
