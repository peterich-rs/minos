# 0010 · AgentEvent Raw variant

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-23 |
| Deciders | fannnzhang |

## Context

Plan 04 introduces a streamed `AgentEvent` surface that crosses Rust,
UniFFI-generated Swift, and flutter_rust_bridge-generated Dart. The typed
variants that exist today are the ones Minos already knows how to render or
reason about directly: token chunks, tool calls/results, reasoning, and done.

Codex's notification set will evolve independently from Minos. If Minos grows
the enum every time codex adds or renames a method, each protocol bump would
force a new Rust enum variant, new Swift/Dart generated bindings, and UI work
across surfaces that may not even consume the event yet.

At the same time, silently dropping unknown codex notifications is not
acceptable. The bridge needs a stable escape hatch so protocol churn does not
become either a silent data loss bug or a binding-churn tax on every release.

## Decision

- `AgentEvent` keeps a small typed core for events Minos handles directly and
  adds exactly one escape hatch variant:
  `Raw { kind: String, payload_json: String }`.
- `minos-agent-runtime::translate` maps unknown or not-yet-rendered codex
  notifications into `AgentEvent::Raw`, preserving the codex method name and
  the original `params` object as JSON text.
- Downstream consumers may ignore unknown `Raw` events safely. Future chat UI
  work can selectively promote specific `kind` values to first-class rendering
  without changing the bridge contract for every codex release.

## Consequences

**Positive**
- Codex protocol churn no longer forces a full Rust/Swift/Dart binding update
  for every new notification shape.
- Unknown notifications stay observable instead of being dropped, which keeps
  the bridge forward-compatible and debuggable.
- The typed `AgentEvent` surface remains small and deliberate while the chat UI
  is still deciding which events deserve first-class rendering.

**Neutral**
- Mobile and macOS consumers need a no-op fallback for `Raw` events they do not
  understand yet.
- Some semantics remain deferred until a later surface chooses to promote a
  specific raw `kind` into typed UI behavior.

## Alternatives Rejected

### Expand `AgentEvent` for every codex release

Rejected.

- It tightly couples Minos's public event surface to codex's release cadence.
- Every new codex notification would force regeneration across UniFFI and frb
  even when no current UI consumes the event.
- The maintenance cost lands in the wrong place: protocol bring-up work becomes
  binding churn instead of focused UI decisions.

### Rewrite `AgentEvent` as an untyped JSON envelope

Rejected.

- It throws away the typed variants Minos already benefits from for token,
  tool, reasoning, and completion events.
- It would push parsing complexity into every consumer instead of keeping the
  stable, high-value cases typed at the bridge boundary.
- It is a larger contract break for generated mobile bindings than adding one
  escape-hatch variant to the existing enum.