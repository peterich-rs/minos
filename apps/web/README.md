# Minos Web

Standalone browser admin client for Minos.

Stack:

- React 19
- TypeScript
- Vite
- Framer Motion
- Space Grotesk + IBM Plex Mono

The web client is intentionally separate from the Flutter/mobile codepath. It
talks to the same backend contract:

- `POST /v1/auth/{register,login,refresh,logout,supabase}`
- `POST /v1/realtime/ws-ticket`
- `GET /v1/hosts` (Host Link list; bind hosts from Desktop)
- `POST /v1/agent-sessions/list`
- `POST /v1/agent-sessions/read-turns`
- `GET /ws/client?ticket=...`

## Local development

```bash
cp .env.example .env.local
pnpm install
pnpm dev
```

By default the app targets `http://127.0.0.1:8787`. Override with:

```bash
VITE_MINOS_BACKEND_URL=https://your-backend.example.com pnpm dev
```

### Supabase login (optional)

When `VITE_SUPABASE_URL` and `VITE_SUPABASE_ANON_KEY` are set, the auth screen
uses Supabase as the IdP (Google OAuth + email/password) and exchanges the
Supabase access token via `POST /v1/auth/supabase` for Minos session tokens.

The backend process needs matching IdP config:

```bash
export SUPABASE_URL=https://<project-ref>.supabase.co
export SUPABASE_JWT_AUD=authenticated   # default if omitted
# If exchange returns invalid_supabase_token, also set the legacy JWT secret
# from Dashboard → Project Settings → API → JWT Secret:
# export SUPABASE_JWT_SECRET='…'
export MINOS_JWT_SECRET='…at least 32 bytes…'
# Local SQLite is fine for dev:
cargo run -p minos-backend
```

No cross-compile or VPS is required for this path: run backend + web on the
same Mac against `127.0.0.1:8787`.

## Checks

```bash
pnpm lint
pnpm build
```

## Current surface

- **Auth**: Supabase (optional) → `POST /v1/auth/supabase` → Minos session; password fallback
- **Shell**: Desktop visual family (`ink` / `surface` tokens, sidebar chrome)
  - Work / Attention / Hosts / Settings with **mock** data (CloudPort next)
- Legacy demo routes under `src/components/*-workspace.tsx` are no longer mounted

Desktop UI SSOT: import pure presenters via `@/shared/*` → `apps/desktop/src/shared/*`
(see docs/architecture-desktop.md).

## Notes

- Browser websocket upgrades cannot send the mobile app's custom auth headers.
  The backend therefore exposes `POST /v1/realtime/ws-ticket`, and the client
  upgrades `/ws/client` with `?ticket=...`.
- Query surfaces now prefer POST-first `.../query` endpoints so host/mobile/web
  clients avoid pushing account-scoped filters and identifiers into URLs.
- This app currently keeps session state in `localStorage` for speed while the
  product surface is still under active construction.
