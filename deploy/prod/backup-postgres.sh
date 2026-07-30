#!/usr/bin/env bash
# Nightly / manual Postgres dump for the production compose stack.
# Install on VPS as /opt/minos/backups/backup-postgres.sh and chmod +x.
set -euo pipefail

STAMP=$(date -u +%Y%m%dT%H%M%SZ)
OUT="/opt/minos/backups/minos-pg-${STAMP}.sql.gz"
cd /opt/minos/deploy

# shellcheck disable=SC1091
set -a
source .env
set +a

docker compose exec -T postgres \
  pg_dump -U "${POSTGRES_USER}" "${POSTGRES_DB}" | gzip > "${OUT}"

find /opt/minos/backups -name 'minos-pg-*.sql.gz' -mtime +14 -delete
echo "wrote ${OUT}"
