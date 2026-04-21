# 0004 · Wire Protocol: JSON-RPC 2.0 over WebSocket

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-21 |
| Deciders | fannnzhang |

## Context

Mac and iOS communicate over a long-lived bidirectional channel. Required capabilities:

- Request / response (e.g., `list_clis`).
- Server-pushed streams for future agent events.
- Type-safe service definition shared between Rust ↔ Rust endpoints.
- Easy debuggability with off-the-shelf tools.
- Composition with Codex's `codex app-server`, which is itself JSON-RPC 2.0.

## Decision

JSON-RPC 2.0 over WebSocket, using `jsonrpsee` (>=0.24) for both server and client. Service definition uses the `jsonrpsee::proc-macros::rpc` macro to generate type-safe traits shared by both sides via the `minos-protocol` crate.

Streaming is expressed as JSON-RPC subscriptions (`jsonrpsee::subscription`).

## Consequences

**Positive**
- Native fit with `codex app-server`. Future Codex integration can pass through events with minimal wrapping.
- One macro, one trait definition; both server and client get the typed surface for free.
- Subscriptions handle streaming uniformly with the same protocol — no second transport for "events vs requests".
- Debuggable in any WebSocket inspector (Chrome DevTools, `wscat`); messages are human-readable.
- Mature ecosystem: jsonrpsee is well-maintained, used in production by Substrate / Polkadot.

**Neutral**
- JSON payloads are larger than binary alternatives. At MVP message rates this is irrelevant. Even at P1 streaming rates the bottleneck is far below WebSocket frame budget.
- Bandwidth cost matters only on cellular, where text payloads compress well over Tailscale's wire format anyway.

## Alternatives Rejected

### CBOR + custom envelope

A binary alternative with smaller payloads and broadly-supported tooling. Rejected:
- Loses human-readable debuggability — every inspection requires a decoder.
- Bandwidth gains are negligible at our scale; the tradeoff makes sense for embedded / IoT, not for ~20 messages/s mobile-developer workloads.
- Custom envelope = custom request/response correlation, custom streaming primitives, custom error semantics. JSON-RPC 2.0 standardizes all of these.

### Protocol Buffers + gRPC-Web / Connect

Heavier toolchain (proto compiler, generated code in Rust + Dart + Swift), more rigid contract evolution. Rejected because:
- gRPC-Web requires a proxy in browsers; we do not need a browser path in MVP, but needing a separate codegen pipeline for Dart and Swift adds maintenance overhead.
- Schema evolution discipline (field numbers, proto2 vs proto3) is not free.
- The reuse benefit (Codex `app-server` already speaks JSON-RPC) is forfeited.
- `jsonrpsee` macros give us comparable type safety with much less ceremony.
