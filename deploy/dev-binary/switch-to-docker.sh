#!/usr/bin/env bash
# VPS-side: stop systemd binary backend, start Docker minos-backend again.
# Postgres/Redis remain; Caddy unchanged. Mutual exclusion on :8787.
#
# Usage (on VPS as root or via sudo):
#   /opt/minos/bin/switch-to-docker.sh [--compose-dir DIR] [--no-health]
set -euo pipefail

COMPOSE_DIR="${MINOS_COMPOSE_DIR:-/opt/minos/deploy}"
COMPOSE_FILE="${COMPOSE_DIR}/docker-compose.yml"
HEALTH=1

usage() {
  cat <<'EOF'
Switch VPS from systemd binary back to Docker minos-backend.

Usage:
  switch-to-docker.sh [options]

Options:
  --compose-dir DIR   Directory with docker-compose.yml + .env (default: /opt/minos/deploy)
  --no-health         Skip health check after start
  -h, --help          Show this help

Requires:
  - /opt/minos/deploy/docker-compose.yml
  - /opt/minos/deploy/minos.env (SSOT; .env should symlink to it)
  - MINOS_BACKEND_IMAGE set in minos.env
  - docker compose available
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --compose-dir)
      COMPOSE_DIR="${2:?}"
      COMPOSE_FILE="${COMPOSE_DIR}/docker-compose.yml"
      shift 2
      ;;
    --no-health)
      HEALTH=0
      shift
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

if [[ "$(id -u)" -ne 0 ]]; then
  echo "error: run as root (or sudo $0)" >&2
  exit 1
fi

if [[ ! -f "$COMPOSE_FILE" ]]; then
  echo "error: missing $COMPOSE_FILE" >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "error: docker not found" >&2
  exit 1
fi

echo "==> stop systemd minos-backend"
if systemctl list-unit-files minos-backend.service >/dev/null 2>&1; then
  systemctl stop minos-backend 2>/dev/null || true
  systemctl disable minos-backend 2>/dev/null || true
else
  echo "    (unit not present; continuing)"
fi

# Confirm port free
if command -v ss >/dev/null 2>&1; then
  if ss -lntp 2>/dev/null | grep -q ':8787'; then
    echo "error: port 8787 still in use after stopping systemd unit:" >&2
    ss -lntp 2>/dev/null | grep ':8787' || true
    exit 1
  fi
fi

echo "==> start Docker stack (postgres + redis + minos-backend)"
cd "$COMPOSE_DIR"
docker compose pull minos-backend || true
docker compose up -d
docker compose ps

if [[ "$HEALTH" -eq 1 ]]; then
  health="/opt/minos/bin/healthcheck.sh"
  if [[ -x "$health" ]]; then
    bash "$health" --wait 90
  else
    for _ in $(seq 1 30); do
      if curl -fsS http://127.0.0.1:8787/health/live >/dev/null 2>&1 \
        && curl -fsS http://127.0.0.1:8787/health/ready >/dev/null 2>&1; then
        curl -fsS http://127.0.0.1:8787/health/live; echo
        curl -fsS http://127.0.0.1:8787/health/ready; echo
        echo "OK"
        exit 0
      fi
      sleep 2
    done
    echo "error: Docker backend health check failed" >&2
    docker compose logs --tail=80 minos-backend || true
    exit 1
  fi
fi

echo "==> Docker mode active (production SSOT path)"
echo "    docs: docs/ops/vps-deploy.md"
