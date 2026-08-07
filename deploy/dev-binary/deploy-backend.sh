#!/usr/bin/env bash
# Ship a prebuilt linux minos-backend binary to the VPS and (optionally) restart
# the systemd unit behind existing Caddy. Does not clone the monorepo on the VPS.
#
# Layout on VPS:
#   /opt/minos/releases/<git-sha>/minos-backend
#   /opt/minos/current -> releases/<git-sha>
#   /opt/minos/deploy/minos.env   (EnvironmentFile for systemd)
#   /etc/systemd/system/minos-backend.service
#
# Usage:
#   ./deploy/dev-binary/deploy-backend.sh --host user@vps [options]
#   ./deploy/dev-binary/deploy-backend.sh --help
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOST="${MINOS_DEPLOY_HOST:-}"
SSH_OPTS="${MINOS_DEPLOY_SSH_OPTS:-}"
REMOTE_ROOT="${MINOS_DEPLOY_REMOTE_ROOT:-/opt/minos}"
BINARY_LOCAL="${MINOS_DEPLOY_BINARY:-$ROOT/dist/minos-backend-linux-amd64}"
BUILD=1
BUILD_METHOD="${MINOS_BUILD_METHOD:-docker}"
INSTALL_UNIT=1
RESTART=1
HEALTH=1
DRY_RUN=0

usage() {
  cat <<'EOF'
Deploy minos-backend binary to a VPS (coexists with Docker Postgres/Redis/Caddy).

Usage:
  deploy-backend.sh --host user@vps [options]

Options:
  --host HOST           SSH target (required), e.g. root@203.0.113.10 or minos@vps
  --binary PATH         Local linux binary (default: dist/minos-backend-linux-amd64)
  --remote-root PATH    VPS install root (default: /opt/minos)
  --skip-build          Do not rebuild; require --binary (or default path) to exist
  --build-method M      Passed to build-linux-backend.sh (default: docker)
  --no-unit             Do not install/update systemd unit template
  --no-restart          Upload + symlink only; do not systemctl restart
  --no-health           Skip post-restart health curls
  --dry-run             Print planned actions; no SSH/rsync/build side effects beyond planning
  -h, --help            Show this help

Environment:
  MINOS_DEPLOY_HOST, MINOS_DEPLOY_BINARY, MINOS_DEPLOY_REMOTE_ROOT,
  MINOS_DEPLOY_SSH_OPTS, MINOS_BUILD_METHOD

Mutual exclusion:
  Do NOT run Docker service minos-backend and systemd minos-backend on :8787
  at the same time. See docs/ops/vps-dev-binary.md and switch-to-binary.sh.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host)
      HOST="${2:?--host requires user@host}"
      shift 2
      ;;
    --binary)
      BINARY_LOCAL="${2:?--binary requires a path}"
      shift 2
      ;;
    --remote-root)
      REMOTE_ROOT="${2:?--remote-root requires a path}"
      shift 2
      ;;
    --skip-build)
      BUILD=0
      shift
      ;;
    --build-method)
      BUILD_METHOD="${2:?--build-method requires a value}"
      shift 2
      ;;
    --no-unit)
      INSTALL_UNIT=0
      shift
      ;;
    --no-restart)
      RESTART=0
      shift
      ;;
    --no-health)
      HEALTH=0
      shift
      ;;
    --dry-run)
      DRY_RUN=1
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

if [[ -z "$HOST" ]]; then
  echo "error: --host is required" >&2
  usage >&2
  exit 2
fi

sha="$(git -C "$ROOT" rev-parse --short=12 HEAD 2>/dev/null || echo "unknown")"
remote_release="${REMOTE_ROOT}/releases/${sha}"
remote_current="${REMOTE_ROOT}/current"
remote_bin="${remote_release}/minos-backend"
unit_src="$ROOT/deploy/dev-binary/minos-backend.service"
health_src="$ROOT/deploy/dev-binary/healthcheck.sh"
switch_bin_src="$ROOT/deploy/dev-binary/switch-to-binary.sh"
switch_docker_src="$ROOT/deploy/dev-binary/switch-to-docker.sh"

ssh_base=(ssh)
# shellcheck disable=SC2206
if [[ -n "$SSH_OPTS" ]]; then
  # intentional word-split for extra ssh flags from env
  ssh_base+=( $SSH_OPTS )
fi
ssh_base+=( "$HOST" )

rsync_ssh="ssh"
if [[ -n "$SSH_OPTS" ]]; then
  rsync_ssh="ssh $SSH_OPTS"
fi

run() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '[dry-run]'
    printf ' %q' "$@"
    printf '\n'
    return 0
  fi
  "$@"
}

echo "==> deploy minos-backend sha=$sha host=$HOST remote_root=$REMOTE_ROOT"

if [[ "$BUILD" -eq 1 ]]; then
  echo "==> build linux binary (method=$BUILD_METHOD)"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[dry-run] $ROOT/deploy/dev-binary/build-linux-backend.sh --method $BUILD_METHOD --out $BINARY_LOCAL"
  else
    "$ROOT/deploy/dev-binary/build-linux-backend.sh" --method "$BUILD_METHOD" --out "$BINARY_LOCAL"
  fi
fi

if [[ "$DRY_RUN" -eq 0 && ! -f "$BINARY_LOCAL" ]]; then
  echo "error: binary not found at $BINARY_LOCAL (build failed or use --skip-build after a successful build)" >&2
  exit 1
fi

if [[ "$DRY_RUN" -eq 0 ]]; then
  if command -v file >/dev/null 2>&1; then
    if file "$BINARY_LOCAL" | grep -qi 'Mach-O\|Darwin'; then
      echo "error: $BINARY_LOCAL looks like a macOS binary; build linux/amd64 first" >&2
      exit 1
    fi
  fi
fi

echo "==> remote mkdir $remote_release"
run "${ssh_base[@]}" "sudo mkdir -p $(printf %q "$remote_release") $(printf %q "${REMOTE_ROOT}/deploy") $(printf %q "${REMOTE_ROOT}/bin") && sudo chown -R \$(id -un):\$(id -gn) $(printf %q "${REMOTE_ROOT}/releases") $(printf %q "${REMOTE_ROOT}/bin") 2>/dev/null || true"

echo "==> rsync binary → ${HOST}:${remote_bin}"
if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "[dry-run] rsync -avz -e \"$rsync_ssh\" $BINARY_LOCAL ${HOST}:${remote_bin}.tmp"
else
  # Upload to a temp path then atomic move under sudo if needed.
  rsync -avz -e "$rsync_ssh" "$BINARY_LOCAL" "${HOST}:${remote_bin}.tmp"
  "${ssh_base[@]}" "install -m 755 $(printf %q "${remote_bin}.tmp") $(printf %q "$remote_bin") && rm -f $(printf %q "${remote_bin}.tmp")"
fi

echo "==> symlink $remote_current → releases/$sha"
run "${ssh_base[@]}" "ln -sfn $(printf %q "$remote_release") $(printf %q "$remote_current")"

# Ship helper scripts to a stable path on the VPS (no monorepo).
echo "==> install helper scripts under ${REMOTE_ROOT}/bin"
if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "[dry-run] rsync helpers → ${HOST}:${REMOTE_ROOT}/bin/"
else
  merge_src="$ROOT/deploy/dev-binary/merge-minos-env.sh"
  rsync -avz -e "$rsync_ssh" \
    "$health_src" "$switch_bin_src" "$switch_docker_src" "$merge_src" \
    "${HOST}:${REMOTE_ROOT}/bin/"
  "${ssh_base[@]}" "chmod +x $(printf %q "${REMOTE_ROOT}/bin")/*.sh"
fi

if [[ "$INSTALL_UNIT" -eq 1 ]]; then
  echo "==> install systemd unit (template paths: ${REMOTE_ROOT})"
  # Rewrite WorkingDirectory / ExecStart / EnvironmentFile for non-default roots.
  unit_tmp="$(mktemp)"
  sed \
    -e "s|/opt/minos/current/minos-backend|${REMOTE_ROOT}/current/minos-backend|g" \
    -e "s|/opt/minos/deploy/minos.env|${REMOTE_ROOT}/deploy/minos.env|g" \
    -e "s|WorkingDirectory=/opt/minos|WorkingDirectory=${REMOTE_ROOT}|g" \
    "$unit_src" >"$unit_tmp"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[dry-run] install unit from $unit_tmp → /etc/systemd/system/minos-backend.service"
    rm -f "$unit_tmp"
  else
    rsync -avz -e "$rsync_ssh" "$unit_tmp" "${HOST}:/tmp/minos-backend.service"
    rm -f "$unit_tmp"
    "${ssh_base[@]}" "sudo install -m 644 /tmp/minos-backend.service /etc/systemd/system/minos-backend.service && rm -f /tmp/minos-backend.service && sudo systemctl daemon-reload"
  fi
fi

if [[ "$RESTART" -eq 1 ]]; then
  echo "==> restart systemd minos-backend (ensure Docker backend is stopped first)"
  remote_restart="$(cat <<EOF
set -euo pipefail
COMPOSE="${REMOTE_ROOT}/deploy/docker-compose.yml"
ENVF="${REMOTE_ROOT}/deploy/minos.env"
if command -v docker >/dev/null 2>&1 && [[ -f "\$COMPOSE" ]]; then
  if sudo docker compose -f "\$COMPOSE" ps minos-backend 2>/dev/null | grep -Eqi 'Up|running'; then
    echo "WARN: Docker minos-backend appears running — stop it before binary mode" >&2
    echo "      sudo ${REMOTE_ROOT}/bin/switch-to-binary.sh" >&2
  fi
fi
if [[ ! -f "\$ENVF" ]]; then
  echo "WARN: missing \$ENVF — unit installed but not started" >&2
  echo "     Copy deploy/prod/minos.env.example there (use 127.0.0.1 for DB/Redis)." >&2
  exit 0
fi
sudo systemctl enable minos-backend
sudo systemctl restart minos-backend
sudo systemctl --no-pager --full status minos-backend || true
EOF
)"
  run "${ssh_base[@]}" "bash -s" <<<"$remote_restart"
fi

if [[ "$HEALTH" -eq 1 && "$RESTART" -eq 1 ]]; then
  echo "==> health check"
  run "${ssh_base[@]}" "bash $(printf %q "${REMOTE_ROOT}/bin/healthcheck.sh") --wait 60"
fi

echo "==> done sha=$sha"
echo "    binary:  $remote_bin"
echo "    current: $remote_current -> releases/$sha"
echo "    env:     ${REMOTE_ROOT}/deploy/minos.env"
echo "    unit:    minos-backend.service"
echo
echo "First-time switch from Docker backend:"
echo "  ssh $HOST 'sudo ${REMOTE_ROOT}/bin/switch-to-binary.sh'"
echo "Rollback to Docker backend:"
echo "  ssh $HOST 'sudo ${REMOTE_ROOT}/bin/switch-to-docker.sh'"
