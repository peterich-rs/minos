#!/usr/bin/env bash
# Notarize a CI-published macOS Desktop release on a local Mac, then re-upload.
#
# CI intentionally codesigns only (Apple notary regularly exceeds the 6h Actions
# job cap). After desktop-release.yml finishes, run this on a machine with
# Xcode CLT + notarytool credentials.
#
# Usage:
#   export APPLE_ID=you@example.com
#   export APPLE_APP_SPECIFIC_PASSWORD=xxxx-xxxx-xxxx-xxxx
#   export APPLE_TEAM_ID=XXXXXXXXXX
#   export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/minos.key)"
#   # optional: TAURI_SIGNING_PRIVATE_KEY_PASSWORD
#   # optional: GH_TOKEN / gh auth for download+upload
#
#   apps/desktop/scripts/notarize-macos-release.sh --version 0.2.0
#   apps/desktop/scripts/notarize-macos-release.sh --version 0.2.0 --skip-upload
#   apps/desktop/scripts/notarize-macos-release.sh --app ./Minos.app --dmg ./Minos.dmg
#
# What it does:
#   1. Download (or use) .app.tar.gz + DMG from GitHub release vX.Y.Z
#   2. notarytool submit + wait + staple
#   3. Re-pack .app → tar.gz, re-sign with Tauri updater minisign key
#   4. Re-upload assets + regenerate latest.json for minos-desktop-latest
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
DESKTOP="$ROOT/apps/desktop"
WORKDIR="${TMPDIR:-/tmp}/minos-notarize-$$"
REPO="${GITHUB_REPOSITORY:-}"
if [[ -z "$REPO" ]] && command -v gh >/dev/null 2>&1; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
fi
REPO="${REPO:-peterich-rs/minos}"

VERSION=""
APP_PATH=""
DMG_PATH=""
ARCHIVE_PATH=""
SKIP_UPLOAD=0
PROFILE_NAME="minos-notary"

usage() {
  sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="${2#v}"; shift 2 ;;
    --app) APP_PATH="$2"; shift 2 ;;
    --dmg) DMG_PATH="$2"; shift 2 ;;
    --archive) ARCHIVE_PATH="$2"; shift 2 ;;
    --skip-upload) SKIP_UPLOAD=1; shift ;;
    -h|--help) usage 0 ;;
    *) echo "unknown arg: $1" >&2; usage 1 ;;
  esac
done

need() {
  if [[ -z "${!1:-}" ]]; then
    echo "missing env: $1" >&2
    exit 1
  fi
}

need APPLE_ID
# Accept either name used by Tauri CI or the shorter APPLE_PASSWORD.
if [[ -z "${APPLE_APP_SPECIFIC_PASSWORD:-}" && -z "${APPLE_PASSWORD:-}" ]]; then
  echo "missing env: APPLE_APP_SPECIFIC_PASSWORD (or APPLE_PASSWORD)" >&2
  exit 1
fi
APPLE_PASSWORD_VALUE="${APPLE_APP_SPECIFIC_PASSWORD:-$APPLE_PASSWORD}"
need APPLE_TEAM_ID
need TAURI_SIGNING_PRIVATE_KEY

for bin in xcrun tar gh node pnpm; do
  command -v "$bin" >/dev/null 2>&1 || {
    echo "missing required tool: $bin" >&2
    exit 1
  }
done

cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT
mkdir -p "$WORKDIR"
cd "$WORKDIR"

TAG=""
if [[ -n "$VERSION" ]]; then
  TAG="v${VERSION}"
fi

download_from_release() {
  local tag="$1"
  echo "==> downloading assets from release $tag ($REPO)"
  gh release download "$tag" \
    --repo "$REPO" \
    --pattern "Minos_${VERSION}_aarch64.app.tar.gz" \
    --pattern "Minos_${VERSION}_aarch64.dmg" \
    --pattern "Minos_${VERSION}_aarch64.app.tar.gz.sig" \
    --clobber \
    --dir "$WORKDIR" || true

  # Fallback: original DMG name from Tauri (if stable name missing).
  if [[ ! -f "Minos_${VERSION}_aarch64.dmg" ]]; then
    gh release download "$tag" \
      --repo "$REPO" \
      --pattern "*.dmg" \
      --clobber \
      --dir "$WORKDIR" || true
  fi

  ARCHIVE_PATH="$WORKDIR/Minos_${VERSION}_aarch64.app.tar.gz"
  if [[ ! -f "$ARCHIVE_PATH" ]]; then
    echo "missing Minos_${VERSION}_aarch64.app.tar.gz on release $tag" >&2
    exit 1
  fi
  # Prefer stable DMG name; else first dmg in workdir.
  if [[ -f "Minos_${VERSION}_aarch64.dmg" ]]; then
    DMG_PATH="$WORKDIR/Minos_${VERSION}_aarch64.dmg"
  else
    DMG_PATH="$(find "$WORKDIR" -maxdepth 1 -name '*.dmg' -type f | head -1 || true)"
  fi
}

if [[ -n "$VERSION" && -z "$APP_PATH" && -z "$ARCHIVE_PATH" ]]; then
  download_from_release "$TAG"
fi

if [[ -z "$APP_PATH" ]]; then
  if [[ -z "${ARCHIVE_PATH:-}" || ! -f "$ARCHIVE_PATH" ]]; then
    echo "need --version (download) or --archive / --app" >&2
    exit 1
  fi
  echo "==> extracting $ARCHIVE_PATH"
  tar -xzf "$ARCHIVE_PATH" -C "$WORKDIR"
  APP_PATH="$(find "$WORKDIR" -maxdepth 2 -name '*.app' -type d | head -1)"
fi

if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "no .app found" >&2
  exit 1
fi

APP_NAME="$(basename "$APP_PATH")"
echo "==> app: $APP_PATH"

# Store credentials in a temporary keychain profile for notarytool (no password on argv after).
echo "==> configuring notarytool profile ($PROFILE_NAME)"
xcrun notarytool store-credentials "$PROFILE_NAME" \
  --apple-id "$APPLE_ID" \
  --team-id "$APPLE_TEAM_ID" \
  --password "$APPLE_PASSWORD_VALUE" \
  >/dev/null

submit_and_staple() {
  local path="$1"
  local label="$2"
  if [[ ! -e "$path" ]]; then
    echo "skip $label (missing: $path)"
    return 0
  fi
  echo "==> notarytool submit $label (this can take many minutes)…"
  # Zip .app for submit; DMG submits as-is.
  local submit_path="$path"
  if [[ "$path" == *.app ]]; then
    submit_path="${path}.zip"
    ditto -c -k --keepParent "$path" "$submit_path"
  fi
  xcrun notarytool submit "$submit_path" \
    --keychain-profile "$PROFILE_NAME" \
    --wait
  echo "==> staple $label"
  xcrun stapler staple "$path"
  xcrun stapler validate "$path" || true
}

submit_and_staple "$APP_PATH" ".app"
if [[ -n "${DMG_PATH:-}" && -f "$DMG_PATH" ]]; then
  submit_and_staple "$DMG_PATH" "dmg"
else
  echo "==> no DMG to notarize (optional)"
fi

# Re-pack updater archive from stapled .app
OUT_DIR="$WORKDIR/out"
mkdir -p "$OUT_DIR"
STABLE_ARCHIVE="Minos_${VERSION:-local}_aarch64.app.tar.gz"
if [[ -z "$VERSION" ]]; then
  STABLE_ARCHIVE="${APP_NAME}.tar.gz"
fi
echo "==> packing $STABLE_ARCHIVE"
(
  cd "$(dirname "$APP_PATH")"
  tar -czf "$OUT_DIR/$STABLE_ARCHIVE" "$(basename "$APP_PATH")"
)

echo "==> signing updater archive (minisign / tauri signer)"
export TAURI_SIGNING_PRIVATE_KEY
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
(
  cd "$DESKTOP"
  pnpm tauri signer sign "$OUT_DIR/$STABLE_ARCHIVE"
)
test -f "$OUT_DIR/${STABLE_ARCHIVE}.sig" || {
  echo "missing ${STABLE_ARCHIVE}.sig after signer" >&2
  exit 1
}

if [[ -n "${DMG_PATH:-}" && -f "$DMG_PATH" ]]; then
  STABLE_DMG="Minos_${VERSION:-local}_aarch64.dmg"
  cp "$DMG_PATH" "$OUT_DIR/$STABLE_DMG"
fi

echo "==> artifacts in $OUT_DIR"
ls -la "$OUT_DIR"

if [[ "$SKIP_UPLOAD" -eq 1 ]]; then
  echo "skip upload (--skip-upload). Copy from $OUT_DIR"
  # Keep workdir for inspection
  trap - EXIT
  echo "WORKDIR=$WORKDIR"
  exit 0
fi

if [[ -z "$TAG" ]]; then
  echo "no --version: cannot upload to GitHub release" >&2
  exit 1
fi

echo "==> uploading notarized assets to $TAG + minos-desktop-latest"
gh release upload "$TAG" \
  "$OUT_DIR/$STABLE_ARCHIVE" \
  "$OUT_DIR/${STABLE_ARCHIVE}.sig" \
  --repo "$REPO" \
  --clobber
gh release upload minos-desktop-latest \
  "$OUT_DIR/$STABLE_ARCHIVE" \
  "$OUT_DIR/${STABLE_ARCHIVE}.sig" \
  --repo "$REPO" \
  --clobber

if [[ -f "$OUT_DIR/Minos_${VERSION}_aarch64.dmg" ]]; then
  gh release upload "$TAG" "$OUT_DIR/Minos_${VERSION}_aarch64.dmg" --repo "$REPO" --clobber
fi

# Refresh latest.json signature for darwin-aarch64 (sig content changed).
echo "==> regenerating latest.json"
META="$WORKDIR/meta"
mkdir -p "$META"
cp "$OUT_DIR/${STABLE_ARCHIVE}.sig" "$META/darwin-aarch64.sig"
echo "https://github.com/${REPO}/releases/download/minos-desktop-latest/${STABLE_ARCHIVE}" \
  >"$META/darwin-aarch64.url"

# Pull other platforms' meta from current latest.json if present.
if gh release download minos-desktop-latest \
  --repo "$REPO" \
  --pattern "latest.json" \
  --clobber \
  --dir "$WORKDIR" 2>/dev/null; then
  # Preserve non-darwin entries from previous latest.json via jq if available.
  if command -v jq >/dev/null 2>&1 && [[ -f "$WORKDIR/latest.json" ]]; then
    # Write a combined latest via generate script if linux meta still on release.
    :
  fi
fi

# Prefer regenerate with available platform triples we can fetch from release.
TRIPLES=()
TRIPLES+=("darwin-aarch64:${META}/darwin-aarch64.sig:https://github.com/${REPO}/releases/download/minos-desktop-latest/${STABLE_ARCHIVE}")

# Try keep linux platform from existing latest.json
if command -v jq >/dev/null 2>&1 && [[ -f "$WORKDIR/latest.json" ]]; then
  for key in linux-x86_64 darwin-x86_64 windows-x86_64; do
    url=$(jq -r --arg k "$key" '.platforms[$k].url // empty' "$WORKDIR/latest.json" 2>/dev/null || true)
    sig=$(jq -r --arg k "$key" '.platforms[$k].signature // empty' "$WORKDIR/latest.json" 2>/dev/null || true)
    if [[ -n "$url" && -n "$sig" && "$key" != "darwin-aarch64" ]]; then
      echo "$sig" >"$META/${key}.sig"
      TRIPLES+=("${key}:${META}/${key}.sig:${url}")
    fi
  done
fi

chmod +x "$DESKTOP/scripts/generate-latest-json.sh"
"$DESKTOP/scripts/generate-latest-json.sh" "$VERSION" "${TRIPLES[@]}" >"$OUT_DIR/latest.json"
cat "$OUT_DIR/latest.json"
gh release upload minos-desktop-latest "$OUT_DIR/latest.json" --repo "$REPO" --clobber
gh release upload "$TAG" "$OUT_DIR/latest.json" --repo "$REPO" --clobber

echo "==> done. Notarized + stapled assets published for $TAG"
echo "    archive: $STABLE_ARCHIVE"
echo "    note: first-party downloads should use the re-uploaded DMG / archive"
