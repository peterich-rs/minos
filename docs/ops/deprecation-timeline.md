# Retired Backend Surfaces

This document records backend surfaces that are no longer mounted. There is no
runtime compatibility gate for these paths.

## Removed HTTP Routes

The legacy `/v1/threads/*` routes were removed. Clients must use:

- `POST /v1/agent-sessions/list`
- `POST /v1/agent-sessions/read-turns`
- `POST /v1/agent-sessions/start`
- `POST /v1/agent-sessions/send-input`
- `POST /v1/agent-sessions/stop`

The legacy caller-scoped `/v1/me/*` routes were removed. Clients must use:

- `POST /v1/pairing/list-hosts` for account-side host discovery
- `POST /v1/host/installations/self` for host-side self inspection
- `POST /v1/profiles/self`
- `POST /v1/profiles/minos-id`
- `POST /v1/profiles/display-name`
- `POST /v1/profiles/search`

The legacy account websocket ticket route and mixed websocket upgrade were
removed from supported clients. Clients must use:

- `POST /v1/realtime/ws-ticket`
- `GET /ws/client?ticket=...`

## Still Tracked

`X-Device-*` headers remain part of the host rail and local development tools.
They are not a supported account/mobile business identity surface.

`forward_rpc` and `paired_with` have been removed from the backend runtime.
