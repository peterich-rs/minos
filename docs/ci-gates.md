# CI / local quality gates

Single source of truth for what runs where. CI jobs call the same xtask/`just`
commands developers use; YAML only adds tool-only checks (sqlx offline metadata)
and the Postgres service smoke.

The Swift `apps/macos` host UI and UniFFI bridge have been removed. Host GUI is
`apps/desktop` (Tauri). Host runtime remains `minos-daemon`.

## Local commands

| Command | What it runs |
|---------|----------------|
| `just check` / `cargo xtask check-all` | `check-rust` → frb drift → (macOS) `cargo check -p minos-desktop` → full Flutter when fvm is present |
| `just check-rust` / `cargo xtask check-rust` | phased rust gate (see below) |
| `just check-backend` | platform contract + schema-parity + backend tests + daemon `test-support` |
| `just check-backend-pg` | schema-parity + `MINOS_PG_TESTS=1` Postgres smokes |
| `just check-web` | `apps/web` `pnpm check` |
| `just check-desktop` | `apps/desktop` `pnpm check:all` |

Stub lints (`lint-conventions`, `lint-metrics`) remain as standalone xtask
commands but are **not** part of any gate until implemented.

## Phase order (cheapest first)

Every composite gate runs **static → compile → test** so cheap failures exit
before clippy / the suite / Flutter.

### `check-rust`

1. **static** — `cargo fmt --check` → lint-naming → lint-docs → backend platform schema drift → schema-parity
2. **compile** — `cargo clippy --workspace --all-targets --exclude minos-desktop -D warnings`
3. **test** — `cargo test --workspace --exclude minos-desktop` → `cargo test -p minos-daemon --features test-support`

### local `check-all` (after `check-rust`)

1. frb codegen drift (static/codegen; skips if codegen binary missing)
2. on macOS: `cargo check -p minos-desktop`
3. Flutter full when fvm is present: format → analyze → build host dylib → `flutter test`
4. optional codex smoke (`--with-codex`)

## CI matrix (`.github/workflows/ci.yml`)

| Job | Runner | Command / steps | Owns |
|-----|--------|-----------------|------|
| `backend` | ubuntu | sqlx prepare --check → `cargo xtask check-rust` | Linux x64 Rust quality, daemon integration tests, schema-parity |
| `backend-pg` | ubuntu + postgres:16 | `needs: [backend]` → schema-parity → `pg_migration` + `pg_contract_smoke` | Production dialect smoke |
| `mobile` | ubuntu | format `lib`+`test`, analyze, test `--exclude-tags ffi`, frb drift | Mobile Dart gates + FRB drift |
| `web` | ubuntu | `apps/web` `pnpm check` | Web admin |
| `desktop` | macos-15 | `needs: [backend]` → `apps/desktop` `pnpm check:all` + `cargo check -p minos-desktop` | Desktop TS + Tauri host compile |

### Explicit non-goals / split ownership

- **No Swift host app / UniFFI / Xcode lane** in CI.
- **`minos-desktop` Rust** is excluded from workspace clippy/test on Linux (GUI deps) and compile-checked on the macOS `desktop` job.
- **Daemon `test-support` tests** live inside `check-rust` / the `backend` job.
- **iOS/Android native packaging** is not a PR gate; use `just build-mobile-*` / release workflows.
- **`cargo deny`** is configured (`deny.toml`) but not yet wired; track separately.

## Version pins

Workflow-level `env` in `ci.yml`:

- `RUST_TOOLCHAIN` — must match `rust-toolchain.toml`
- `FLUTTER_VERSION` — must match `apps/mobile/.fvmrc`
- `FRB_CODEGEN_VERSION` — must match workspace `flutter_rust_bridge` / bootstrap
- `SQLX_CLI_VERSION` — sqlx-cli install pin
