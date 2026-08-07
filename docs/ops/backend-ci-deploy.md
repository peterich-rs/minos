# Backend CI build + VPS deploy

Production app path: **linux binary + systemd** behind host Caddy.  
Postgres/Redis stay on Docker compose. Config SSOT: `/opt/minos/deploy/minos.env`.

| Path | Role |
|------|------|
| [backend-release.yml](../../.github/workflows/backend-release.yml) | Build artifact; deploy on tag / manual |
| [backend-image.yml](../../.github/workflows/backend-image.yml) | Optional GHCR image (compat / Docker backend) |
| `just deploy-backend-dev` | Local emergency deploy (same VPS layout) |

---

## Triggers

| Event | Build | Deploy to VPS |
|-------|-------|----------------|
| `push` `main` | yes → Actions artifact 14d | **no** |
| tag `backend-v*` (e.g. `backend-v0.1.0`) | yes | **yes** (auto) |
| `workflow_dispatch` with **Deploy** checked | yes | **yes** (default) |
| `workflow_dispatch` with Deploy unchecked | yes | no |

No `paths:` filter on the workflow so **tag pushes always run**. Main builds every push (rust-cache keeps this cheap). Optional image publish stays on `backend-image.yml` (path-filtered).

---

## GitHub configuration

### Secrets（仅 2 个，专用于 CI→VPS 部署）

| Secret | Required | Example / notes |
|--------|----------|-----------------|
| `MINOS_VPS_HOST` | for deploy | `root@23.95.95.156` |
| `MINOS_VPS_SSH_KEY` | for deploy | **private** key PEM/OpenSSH text; matching **public** key on VPS `authorized_keys` |

**不要**再为 health 加 secret。公网探测写死为 `https://minos.ainexc.com`（和线上 Caddy / 客户端 origin 一致）。  
Desktop 用的 `MINOS_BACKEND_URL` / `VITE_MINOS_BACKEND_URL` 是**客户端 bake**，和 CI SSH 部署无关，不要混成第三个「backend URL secret」。

CI **never** uploads or rewrites `minos.env`. JWT / DB / R2 stay on the server.

### Environment

Deploy job uses GitHub Environment **`production`**.  
Optional: Settings → Environments → `production` → required reviewers, wait timer, or restrict to `backend-v*` tags.

### VPS SSH deploy key（本地生成 → 公钥上 VPS → 私钥进 GitHub）

在**你自己的电脑**上做一次（不要把私钥提交进 git）：

```bash
# 1) 生成专用 key（空口令，Actions 无法交互输密码）
ssh-keygen -t ed25519 -f ./minos-vps-deploy -C "github-actions-minos-backend" -N ""

# 会得到：
#   ./minos-vps-deploy      ← 私钥 → 只进 GitHub Secret MINOS_VPS_SSH_KEY
#   ./minos-vps-deploy.pub  ← 公钥 → 装到 VPS

# 2) 公钥装到 VPS（用户须与 MINOS_VPS_HOST 一致，例如 root）
ssh root@23.95.95.156 'mkdir -p ~/.ssh && chmod 700 ~/.ssh'
cat ./minos-vps-deploy.pub | ssh root@23.95.95.156 'cat >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys'

# 3) 本机验证（应免密登录）
ssh -i ./minos-vps-deploy root@23.95.95.156 'echo ok && hostname'

# 4) GitHub → repo Settings → Secrets and variables → Actions → New repository secret
#    Name:  MINOS_VPS_HOST
#    Value: root@23.95.95.156
#
#    Name:  MINOS_VPS_SSH_KEY
#    Value: 私钥全文（含 -----BEGIN … KEY----- 到 END）
#    macOS 可：pbcopy < ./minos-vps-deploy

# 5) 本地私钥勿提交；可移出仓库目录或删掉本机副本（GitHub 已保存即可）
#    rm -f ./minos-vps-deploy ./minos-vps-deploy.pub   # 可选；留备份也行
```

Prefer a dedicated deploy key; do not reuse your daily personal SSH key long-term.

---

## Operator flows

### A. Tag-driven production release (recommended)

```bash
# after main is green and you intend to ship
git checkout main && git pull
git tag -a backend-v0.1.0 -m "backend 0.1.0"
git push origin backend-v0.1.0
# → backend-release.yml builds + deploys
# Watch: Actions → backend-release
```

Tag name must match `backend-v*` (same family as image tags in `backend-image.yml`).

### B. Manual deploy from Actions UI

1. Actions → **backend-release** → Run workflow  
2. Branch/tag: usually `main`  
3. Leave **Deploy binary to VPS after build** checked  
4. Run  

### C. Local emergency (bypass CI)

```bash
just deploy-backend-dev root@VPS
# or: ./deploy/dev-binary/deploy-backend.sh --host root@VPS
```

Same layout: `/opt/minos/releases/<sha>/`, `current`, `systemctl restart`.

### D. Config-only change

```bash
ssh root@VPS
sudoedit /opt/minos/deploy/minos.env
sudo systemctl restart minos-backend
/opt/minos/bin/healthcheck.sh --public https://minos.ainexc.com
```

---

## What deploy does

1. `cargo build --release -p minos-backend` on `ubuntu-latest`, `strip`  
2. Upload `minos-backend-linux-amd64` artifact  
3. If deploy: `deploy/dev-binary/deploy-backend.sh --skip-build` over SSH  
   - `releases/<12-char-sha>/minos-backend`  
   - `current` symlink  
   - helper scripts + unit refresh  
   - `systemctl restart minos-backend`  
   - loopback health via remote `healthcheck.sh`  
4. Public `https://…/health/live` + `/health/ready`

Rollback on VPS:

```bash
ln -sfn /opt/minos/releases/<old-sha> /opt/minos/current
systemctl restart minos-backend
/opt/minos/bin/healthcheck.sh --public https://minos.ainexc.com
```

---

## Related

- Env SSOT: [vps-dev-binary.md](./vps-dev-binary.md), `deploy/prod/minos.env.example`  
- Compose data plane / Caddy: [vps-deploy.md](./vps-deploy.md)  
- Optional image: `.github/workflows/backend-image.yml`
