# 0011 · Broker Envelope Protocol (kind-tagged routing vs jsonrpsee server)

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-23 |
| Deciders | fannnzhang |

## Context

ADR 0009 introduces a broker (`minos-relay`) that sits between paired clients. ADR 0004 previously committed the project to JSON-RPC 2.0 over WebSocket via `jsonrpsee`, with the Mac daemon acting as the jsonrpsee server and the mobile client as a jsonrpsee client.

The broker topology changes the relay's role fundamentally. The relay is responsible for two disjoint classes of message:

1. **Local RPCs that the relay itself handles.** A small, stable surface: `request_pairing_token`, `pair`, `forget_peer`, `ping`. These are relay-only and never travel to a peer.
2. **Peer-to-peer RPCs that the relay must forward opaquely.** The `#[rpc]` trait in `minos-protocol` (`list_clis`, future `subscribe_events`, and every business RPC yet unwritten) is a contract **between** clients. The relay has no reason to parse these, and every new method would otherwise force a relay-side code change.

Additionally, the relay must push server-originated events (`Paired`, `PeerOnline`, `PeerOffline`, `ServerShutdown`) that are not responses to any client request.

`jsonrpsee`'s server model assumes the server handles every method. Forcing it into a broker role would either require a catch-all `forward` method that defeats its type-safety advantages or duplicate every peer-to-peer RPC as a passthrough handler. Either choice tightly couples relay releases to product iteration on the peer-to-peer surface.

## Decision

Introduce an **envelope protocol** on the relay's `/devices` WebSocket. Every frame is a JSON object with a mandatory `v` (version) field and a `kind` tag that determines routing:

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Envelope {
    LocalRpc          { v: u8, id: u64, method: LocalRpcMethod, params: Value },
    LocalRpcResponse  { v: u8, id: u64, outcome: LocalRpcOutcome },
    Forward           { v: u8, payload: Value },
    Forwarded         { v: u8, from: DeviceId, payload: Value },
    Event             { v: u8, event: EventKind },
}
```

- `LocalRpc` → relay handles, relay responds with `LocalRpcResponse`. Method names are a typed enum (`LocalRpcMethod`), not a free string.
- `Forward` → relay looks up the sender's paired peer and emits `Forwarded` to that peer. `payload` is not parsed.
- `Forwarded` → delivered by the relay to a client; `from` identifies the originating peer.
- `Event` → server-pushed notifications (`Paired`, `PeerOnline`, `PeerOffline`, `Unpaired`, `ServerShutdown`).

`jsonrpsee`'s `#[rpc]` trait stays in `minos-protocol` as the authoritative schema for peer-to-peer RPCs. Each client runs `jsonrpsee` end-to-end against the opposite client, with the relay as a transparent transport for `payload`. Correlation of peer-to-peer responses uses the JSON-RPC 2.0 `id` inside `payload`; the relay never inspects it.

## Consequences

**Positive**
- Relay code is decoupled from peer-to-peer schema evolution. Adding a new business RPC (`run_codex`, `attach_skill`, etc.) is a change in `minos-protocol` + the two client impls; the relay is not touched.
- Relay remains simple and observable. The full set of backend-side methods fits in a table of four rows. The forward path is a DashMap lookup and a channel send.
- Kind tag makes the message intent unambiguous: "this is a backend RPC" vs "this is a peer message" vs "this is a server event" cannot be confused.
- Client-side `jsonrpsee` stays in play for the peer-to-peer surface — we keep its macro-generated types and stream-subscription primitives for free.
- Versioning is baked in from day one (`v: 1`). Future breaking changes bump to `v: 2` while the relay supports a transition window of both.

**Neutral / accepted cost**
- Two parallel protocol layers on the same socket (the relay's local RPC dispatcher and the peer-to-peer jsonrpsee stack). Documented in spec `minos-relay-backend-design.md` §6; the dual layering is explicit and small, not hidden.
- `payload` is `serde_json::Value` over the relay boundary. A peer could send malformed JSON-RPC that the relay would forward; the receiving peer rejects it via its own jsonrpsee error path. The relay does not attempt to validate and does not need to.
- Event schema lives on the relay side of the envelope. Growing event types requires a coordinated client update. Accepted in exchange for typed event variants instead of stringly-typed event streams.

**Negative / explicit trade-off**
- Debugging tools will see two layers of framing in logs: the envelope and, for forward traffic, the jsonrpsee payload inside. For MVP this is tolerable; ops tooling (log decoder, message inspector) can always unwrap the two layers programmatically.
- Golden files for the envelope schema become as load-bearing as those for peer-to-peer RPCs. One more set of fixtures to maintain.

## Alternatives Rejected

### Run `jsonrpsee::Server` on the relay, implement every peer-to-peer method as a passthrough

A single protocol, no envelope. Every method on the relay either handles locally (`pair`, `ping`) or forwards to the paired peer. Rejected:
- Every new client-to-client RPC requires adding a passthrough shim on the relay. Relay releases gate product iteration.
- "Forward" is fundamentally asymmetric with "handle": responses must come back from the callee, not the server. Bending `jsonrpsee`'s server model to this shape is possible but fights the framework.
- Server-pushed events (`Paired`, `PeerOnline`) must piggyback on subscriptions, requiring a parallel subscription surface that does not map cleanly to the semantics.

### Catch-all `forward(target: DeviceId, payload: Value)` method inside jsonrpsee

Keep `jsonrpsee` for the relay, expose one escape-hatch method. Rejected:
- Throws away type safety for the one case (routing) that benefits from it most. `target` is a string; mistakes are runtime, not compile-time.
- The "response" side is still asymmetric: the peer sends a separate `forward` for its response; correlation is the client's job anyway, which is exactly the envelope model but wrapped in an extra layer of method dispatch.

### Binary protocol (CBOR + custom framing)

A compact, typed alternative. Rejected on grounds consistent with ADR 0004: human readability during debugging outweighs bandwidth for our message rates, and we retain the option to layer compression later if profiling ever demands it.

### gRPC bidirectional streams

Stronger streaming primitives, clearer schema discipline. Rejected:
- Introduces a second codegen toolchain (proto compiler) for relay and both clients.
- `jsonrpsee` already exists in the workspace for peer-to-peer; adding gRPC only for relay-local RPCs is disproportionate overhead.
- Browsers cannot talk gRPC natively; we would need gRPC-Web or Connect, which reintroduces HTTP-framework complexity that `axum` already solves for free.
