# 0009 · codex app-server WS transport

| Field | Value |
|---|---|
| Status | Superseded by 0017 |
| Date | 2026-04-23 |
| Deciders | fannnzhang |

This ADR captured the initial bridge choice during app-server bring-up. The
current default production route now lives in
`0017-codex-exec-jsonl-default-route.md`.


## Context

Plan 04 adds a daemon-side bridge to `codex app-server`. Minos already has
an outward WebSocket surface for mobile peers on the Tailscale address; the
new bridge adds a second, inward transport between `minos-agent-runtime`
and the spawned `codex` child.

The key choice is whether that child connection should use loopback
WebSocket or raw stdio pipes. The design spec for plan 04 intentionally
pairs Minos with codex's IDE-integration model: spawn `codex app-server`,
connect over JSON-RPC, and keep Minos responsible for child supervision.

That leaves three concrete pressures:

1. **Alignment with codex's first-class path.** `codex app-server --listen
   ws://127.0.0.1:<port>` is the integration shape codex already treats as
   primary for editor-style clients. Minos should follow that path rather
   than invent a private transport.
2. **Debuggability.** During the bring-up phase, maintainers need to inspect
   raw traffic and failure modes quickly. A loopback WS transport is easy to
   probe, log, and compare against codex examples.
3. **Operational ownership.** Minos already supervises the daemon's own
   runtime and outward WS server. Adding one more loopback port plus child
   lifecycle ownership is acceptable if it keeps the codex bridge explicit.

## Decision

- `minos-agent-runtime` spawns `codex app-server` as a supervised child and
  connects to it over `ws://127.0.0.1:<port>` using JSON-RPC 2.0.
- The codex-facing socket binds only on loopback. It is a private process-to-
  process channel, not a new externally reachable Minos endpoint.
- Port selection and child teardown stay inside `minos-agent-runtime`, so the
  daemon layer owns one runtime handle rather than transport details.
- Minos accepts the cost of one extra loopback port in exchange for a
  transport that matches codex's documented integration path.

## Consequences

**Positive**
- The bridge matches codex's own app-server model instead of introducing a
  Minos-only transport adapter.
- Local debugging is simpler: WS frames are visible to standard tooling and
  tracing can describe connection lifecycle in transport-native terms.
- The codex bridge stays structurally similar to the rest of Minos's networked
  surfaces: JSON-RPC over WebSocket on top of Tokio.

**Neutral**
- Every live agent session now owns one extra loopback port in addition to the
  daemon's outward peer-facing server.
- `minos-agent-runtime` must explicitly supervise child startup, connection,
  shutdown, and crash detection instead of delegating all lifecycle details to
  an inherited pipe.

## Alternatives Rejected

### stdio pipes between daemon and codex

Rejected.

- It diverges from codex's own app-server examples, so Minos would carry a
  custom integration shape with less upstream guidance.
- Pipe traffic is harder to inspect during bring-up than a loopback WS stream,
  which makes protocol debugging slower.
- The child-supervision cost still exists, so stdio saves little while giving
  up transport-level debuggability and symmetry with the IDE-integration path.