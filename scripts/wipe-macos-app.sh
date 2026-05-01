#!/usr/bin/env bash
#
# Nuke all local state for the Minos macOS app + daemon CLI on this Mac.
# Backend rows (the relay's SQLite) are NOT touched — those live server-side.
#
# Safe to re-run. Each step tolerates "already gone".
#
#   ./scripts/wipe-macos-app.sh           # interactive confirm
#   ./scripts/wipe-macos-app.sh --yes     # skip confirm

set -u

YES=0
if [[ "${1:-}" == "--yes" || "${1:-}" == "-y" ]]; then
  YES=1
fi

bold()  { printf '\033[1m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
dim()   { printf '\033[2m%s\033[0m\n' "$*"; }

bold "Will delete the following:"
cat <<'EOF'
  app processes:
    Minos.app, minos-daemon, minosd

  user state:
    ~/Library/Application Support/Minos
    ~/Library/Caches/ai.minos.macos
    ~/Library/Logs/Minos                    (includes the 14 GB xlog)
    ~/Library/Preferences/ai.minos.macos.plist
    ~/Library/HTTPStorages/ai.minos.macos
    ~/Library/WebKit/ai.minos.macos
    ~/.minos                                (daemon CLI home)

  keychain (login + iCloud):
    service=ai.minos.macos  account=device-secret  (all matches)

  app bundles:
    /Applications/Minos.app
    ~/Library/Developer/Xcode/DerivedData/Minos-*

  NOT touched:
    backend DB rows (server-side, delete those manually if needed)
    /Users/zhangfan/develop/github.com/minos                 (source tree)
EOF
echo

if [[ $YES -ne 1 ]]; then
  read -r -p "Proceed? [y/N] " ans
  [[ "$ans" == "y" || "$ans" == "Y" ]] || { red "aborted"; exit 1; }
fi

# 1. processes
bold "[1/5] stopping processes"
osascript -e 'quit app "Minos"' >/dev/null 2>&1 || true
pkill -f 'Minos\.app/Contents/MacOS/Minos'  >/dev/null 2>&1 || true
pkill -x minos-daemon >/dev/null 2>&1 || true
pkill -x minosd       >/dev/null 2>&1 || true
sleep 1
green "  ok"

# 2. user state
bold "[2/5] removing user state"
paths=(
  "$HOME/Library/Application Support/Minos"
  "$HOME/Library/Caches/ai.minos.macos"
  "$HOME/Library/Logs/Minos"
  "$HOME/Library/Preferences/ai.minos.macos.plist"
  "$HOME/Library/HTTPStorages/ai.minos.macos"
  "$HOME/Library/WebKit/ai.minos.macos"
  "$HOME/.minos"
)
for p in "${paths[@]}"; do
  if [[ -e "$p" ]]; then
    rm -rf -- "$p" && dim "  removed $p"
  else
    dim "  skip    $p (absent)"
  fi
done
green "  ok"

# 3. keychain (loop until find returns no match — covers protected + legacy)
bold "[3/5] purging keychain entries (ai.minos.macos / device-secret)"
deleted=0
while security find-generic-password -s ai.minos.macos -a device-secret >/dev/null 2>&1; do
  if security delete-generic-password -s ai.minos.macos -a device-secret >/dev/null 2>&1; then
    deleted=$((deleted + 1))
  else
    red "  delete failed — entry may be in the protected keychain."
    red "  open Keychain Access.app, search 'ai.minos.macos', delete manually."
    break
  fi
done
if [[ $deleted -gt 0 ]]; then
  dim "  removed $deleted keychain entr$([[ $deleted -eq 1 ]] && echo y || echo ies)"
else
  dim "  no entries found"
fi
green "  ok"

# 4. app bundles
bold "[4/5] removing app bundles"
if [[ -e /Applications/Minos.app ]]; then
  rm -rf /Applications/Minos.app && dim "  removed /Applications/Minos.app"
else
  dim "  skip    /Applications/Minos.app (absent)"
fi
shopt -s nullglob
dd_dirs=( "$HOME"/Library/Developer/Xcode/DerivedData/Minos-* )
shopt -u nullglob
if [[ ${#dd_dirs[@]} -gt 0 ]]; then
  for d in "${dd_dirs[@]}"; do
    rm -rf -- "$d" && dim "  removed $d"
  done
else
  dim "  skip    DerivedData/Minos-* (absent)"
fi
green "  ok"

# 5. verify
bold "[5/5] verifying"
ok=1
check_absent() {
  if [[ -e "$1" ]]; then
    red "  STILL EXISTS: $1"
    ok=0
  fi
}
check_absent "$HOME/Library/Application Support/Minos"
check_absent "$HOME/Library/Logs/Minos"
check_absent "$HOME/.minos"
check_absent "/Applications/Minos.app"
if security find-generic-password -s ai.minos.macos -a device-secret >/dev/null 2>&1; then
  red "  STILL EXISTS: keychain entry ai.minos.macos/device-secret"
  red "                (likely in protected keychain — remove via Keychain Access.app)"
  ok=0
fi

echo
if [[ $ok -eq 1 ]]; then
  green "done — clean slate. next App launch will mint a fresh DeviceId and FirstConnect on the backend."
else
  red "done with leftovers — see lines above."
  exit 2
fi
