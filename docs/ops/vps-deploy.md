# VPS production deploy (`minos-backend`)

Primary production path for Minos cloud hub.

| Item | Value |
|------|--------|
| Public origin | `https://minos.ainexc.com` |
| WebSocket | `wss://minos.ainexc.com` (`/ws/client`, `/ws/host`) |
| Host layout | `/opt/minos` runtime-only (images + data + Caddy) |
| Image registry | `ghcr.io/<owner>/minos-backend` |
| Repo manifests | `deploy/prod/` |

**Do not clone the monorepo onto the VPS.** Disk and RAM are limited (~60G SSD / 4G RAM). Build images in GitHub Actions; the VPS only `docker pull`s.

Optional alternate ingress (tunnel instead of public 443): [cloudflare-tunnel-setup.md](./cloudflare-tunnel-setup.md).

Optional chat attachments / media blobs on Cloudflare R2 (keeps large files off the VPS disk): [r2-media.md](./r2-media.md).

### Dev/agent binary bypass

For urgent agent/local deploys that **only replace the `minos-backend` process** (Postgres, Redis, and Caddy stay), use the binary path documented in **[vps-dev-binary.md](./vps-dev-binary.md)**. That flow builds a linux/amd64 binary off-box, rsyncs to `/opt/minos/releases/<sha>/`, and runs under systemd on `127.0.0.1:8787`.

**Do not** run the Docker `minos-backend` container and the systemd binary unit on port 8787 at the same time. Docker + GHCR remains the production SSOT; the binary path is a coexistence bypass, not a replacement for this document.

---

## Traffic path

```
Client ──HTTPS/WSS──► :443 Caddy (TLS / Let's Encrypt)
                          │
                          ▼
                  127.0.0.1:8787  minos-backend (Docker)
                          │
              ┌───────────┴───────────┐
              ▼                       ▼
       127.0.0.1:5432            127.0.0.1:6379
         Postgres                   Redis
```

Agents never run on the VPS. Host machines run `minos-daemon` and dial the same origin on `/ws/host`.

---

## Resource budget (3 vCPU / ~4 GiB)

| Process | mem_limit (compose) | Notes |
|---------|---------------------|--------|
| Postgres 16 | 768m | `shared_buffers=128MB`, `max_connections=50` |
| Redis 7 | 160m | AOF + `maxmemory 128mb` LRU |
| minos-backend | 512m | monolith (`MINOS_RUNTIME_MODE` default) |
| OS + Caddy + page cache | ~1.5–2G remaining | do not add second backend replicas |

Keep `MINOS_RUNTIME_MODE=monolith`. Split http/worker only after multi-instance is required.

---

## Prerequisites

1. DNS: `A minos.ainexc.com → <VPS public IPv4>`.
   - Prefer **DNS-only** (grey cloud) while Caddy issues Let's Encrypt via HTTP-01 on :80.
   - Orange-cloud proxy can be enabled later; long-lived WebSockets need adequate CF timeouts.
2. Host packages: Docker Engine + Compose plugin, Caddy, UFW (22/80/443), fail2ban recommended.
3. GHCR image published by `.github/workflows/backend-image.yml`.

---

## Layout on the VPS

| Path | Purpose |
|------|---------|
| `/opt/minos/deploy` | `docker-compose.yml` + `.env` |
| `/opt/minos/deploy/caddy` or `/etc/caddy/Caddyfile` | reverse proxy |
| `/opt/minos/data/postgres` | Postgres volume |
| `/opt/minos/data/redis` | Redis volume |
| `/opt/minos/backups` | `pg_dump` artifacts |
| `/opt/minos/logs` | Caddy access logs |

Sync **only** files under `deploy/prod/` from a trusted machine:

```bash
# from a laptop with the repo checked out (example)
rsync -av --delete \
  deploy/prod/docker-compose.yml \
  deploy/prod/.env.example \
  user@vps:/tmp/minos-prod/
# then install into /opt/minos as root (see below)
```

Never rsync the full monorepo.

---

## First bring-up

### 1. Install manifests

```bash
sudo mkdir -p /opt/minos/{deploy,data/postgres,data/redis,backups,logs}
sudo useradd --system --home /opt/minos --shell /usr/sbin/nologin minos || true
sudo chown -R minos:minos /opt/minos

# copy compose + env example from laptop (scp/rsync), then:
sudo cp docker-compose.yml /opt/minos/deploy/
sudo cp .env.example /opt/minos/deploy/.env
sudo chmod 600 /opt/minos/deploy/.env
sudo chown root:minos /opt/minos/deploy/.env
```

### 2. Edit `/opt/minos/deploy/.env`

```bash
MINOS_BACKEND_IMAGE=ghcr.io/<owner>/minos-backend:sha-<12hex>
POSTGRES_PASSWORD=<strong>
MINOS_JWT_SECRET=<openssl rand -hex 32>
MINOS_ENV=prod
MINOS_CORS_ORIGINS=https://minos.ainexc.com
```

Pin **immutable** `sha-…` tags in production. Use `latest` only for smoke tests.

### 3. GHCR login (private packages)

```bash
echo "$GHCR_TOKEN" | sudo docker login ghcr.io -u USERNAME --password-stdin
```

Public packages can skip login for pull.

### 4. Caddy

```bash
sudo cp Caddyfile /etc/caddy/Caddyfile   # from deploy/prod/Caddyfile
sudo caddy validate --config /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

Public path allowlist: `/health/live`, `/health/ready`, `/v1/*`, `/ws/*`.  
`/metrics` and `/openapi.json` stay off the public internet.

### 5. Start stack

```bash
cd /opt/minos/deploy
sudo docker compose pull
sudo docker compose up -d
sudo docker compose ps
curl -sS http://127.0.0.1:8787/health/live
curl -sS http://127.0.0.1:8787/health/ready
# after DNS + cert:
curl -sS https://minos.ainexc.com/health/live
curl -sS https://minos.ainexc.com/health/ready
```

### 6. Resource check

```bash
docker stats --no-stream
free -h
```

If available memory stays under ~300MB under light load, lower Postgres `shared_buffers` or backend `mem_limit` before adding features.

---

## Update backend

```bash
cd /opt/minos/deploy
# edit MINOS_BACKEND_IMAGE to the new sha tag
sudo docker compose pull minos-backend
sudo docker compose up -d minos-backend
sudo docker image prune -f
curl -sS https://minos.ainexc.com/health/ready
```

Migrations run on process boot (`sqlx::migrate!`). Schema is latest-only; coordinate wipe/rebuild for breaking migrations in pre-release.

---

## Backups

```bash
sudo cp backup-postgres.sh /opt/minos/backups/
sudo chmod +x /opt/minos/backups/backup-postgres.sh
sudo /opt/minos/backups/backup-postgres.sh
```

Suggested systemd timer: daily 03:15 UTC, retain 14 days (script already deletes older dumps).

---

## Client configuration

| Client | Variable | Production value |
|--------|----------|------------------|
| Mobile / daemon | `MINOS_BACKEND_URL` | `wss://minos.ainexc.com` (legacy `/devices` suffix still normalized) |
| Web | `VITE_MINOS_BACKEND_URL` | `https://minos.ainexc.com` |

Local dev defaults remain `127.0.0.1:8787`.

---

## Firewall

| Port | Public? |
|------|---------|
| 22 | yes (SSH) |
| 80 | yes (ACME + redirect) |
| 443 | yes (HTTPS/WSS) |
| 5432 / 6379 / 8787 | **no** — compose binds `127.0.0.1` only |

---

## Troubleshooting

| Symptom | Check |
|---------|--------|
| LE cert fails | DNS A record, port 80 open, Caddy log, grey-cloud DNS |
| `/health/ready` 503 | `docker compose logs postgres minos-backend`, `MINOS_DATABASE_URL` |
| CORS errors | `MINOS_CORS_ORIGINS` exact browser origin (no `*` in prod) |
| WS drops | Caddy `read_timeout 0` / `write_timeout 0` / `flush_interval -1` |
| OOM kills | `dmesg` / `docker events`, reduce mem_limits |
| Image pull denied | GHCR package visibility + `docker login` |

---

## Related

- Architecture: [architecture-backend.md](../architecture-backend.md), [architecture-overview.md](../architecture-overview.md)
- Secrets rotation: [secrets-rotation.md](./secrets-rotation.md)
- Image workflow: `.github/workflows/backend-image.yml`
- Compose SSOT: `deploy/prod/docker-compose.yml`
- Dev/agent binary bypass: [vps-dev-binary.md](./vps-dev-binary.md)
