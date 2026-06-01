# Deprecation Timeline

This document tracks deprecated surfaces in the Minos backend. Each entry
lists what is deprecated, the replacement, the removal target, and the
migration path for clients.

## Control

Deprecated routes are gated by the `MINOS_ENABLE_DEPRECATED_ROUTES` env var
(default: `true`). Set to `false` to disable deprecated routes at startup.
When disabled, deprecated endpoints return 404.

---

## Active Deprecations

### `/v1/threads/*` routes

| Field | Value |
|-------|-------|
| **Deprecated since** | Phase P8 |
| **Removal target** | Phase P10 or when mobile clients have migrated |
| **Replacement** | `/v1/agent-sessions/*` |
| **Env gate** | `MINOS_ENABLE_DEPRECATED_ROUTES` |

**Routes affected:**

- `POST /v1/threads`
- `POST /v1/threads/query`
- `POST /v1/threads/:thread_id/events`
- `POST /v1/threads/read`
- `POST /v1/threads/:thread_id/last_seq`
- `POST /v1/threads/last-seq`

**Migration path:** Clients should migrate to the agent-sessions surface:

- `POST /v1/agent-sessions/list` replaces thread listing
- `POST /v1/agent-sessions/read-turns` replaces thread event reads

**Metrics:** Hits to these routes are tracked by the
`minos_backend_deprecated_route_total{route}` Prometheus counter. Monitor
this metric to confirm client migration is complete before removal.

### `X-Device-*` headers

| Field | Value |
|-------|-------|
| **Status** | Active (host rail auth), deprecated for client rail |
| **Replacement** | Bearer token auth for client/mobile rail |

The `X-Device-Id`, `X-Device-Role`, `X-Device-Secret`, and `X-Device-Name`
headers are still actively used for the host (agent-host/Mac) authentication
rail. The client (mobile/iOS) rail has migrated to bearer token auth. These
headers remain for backward compatibility with existing host daemons.

### `forward_rpc` (envelope forwarding)

| Field | Value |
|-------|-------|
| **Status** | Active (legacy path) |
| **Replacement** | Direct `target_device_id` stamping on `Envelope::Forward` |

The `forward_rpc` reply-target mapping in `SessionHandle` is a compatibility
path for daemons that do not yet stamp `target_device_id` on replies. Once
all host daemons are updated, this path will be removed.

### `paired_with` (per-session field)

| Field | Value |
|-------|-------|
| **Status** | Removed |
| **Replacement** | Account-scoped pairing via `account_host_pairings` table |

The per-session `paired_with: Option<DeviceId>` field was removed in
ADR-0020 / Phase G. Multi-Mac pairing means there is no single peer to
derive from the session handle. All pairing state lives in the database.

---

## Removed (historical)

### `/v1/me/*` routes

| Field | Value |
|-------|-------|
| **Removed in** | Phase P8 (code retained but router not mounted) |
| **Replaced by** | `/v1/pairing/list-hosts`, `/v1/host/installations/self` |

The `/v1/me/hosts`, `/v1/me/peers`, and `/v1/me/peer` routes were retired.
The module code is retained in `src/http/v1/me.rs` for reference but is not
registered in the router.

---

## Monitoring Removal Readiness

Before removing a deprecated surface:

1. Check `minos_backend_deprecated_route_total` for zero traffic over 7+
   days in production.
2. Confirm no client SDK versions still reference the deprecated paths.
3. Set `MINOS_ENABLE_DEPRECATED_ROUTES=false` in staging for 48 hours to
   catch any remaining callers.
4. Remove the routes, store module, and route inventory entries.
