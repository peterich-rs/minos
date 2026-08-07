# VPS binary deploy (`minos-backend`)

**Production app path:** linux/amd64 `minos-backend` under systemd behind host **Caddy**. Postgres/Redis stay on Docker compose. Config SSOT: `/opt/minos/deploy/minos.env`.

| Path | Role |
|------|------|
| [backend-ci-deploy.md](./backend-ci-deploy.md) + `backend-release.yml` | **CI build + tag/manual deploy** (preferred) |
| `just deploy-backend-dev` / this doc | Local/emergency deploy (same VPS layout) |
| Docker image backend + GHCR | Optional fallback via `switch-to-docker.sh` — see [vps-deploy.md](./vps-deploy.md) |

CI does **not** write secrets; only the binary and unit/helpers are shipped.

---

## Mutual exclusion (port 8787)

Only one process may bind `127.0.0.1:8787`:

- Docker compose service `minos-backend`, **or**
- systemd unit `minos-backend` (this doc)

Running both fails (bind error) or causes flaky routing. Always use the switch scripts.

---

## Layout on the VPS

| Path | Purpose |
|------|---------|
| `/opt/minos/releases/<git-sha>/minos-backend` | Immutable release binary |
| `/opt/minos/current` → `releases/<git-sha>` | Active symlink |
| `/opt/minos/deploy/minos.env` | systemd `EnvironmentFile` (host loopback URLs) |
| `/opt/minos/deploy/docker-compose.yml` + `.env` → `minos.env` | Docker manifests (Postgres/Redis + optional image backend); `.env` is a symlink to the SSOT |
| `/opt/minos/bin/*.sh` | `healthcheck`, `switch-to-binary`, `switch-to-docker` |
| `/etc/systemd/system/minos-backend.service` | Unit (from `deploy/dev-binary/minos-backend.service`) |

Still **do not clone the monorepo** onto the VPS.

---

## Prerequisites

**Laptop / agent host**

- Docker Engine (default build method; works on macOS via `linux/amd64` platform)
- `rsync`, `ssh`, `git`
- Optional alternatives: [cross-rs](https://github.com/cross-rs/cross), [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) + Zig, or a native Linux builder

**VPS**

- Existing production layout from [vps-deploy.md](./vps-deploy.md): Caddy on :443, Postgres/Redis on loopback, `/opt/minos`
- System user `minos` (same as first bring-up)
- Port `8787` free when starting the binary unit

---

## Env vars (`minos.env` SSOT)

Single SSOT shared with compose (`.env` → symlink). For the binary path, DB/Redis hostnames must be **`127.0.0.1`**, not Docker DNS names `postgres` / `redis`.

Canonical template: [`deploy/prod/minos.env.example`](../../deploy/prod/minos.env.example).

| Variable | Notes |
|----------|--------|
| `MINOS_BACKEND_LISTEN` | `127.0.0.1:8787` (Caddy reverse_proxy target) |
| `MINOS_STORAGE_MODE` | `external-sql` |
| `MINOS_DATABASE_URL` | `postgres://…@127.0.0.1:5432/…` |
| `MINOS_CACHE_BACKEND` | `redis` |
| `MINOS_MESSAGE_BUS_BACKEND` | `redis` |
| `MINOS_REDIS_URL` | `redis://127.0.0.1:6379/` |
| `MINOS_JWT_SECRET` | Same as compose; ≥32 bytes |
| `MINOS_ENV` | `prod` for VPS |
| `MINOS_CORS_ORIGINS` | Exact browser origin (no `*`) |
| `RUST_LOG` | e.g. `info,minos_backend=info` |

Optional Supabase keys (`SUPABASE_*`) only if that environment uses cloud identity exchange.

```bash
# one-time on VPS (from laptop)
scp deploy/prod/minos.env.example user@vps:/tmp/minos.env
ssh user@vps 'sudo install -m 640 -o root -g minos /tmp/minos.env /opt/minos/deploy/minos.env'
ssh user@vps 'sudo ln -sfn /opt/minos/deploy/minos.env /opt/minos/deploy/.env'
# edit secrets in /opt/minos/deploy/minos.env only
```

---

## One-command deploy (from monorepo checkout)

```bash
# build linux binary + rsync + install unit + restart
just deploy-backend-dev user@vps

# or without just:
./deploy/dev-binary/deploy-backend.sh --host user@vps
```

Useful flags:

```bash
./deploy/dev-binary/deploy-backend.sh --help
./deploy/dev-binary/deploy-backend.sh --host user@vps --dry-run
./deploy/dev-binary/deploy-backend.sh --host user@vps --skip-build   # reuse dist/
./deploy/dev-binary/deploy-backend.sh --host user@vps --build-method zig
./deploy/dev-binary/deploy-backend.sh --host user@vps --no-restart   # upload only
```

**First time** switching off the Docker backend container:

```bash
ssh user@vps 'sudo /opt/minos/bin/switch-to-binary.sh'
```

That stops only compose `minos-backend`, keeps `postgres` + `redis`, enables/restarts systemd.

---

## Build-only

```bash
./deploy/dev-binary/build-linux-backend.sh --help
./deploy/dev-binary/build-linux-backend.sh                  # docker (default)
./deploy/dev-binary/build-linux-backend.sh --method cross
./deploy/dev-binary/build-linux-backend.sh --method zig
./deploy/dev-binary/build-linux-backend.sh --out /tmp/minos-backend
```

Default output: `dist/minos-backend-linux-amd64`.

macOS hosts **cannot** upload a Darwin binary. Default Docker method builds `linux/amd64` and extracts `/app/minos-backend` from the image (same Dockerfile as GHCR).

---

## Health checks

```bash
# on VPS
/opt/minos/bin/healthcheck.sh
/opt/minos/bin/healthcheck.sh --wait 60
/opt/minos/bin/healthcheck.sh --public https://minos.ainexc.com

# loopback endpoints (same as Docker path)
curl -sS http://127.0.0.1:8787/health/live
curl -sS http://127.0.0.1:8787/health/ready
```

---

## Roll back to Docker (production path)

```bash
ssh user@vps 'sudo /opt/minos/bin/switch-to-docker.sh'
```

This stops/disables systemd `minos-backend`, then `docker compose up -d` under `/opt/minos/deploy`. Caddy is untouched. See [vps-deploy.md](./vps-deploy.md) for image pin updates (`MINOS_BACKEND_IMAGE`).

---

## Traffic path (unchanged ingress)

```
Client ──HTTPS/WSS──► :443 Caddy
                          │
                          ▼
                  127.0.0.1:8787  minos-backend  ← Docker *or* systemd binary
                          │
              ┌───────────┴───────────┐
              ▼                       ▼
       127.0.0.1:5432            127.0.0.1:6379
```

---

## Troubleshooting

| Symptom | Check |
|---------|--------|
| `address already in use` :8787 | Other backend still up — run the correct switch script |
| `/health/ready` 503 | `journalctl -u minos-backend -e`, `MINOS_DATABASE_URL` host must be `127.0.0.1`, Postgres container up |
| Binary is Mach-O | Built on macOS without docker/cross — rebuild with `--method docker` |
| `Permission denied` on binary | `chown minos:minos` release dir; unit runs as `User=minos` |
| Missing env | `/opt/minos/deploy/minos.env` from `deploy/prod/minos.env.example` |
| Apple Silicon slow build | Docker `linux/amd64` emulation; use a Linux x86_64 builder or CI artifact when available |

---

## Repo files

| File | Role |
|------|------|
| `deploy/dev-binary/build-linux-backend.sh` | Build linux binary |
| `deploy/dev-binary/deploy-backend.sh` | Build + rsync + unit + restart |
| `deploy/dev-binary/healthcheck.sh` | `/health/live` + `/health/ready` |
| `deploy/dev-binary/switch-to-binary.sh` | Docker backend → systemd |
| `deploy/dev-binary/switch-to-docker.sh` | systemd → Docker backend |
| `deploy/dev-binary/minos-backend.service` | systemd unit template |
| `deploy/prod/minos.env.example` | Single VPS env SSOT template |
| `deploy/dev-binary/merge-minos-env.sh` | Merge legacy dual env → `minos.env` |

Related: [vps-deploy.md](./vps-deploy.md), `deploy/prod/docker-compose.yml`, `Dockerfile`.
