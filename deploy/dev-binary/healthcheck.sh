#!/usr/bin/env bash
# Probe minos-backend liveness/readiness on loopback (behind Caddy).
#
# Usage:
#   ./healthcheck.sh [--base URL] [--wait SECONDS] [--public URL]
#   ./healthcheck.sh --help
set -euo pipefail

BASE="${MINOS_HEALTH_BASE:-http://127.0.0.1:8787}"
PUBLIC="${MINOS_HEALTH_PUBLIC:-}"
WAIT_SECS=0
INTERVAL=2

usage() {
  cat <<'EOF'
Health-check minos-backend (/health/live and /health/ready).

Usage:
  healthcheck.sh [options]

Options:
  --base URL       Loopback origin (default: http://127.0.0.1:8787)
  --public URL     Optional public origin, e.g. https://minos.ainexc.com
  --wait SECONDS   Retry until ready or timeout (default: 0 = single attempt)
  --interval SECS  Sleep between retries when --wait > 0 (default: 2)
  -h, --help       Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      BASE="${2:?}"
      shift 2
      ;;
    --public)
      PUBLIC="${2:?}"
      shift 2
      ;;
    --wait)
      WAIT_SECS="${2:?}"
      shift 2
      ;;
    --interval)
      INTERVAL="${2:?}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: required command not found: $1" >&2
    exit 1
  }
}
need_cmd curl

check_once() {
  local origin="$1"
  local label="$2"
  echo "-- $label live:  ${origin}/health/live"
  curl -fsS --max-time 5 "${origin}/health/live"
  echo
  echo "-- $label ready: ${origin}/health/ready"
  curl -fsS --max-time 5 "${origin}/health/ready"
  echo
}

deadline=$((SECONDS + WAIT_SECS))
attempt=0
while true; do
  attempt=$((attempt + 1))
  if check_once "$BASE" "loopback"; then
    if [[ -n "$PUBLIC" ]]; then
      check_once "$PUBLIC" "public" || {
        echo "error: public health check failed (loopback ok; check Caddy/DNS)" >&2
        exit 1
      }
    fi
    echo "OK (attempt $attempt)"
    exit 0
  fi
  if [[ "$WAIT_SECS" -le 0 || "$SECONDS" -ge "$deadline" ]]; then
    echo "error: health check failed after ${attempt} attempt(s)" >&2
    exit 1
  fi
  echo "retry in ${INTERVAL}s (wait budget remaining ~$((deadline - SECONDS))s)..."
  sleep "$INTERVAL"
done
