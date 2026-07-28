#!/usr/bin/env bash
# Local smoke checks for the desktop auto-updater wiring (no GUI install).
#
# What this verifies:
#  1. Public key secret is present on the remote (via gh).
#  2. Release conf inject works with the local keypair.
#  3. Optional: if minos-desktop-latest/latest.json exists, validate JSON shape.
#
# Full check → download → prepare → install → relaunch requires a signed
# release build + running Desktop; that path is exercised by desktop-release.yml
# and manual Host → Updates after a successful publish.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"

echo "==> GitHub secrets"
gh secret list | grep -E 'MINOS_UPDATER_PUBLIC_KEY|TAURI_SIGNING_PRIVATE_KEY' || {
  echo "missing updater secrets" >&2
  exit 1
}

echo "==> Repo variables"
gh variable list | grep MINOS_UPDATER_ENDPOINT || true

KEY_PUB="${HOME}/.tauri/minos.key.pub"
KEY_PRIV="${HOME}/.tauri/minos.key"
if [[ ! -f "$KEY_PUB" || ! -f "$KEY_PRIV" ]]; then
  echo "local keypair missing at ~/.tauri/minos.key(.pub)" >&2
  exit 1
fi

echo "==> build-release-config.mjs"
cd apps/desktop
export MINOS_UPDATER_PUBLIC_KEY
MINOS_UPDATER_PUBLIC_KEY="$(cat "$KEY_PUB")"
export MINOS_UPDATER_ENDPOINT="${MINOS_UPDATER_ENDPOINT:-https://github.com/peterich-rs/minos/releases/download/minos-desktop-latest/latest.json}"
node scripts/build-release-config.mjs
test -f src-tauri/tauri.release.conf.json
# Ensure pubkey landed in conf
grep -q 'pubkey' src-tauri/tauri.release.conf.json
echo "wrote src-tauri/tauri.release.conf.json"

echo "==> latest.json (if published)"
ENDPOINT="$MINOS_UPDATER_ENDPOINT"
if curl -fsSL "$ENDPOINT" -o /tmp/minos-latest.json 2>/dev/null; then
  if command -v jq >/dev/null; then
    jq -e '.version and .platforms' /tmp/minos-latest.json >/dev/null
    echo "latest.json OK: version=$(jq -r .version /tmp/minos-latest.json)"
    jq -r '.platforms | keys[]' /tmp/minos-latest.json | sed 's/^/  platform: /'
  else
    echo "latest.json downloaded (jq not installed — skip schema check)"
  fi
else
  echo "latest.json not published yet (expected until first desktop-release run)"
fi

echo "==> prepare_for_app_update is registered in Rust sources"
grep -q 'prepare_for_app_update' src-tauri/src/commands/updater.rs
grep -q 'prepare_for_app_update' src-tauri/src/lib.rs

echo "OK: local updater wiring checks passed"
