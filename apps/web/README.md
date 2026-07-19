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

- `POST /v1/auth/{register,login,refresh,logout}`
- `POST /v1/realtime/ws-ticket`
- `POST /v1/pairing/list-hosts`
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

## Checks

```bash
pnpm lint
pnpm build
```

## Current surface

- Browser-admin login/register
- Host discovery
- Thread list + thread history
- Live websocket subscription via short-lived ws ticket
- Start thread / send follow-up turn / close thread

## Notes

- Browser websocket upgrades cannot send the mobile app's custom auth headers.
  The backend therefore exposes `POST /v1/realtime/ws-ticket`, and the client
  upgrades `/ws/client` with `?ticket=...`.
- Query surfaces now prefer POST-first `.../query` endpoints so host/mobile/web
  clients avoid pushing account-scoped filters and identifiers into URLs.
- This app currently keeps session state in `localStorage` for speed while the
  product surface is still under active construction.
