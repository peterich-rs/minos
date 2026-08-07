#!/usr/bin/env bash
# VPS-side: stop Docker minos-backend, keep Postgres/Redis (and Caddy), start
# systemd binary unit on 127.0.0.1:8787.
#
# Safe to re-run (idempotent where reasonable).
#
# Usage (on VPS as root or via sudo):
#   /opt/minos/bin/switch-to-binary.sh [--compose-dir DIR] [--no-health]
set -euo pipefail

COMPOSE_DIR="${MINOS_COMPOSE_DIR:-/opt/minos/deploy}"
REMOTE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." 2>/dev/null && pwd || echo /opt/minos)"
# Prefer parent of bin/ when installed under /opt/minos/bin
if [[ -d "${REMOTE_ROOT}/releases" ]]; then
  :
elif [[ -d /opt/minos/releases ]]; then
  REMOTE_ROOT=/opt/minos
fi
COMPOSE_FILE="${COMPOSE_DIR}/docker-compose.yml"
ENV_FILE="${REMOTE_ROOT}/deploy/minos.env"
HEALTH=1

usage() {
  cat <<'EOF'
Switch VPS from Docker minos-backend to systemd binary (Postgres/Redis/Caddy stay).

Usage:
  switch-to-binary.sh [options]

Options:
  --compose-dir DIR   Directory with docker-compose.yml (default: /opt/minos/deploy)
  --no-health         Skip health check after start
  -h, --help          Show this help

Requires:
  - /opt/minos/current/minos-backend (symlink from deploy-backend.sh)
  - /opt/minos/deploy/minos.env (see deploy/prod/minos.env.example)
  - minos-backend.service installed
  - Postgres + Redis reachable on 127.0.0.1 (compose or otherwise)
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

if [[ ! -x "${REMOTE_ROOT}/current/minos-backend" ]]; then
  echo "error: missing executable ${REMOTE_ROOT}/current/minos-backend — run deploy-backend.sh first" >&2
  exit 1
fi

if [[ ! -f "$ENV_FILE" ]]; then
  echo "error: missing $ENV_FILE" >&2
  echo "  From laptop: scp deploy/prod/minos.env.example user@vps:/tmp/minos.env" >&2
  echo "  Then: sudo install -m 640 -o root -g minos /tmp/minos.env $ENV_FILE" >&2
  echo "  And:  sudo ln -sfn $ENV_FILE ${REMOTE_ROOT}/deploy/.env" >&2
  echo "  Use 127.0.0.1 for Postgres/Redis (not docker service hostnames)." >&2
  exit 1
fi

if [[ ! -f /etc/systemd/system/minos-backend.service ]]; then
  echo "error: systemd unit not installed; re-run deploy-backend.sh without --no-unit" >&2
  exit 1
fi

echo "==> stop Docker minos-backend (leave postgres + redis)"
if [[ -f "$COMPOSE_FILE" ]] && command -v docker >/dev/null 2>&1; then
  # scale/stop only the backend service; ignore if already stopped
  (cd "$COMPOSE_DIR" && docker compose stop minos-backend) || true
  (cd "$COMPOSE_DIR" && docker compose rm -f minos-backend) || true
  # Ensure data plane still up
  (cd "$COMPOSE_DIR" && docker compose up -d postgres redis) || true
else
  echo "    (no compose file or docker; assuming Postgres/Redis already managed)"
fi

# Free port if something else still holds 8787
if command -v ss >/dev/null 2>&1; then
  if ss -lntp 2>/dev/null | grep -q ':8787'; then
    echo "==> port 8787 still in use before starting binary unit:"
    ss -lntp 2>/dev/null | grep ':8787' || true
  fi
fi

echo "==> enable + start systemd minos-backend"
systemctl daemon-reload
systemctl enable minos-backend
systemctl restart minos-backend
systemctl --no-pager --full status minos-backend || true

if [[ "$HEALTH" -eq 1 ]]; then
  health="${REMOTE_ROOT}/bin/healthcheck.sh"
  if [[ -x "$health" ]]; then
    bash "$health" --wait 60
  else
    curl -fsS http://127.0.0.1:8787/health/live
    echo
    curl -fsS http://127.0.0.1:8787/health/ready
    echo
  fi
fi

echo "==> binary mode active"
echo "    process: systemctl status minos-backend"
echo "    rollback: ${REMOTE_ROOT}/bin/switch-to-docker.sh"
