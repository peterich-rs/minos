# Local full-chain smoke (Desktop + Mobile simulator + local backend)

**Config SSOT:** repo-root [`.env.local`](../../.env.local) (gitignored).  
Schema / comments: [`.env.example`](../../.env.example).

No public domain required for this profile: backend binds `127.0.0.1:8787`, Mobile simulator uses loopback, Desktop/Web use Vite on localhost.

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

### Tab 1 — Backend

```bash
cd /Users/zhangfan/develop/github.com/minos
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
| Local loopback | Keep section 2 as `127.0.0.1` (default) |
| Prod hub | Use section 5 `MINOS_PROD_*` values for `MINOS_BACKEND_URL` / `VITE_*` |
| Tunnel + Access | Point WS at `MINOS_TUNNEL_WS`; keep CF Access secrets |

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
