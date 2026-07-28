#!/usr/bin/env bash
# Assemble a multi-platform Tauri updater latest.json.
#
# Usage:
#   generate-latest-json.sh <version> <platform-key:sig-file:archive-url>...
#
# Example:
#   ./generate-latest-json.sh 0.2.0 \
#     darwin-aarch64:./Minos.app.tar.gz.sig:https://github.com/org/Minos/releases/download/v0.2.0/Minos_aarch64.app.tar.gz \
#     linux-x86_64:./Minos.AppImage.sig:https://github.com/org/Minos/releases/download/v0.2.0/Minos_amd64.AppImage
#
# Platform keys (Tauri 2):
#   darwin-aarch64 | darwin-x86_64 | linux-x86_64 | windows-x86_64
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "Usage: generate-latest-json.sh <version> <platform-key:sig-file:archive-url>..." >&2
  exit 1
fi

VERSION="$1"
shift

if ! command -v jq >/dev/null 2>&1; then
  echo "Error: jq is required" >&2
  exit 1
fi

platform_args=()
platforms_obj="{}"
i=0
for triple in "$@"; do
  key="${triple%%:*}"
  rest="${triple#*:}"
  sig_file="${rest%%:*}"
  url="${rest#*:}"
  if [[ "$key" == "$triple" || "$sig_file" == "$rest" || -z "$key" || -z "$sig_file" || -z "$url" ]]; then
    echo "Error: malformed triple '$triple' (expected platform-key:sig-file:archive-url)" >&2
    exit 1
  fi
  if [[ ! -f "$sig_file" ]]; then
    echo "Error: signature file not found: $sig_file" >&2
    exit 1
  fi

  sig_arg="sig$i"
  url_arg="url$i"
  platform_args+=(--arg "$sig_arg" "$(cat "$sig_file")" --arg "$url_arg" "$url")
  platforms_obj="$platforms_obj + { \"$key\": { signature: \$$sig_arg, url: \$$url_arg } }"
  i=$((i + 1))
done

jq -n \
  --arg version "$VERSION" \
  --arg notes "Minos v$VERSION" \
  --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "${platform_args[@]}" \
  "{ version: \$version, notes: \$notes, pub_date: \$pub_date, platforms: ($platforms_obj) }"
