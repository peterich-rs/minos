# Local full-chain smoke (Desktop + Mobile simulator + local backend)

**Config SSOT:** repo-root [`.env.local`](../../.env.local) (gitignored).  
Schema / comments: [`.env.example`](../../.env.example).

**Public product domain:** `https://minos.ainexc.com` (VPS + Caddy).  
`minos.fan-nn.top` was only an old Cloudflare Tunnel experiment — do not use it for new work.

No public domain required for **local** smoke: backend binds `127.0.0.1:8787`, Mobile simulator uses loopback, Desktop/Web use Vite on localhost.

## Prerequisites

1. Fill / refresh root `.env.local` (already organized for LOCAL profile).
2. `just sync-local-env` (optional mirror into `apps/*/`).
3. Supabase Dashboard → Authentication → URL configuration:
   - **Site URL:** `http://localhost:5173`
   - **Redirect URLs:**  
     `http://localhost:5173/**`  
     `http://localhost:1420/**` (Desktop Vite)  
     `http://127.0.0.1:5173/**`  
     `http://127.0.0.1:1420/**`
4. Email auth enabled in Supabase (password sign-in / sign-up).

## One-time: clean local SQLite after schema wipe

```bash
cd /path/to/minos
rm -f minos-backend.db minos-backend.db-wal minos-backend.db-shm
```

## Tab commands (copy-paste)

Run each in a **separate terminal tab**, from the **repo root**.

> **Shell trap:** if your parent shell already `export`ed `MINOS_BACKEND_URL=wss://minos.fan-nn.top/...`,
> it **overrides** `.env.local` (just dotenv does not clobber existing exports).  
> Start each tab with:
>
> ```bash
> unset MINOS_BACKEND_URL MINOS_BACKEND_PUBLIC_URL
> ```
>
> Then `just print-local-env` should show `ws://127.0.0.1:8787/devices`.

### Tab 1 — Backend

```bash
cd /Users/zhangfan/develop/github.com/minos
unset MINOS_BACKEND_URL MINOS_BACKEND_PUBLIC_URL
just print-local-env
just backend
```

Health:

```bash
curl -sS http://127.0.0.1:8787/health/live
curl -sS http://127.0.0.1:8787/health/ready
```

### Tab 2 — Desktop (Host Link + local daemon)

Desktop UI (Vite, port **1420**):

```bash
cd /Users/zhangfan/develop/github.com/minos/apps/desktop
pnpm install   # first time only
pnpm dev
```

Daemon (needed for Link this Mac + agents) — typically started with Tauri:

```bash
cd /Users/zhangfan/develop/github.com/minos/apps/desktop
pnpm tauri dev
```

(`pnpm dev` alone is UI-only; full Host Link needs Tauri + daemon.)

### Tab 3 — Mobile simulator

```bash
cd /Users/zhangfan/develop/github.com/minos
just dev-mobile-ios
# or: just dev-mobile-android
```

`just` loads `.env.local` and passes:

- `--dart-define=MINOS_BACKEND_URL=…`
- `--dart-define=SUPABASE_URL=…`
- `--dart-define=SUPABASE_ANON_KEY=…`

### Tab 4 — Web (optional)

```bash
cd /Users/zhangfan/develop/github.com/minos/apps/web
pnpm install   # first time only
pnpm dev       # usually http://localhost:5173
```

## Smoke path

1. **Desktop:** Sign up / sign in with Supabase email+password (same project as Mobile).
2. **Desktop:** Ensure daemon online → **Link this Mac**.
3. **Mobile:** Same email+password → Hosts shows the Mac online.
4. Open a session / send a message / confirm stream.

## Switch profiles later

| Goal | Change in `.env.local` |
|------|-------------------------|
| Local loopback | Keep `127.0.0.1` (default) |
| Prod hub | Set `MINOS_BACKEND_URL=wss://minos.ainexc.com/devices` and `VITE_MINOS_BACKEND_URL=https://minos.ainexc.com` |

Do **not** point new clients at `minos.fan-nn.top`.

VPS binary deploy (when ready):

```bash
just deploy-backend-dev "${MINOS_VPS_DEPLOY_HOST:-root@YOUR_VPS}"
```

## Missing piece checklist

| Item | Local needed? |
|------|----------------|
| `SUPABASE_URL` + anon/publishable key | **Yes** (clients + exchange) |
| `SUPABASE_JWT_SECRET` | Only if JWKS verify fails (HS256 legacy) |
| `MINOS_JWT_SECRET` | **Yes** (backend session JWT) |
| Public domain / TLS | **No** for simulator + local desktop |
| Cloudflare Access | **No** for LOCAL profile |
| VPS SSH | Only for deploy tab, not for local smoke |
