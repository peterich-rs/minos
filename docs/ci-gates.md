# CI / local quality gates

Single source of truth for what runs where. CI jobs call the same xtask/`just`
commands developers use; YAML does not invent extra `cargo test -p …` steps
except for tool-only checks (sqlx offline metadata) and the narrow Windows
portability lane.

## Local commands

| Command | What it runs |
|---------|----------------|
| `just check` / `cargo xtask check-all` | `check-rust` + (macOS) native/Swift/`minos-desktop` + full Flutter when fvm is present + frb drift when codegen is installed |
| `just check-rust` / `cargo xtask check-rust` | fmt, clippy (`--exclude minos-desktop`), lint-naming, lint-docs, `cargo test --workspace --exclude minos-desktop`, `cargo test -p minos-daemon --features test-support`, backend platform schema drift |
| `just check-macos` / `cargo xtask check-macos` | UniFFI, universal staticlib, XcodeGen, xcodebuild, MinosTests, swiftlint, `cargo check -p minos-desktop`, Flutter `--tags ffi` (macOS only) |
| `just check-backend` | schema drift + backend tests + daemon `test-support` tests |
| `just check-web` | `apps/web` `pnpm check` (eslint + production build) |
| `just check-desktop` | `apps/desktop` `pnpm check:all` (tsc + unit tests + biome + file-size + px-text) |

Stub lints (`lint-conventions`, `lint-metrics`) remain as standalone xtask
commands but are **not** part of any gate until implemented.

## CI matrix (`.github/workflows/ci.yml`)

| Job | Runner | Command / steps | Owns |
|-----|--------|-----------------|------|
| `rust` | ubuntu | sqlx prepare --check → `cargo xtask check-rust` | Rust quality, daemon integration tests, backend schema |
| `windows-host` | windows | host crate tests + daemon `test-support` + bin builds | Windows portability only |
| `dart` | ubuntu | format `lib`+`test`, analyze, test `--exclude-tags ffi`, frb drift | Mobile logic + **sole** frb drift owner |
| `frontend` | ubuntu | `apps/web` `pnpm check` + `apps/desktop` `pnpm check:all` | Web admin + desktop JS/TS |
| `macos` | macos-15 | `needs: [rust]` → bootstrap → `cargo xtask check-macos` | Apple native, `minos-desktop` Rust check, Flutter ffi |

### Explicit non-goals / split ownership

- **macOS does not re-run** workspace `cargo test`, dart format/analyze, or frb drift.
- **`minos-desktop` Rust** is excluded from workspace clippy/test on Linux (GUI deps). CI compiles it on macOS via `check-macos`. Desktop **frontend** gates run on ubuntu `frontend`.
- **Daemon `test-support` tests** live inside `check-rust` (and are re-run on Windows for OS coverage). They are not optional CI decoration.
- **`cargo deny`** is configured (`deny.toml`) but not yet wired; track separately (wire or delete).

## Version pins

Workflow-level `env` in `ci.yml`:

- `RUST_TOOLCHAIN` — must match `rust-toolchain.toml`
- `FLUTTER_VERSION` — must match `apps/mobile/.fvmrc`
- `FRB_CODEGEN_VERSION` — must match workspace `flutter_rust_bridge` / bootstrap
- `SQLX_CLI_VERSION` — sqlx-cli install pin
