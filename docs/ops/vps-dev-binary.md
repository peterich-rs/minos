# VPS dev/agent binary bypass (`minos-backend`)

**Urgency path for agents and local ops.** Build a linux/amd64 `minos-backend` binary on a trusted machine, ship it to the VPS, and run it under systemd behind the **existing Caddy**. Postgres, Redis, and Caddy stay as they are (usually Docker compose data plane).

| Path | Role |
|------|------|
| Docker `deploy/prod/` + GHCR | **Production SSOT** — see [vps-deploy.md](./vps-deploy.md) |
| This binary bypass | Dev/agent speed: no GHCR wait, no monorepo on VPS |

Do **not** replace Docker as the long-term production deploy model. Prefer GHCR images for durable releases.

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
| `/opt/minos/deploy/backend.env` | systemd `EnvironmentFile` (host loopback URLs) |
| `/opt/minos/deploy/docker-compose.yml` + `.env` | Unchanged Docker manifests (Postgres/Redis + optional image backend) |
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

## Env vars (`backend.env`)

Reuse the same secrets as compose `.env`. Hostnames differ: use **`127.0.0.1`**, not Docker DNS names `postgres` / `redis`.

Canonical template: [`deploy/dev-binary/env.example`](../../deploy/dev-binary/env.example).

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
scp deploy/dev-binary/env.example user@vps:/tmp/backend.env
ssh user@vps 'sudo install -m 600 -o root -g minos /tmp/backend.env /opt/minos/deploy/backend.env'
# edit secrets to match /opt/minos/deploy/.env
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
| Missing env | `/opt/minos/deploy/backend.env` from `env.example` |
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
| `deploy/dev-binary/env.example` | `EnvironmentFile` template |

Related: [vps-deploy.md](./vps-deploy.md), `deploy/prod/docker-compose.yml`, `Dockerfile`.
