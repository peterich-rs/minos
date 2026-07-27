# Desktop auto-update

Minos Desktop uses the same **Tauri 2 updater** shape as Buzz: signed release
artifacts, a rolling `latest.json` endpoint, in-app check/download/install, and
a hard stop of managed daemon/agent processes before binary swap.

## Architecture

```
CI release
  ├── vX.Y.Z                 user-facing installers (DMG / AppImage / …)
  └── minos-desktop-latest   rolling updater release
        └── latest.json  + per-platform archive + .sig
                 ▲
                 │ check()
        Desktop  tauri-plugin-updater  (release builds only)
                 │ prepare_for_app_update → stop daemon/agents
                 │ install + relaunch
        UI: Host → Updates  ·  sidebar nudge card
```

## Phase A (shipped in app)

| Piece | Location |
|-------|----------|
| Build-time gate | `apps/desktop/src-tauri/build.rs` → `cfg(minos_updater_enabled)` when `MINOS_UPDATER_PUBLIC_KEY` **and** `MINOS_UPDATER_ENDPOINT` are set |
| Release conf delta | `apps/desktop/scripts/build-release-config.mjs` → `tauri.release.conf.json` |
| Plugin register | `apps/desktop/src-tauri/src/lib.rs` (updater only if cfg + non-debug) |
| Platform support | `is_auto_update_supported` — Linux requires AppImage (`APPIMAGE` env) |
| Pre-install teardown | `prepare_for_app_update` stops managed daemon (Phase C) |
| Frontend state machine | `features/settings/hooks/use-updater.ts` |
| UI | Host → Updates (`UpdateChecker`), sidebar (`SidebarUpdateCard`) |
| Capabilities | `updater:allow-*`, `process:allow-restart` |

Local `pnpm tauri:dev` / debug builds **never** enable the updater plugin, so
dev binaries never hit a production endpoint.

## Phase B — release pipeline (operator checklist)

### Secrets (GitHub Actions)

| Secret | Purpose |
|--------|---------|
| `MINOS_UPDATER_PUBLIC_KEY` | minisign public key embedded in the app (also used by conf inject) |
| `TAURI_SIGNING_PRIVATE_KEY` | minisign private key for signing updater archives |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | password for the private key (if any) |
| `MINOS_BACKEND_URL` | required for release `minos-daemon` builds (`crates/minos-daemon/build.rs`) |
| Apple signing / notarize secrets | codesign + notarize DMG / .app (required for macOS Gatekeeper trust) |

Generate a keypair once (see [Tauri updater](https://v2.tauri.app/plugin/updater/)):

```sh
npm run tauri signer generate -w ~/.tauri/minos.key
# public key → MINOS_UPDATER_PUBLIC_KEY
# private key file → TAURI_SIGNING_PRIVATE_KEY
```

### Rolling endpoint

Recommended:

```text
https://github.com/peterich-rs/minos/releases/download/minos-desktop-latest/latest.json
```

Configured as repo variable `MINOS_UPDATER_ENDPOINT`. Updater minisign secrets
are stored as Actions secrets (`MINOS_UPDATER_PUBLIC_KEY`,
`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).

Publish two GitHub releases per cut (both created with `--latest=false` so they
do not fight for the GitHub “latest” pointer):

1. **`vX.Y.Z`** — human installers + notes (optional git tag `vX.Y.Z`)
2. **`minos-desktop-latest`** — **named** rolling release only (no movable git
   tag / force-push). Assets + `latest.json` are clobbered each cut.

CI signs updater archives and runs `node scripts/verify-updater-sig.mjs`
(same minisign algorithm as `tauri-plugin-updater`) against
`MINOS_UPDATER_PUBLIC_KEY` so key rotation mismatches fail the job instead of
client install.

`prepare_for_app_update` hard-timeouts managed daemon stop (same budget as app
exit) and returns `Err` on failure; install is blocked until prepare succeeds.

If prepare/install/relaunch fails **after** teardown may have run, the UI calls
`restore_after_failed_update`: clears the prepare guard and
`DaemonBridge::connect` starts a fresh managed daemon so the shell is not left
without local RPC. The original install error is still shown; restore failure
is appended to that message.

### Build steps (outline)

```sh
cd apps/desktop
export MINOS_UPDATER_PUBLIC_KEY=...
export MINOS_UPDATER_ENDPOINT=https://github.com/<owner>/Minos/releases/download/minos-desktop-latest/latest.json
export TAURI_SIGNING_PRIVATE_KEY=...
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=...

node scripts/build-release-config.mjs
# same MINOS_UPDATER_* in the environment so build.rs enables the plugin
pnpm tauri build --config src-tauri/tauri.release.conf.json
```

Assemble multi-platform `latest.json`:

```sh
./scripts/generate-latest-json.sh 0.2.0 \
  darwin-aarch64:./path/to.app.tar.gz.sig:https://.../Minos_aarch64.app.tar.gz \
  linux-x86_64:./path/to.AppImage.sig:https://.../Minos.AppImage \
  > latest.json
gh release upload minos-desktop-latest latest.json --clobber
```

Platform keys: `darwin-aarch64`, `darwin-x86_64`, `linux-x86_64`, `windows-x86_64`.

### Linux note

Only **AppImage** supports in-app update. `.deb` installs see
`manual-required` and open the GitHub releases page.

### macOS note

Codesign + notarize the `.app` / DMG in CI. After notarization, rebuild the
updater `.tar.gz` from the signed app and re-sign with the Tauri updater key
(same pattern as Buzz `release.yml`).

Workflow skeleton: `.github/workflows/desktop-release.yml` (manual
`workflow_dispatch` until secrets and signing are configured).

## Phase C — process model

Before `update.install()` + `relaunch()`:

1. Frontend calls `prepare_for_app_update`
2. Host stops the managed in-process daemon (and thus agent children)
3. Install swaps the Desktop bundle
4. Relaunch starts a clean process tree

On failure after step 1–2 (install signature/disk error, relaunch error, or
prepare timeout after teardown began):

5. Frontend calls `restore_after_failed_update` → reset guard + restart managed daemon
6. Workspace `connection` is updated so the UI is not stuck “daemon down”

Exit / signal teardown shares the same stop path and is idempotent with
prepare (no double-stop races).

## Manual verification (dev)

1. `pnpm check` / `pnpm test` — UI unit tests for visibility helper  
2. `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` — compiles
   with plugins; updater **not** registered in debug  
3. Host → Updates shows **unavailable** on local builds (expected)  
4. Release smoke: build with both env vars, confirm `is_updater_plugin_enabled`
   is true in a release binary, and that check hits `latest.json`
