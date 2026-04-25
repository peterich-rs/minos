# 0013 · `minos-ui-protocol` Unified Event Shape (one viewer, three CLIs)

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-24 |
| Deciders | fannnzhang |

## Context

The mobile app must render a chronological view of agent activity originating from at least three different CLIs — `codex`, `claude`, and `gemini`. Each one has its own native streaming protocol shape (Codex: `item/started`, `item/agentMessage/delta`, `item/toolCall/*`, `turn/completed`, …; Claude and Gemini ship comparable but not identical event vocabularies).

Earlier prior art (`AgentEvent` in `minos-domain`, see ADR 0010-agent-event-raw-variant.md) attempted a flatter shape with a fall-through `Raw` variant. Two problems made it inadequate as the mobile contract:

1. **Where the translation lives matters.** `AgentEvent` was emitted by `minos-agent-runtime` on the host. The relay was a dumb broker that forwarded already-translated events; mobile then had to keep translator state to do the *same* translation on history reads (because raw events weren't on the relay). This forced the translation logic to be duplicated on host *and* mobile or for the host to push every translated event into the relay's database — neither of which the architecture supported.
2. **The discriminator was at the wrong level.** `AgentEvent::Raw { method, payload }` mixed lifecycle events (`thread/started`, `thread/closed`) with content (`agentMessage/delta`) at the same level. A mobile-side state machine that wants to render "one tile per text-delta accumulated under one assistant message" had to re-derive message boundaries from the stream every time.

The plan-05 spec also explicitly scopes the mobile app to a *deliberately plain* debug viewer. That makes a flat, kind-tagged event the natural rendering primitive: one `ListTile` per `UiEventMessage`. A future chat UI is a separate design pass.

## Decision

Introduce a new dedicated crate, **`minos-ui-protocol`**, that owns:

- One enum `UiEventMessage` whose variants are the union of meaningful events any agent CLI can emit, tagged at the **discriminator level the UI cares about**. Variants today:

  ```text
  ThreadOpened / ThreadTitleUpdated / ThreadClosed
  MessageStarted / MessageCompleted
  TextDelta / ReasoningDelta
  ToolCallPlaced / ToolCallCompleted
  Error
  Raw
  ```

- One translator function per CLI: `translate_codex(state, raw)` is fully implemented; `translate_claude` and `translate_gemini` are typed stubs returning `TranslationError::NotImplemented`. The codex translator carries per-thread state (`CodexTranslatorState`) for tool-call argument buffering and open-message tracking.

- A `Raw { kind, payload_json }` escape hatch that any translator can emit when it sees a method it doesn't recognize. The viewer renders `Raw` as a monospace `kind: payload` line so unknown events stay visible without crashing the UI or stalling the stream.

The crate is consumed in two places:
- **Backend** (`minos-backend::ingest`): runs `translate_*` on every persisted raw event before fanning out.
- **Mobile** (`minos-mobile::client`): receives the already-translated `UiEventMessage` over the wire and renders it directly. **Mobile owns no translation logic.**

The native event format stays available — backend persists raw events under `(thread_id, seq)` and re-runs the translator on history reads, so a translator change ships fully retroactive without a backfill.

Spec references: §5.2 (single-shape rationale), §6.4 (the enum + serde shape), §6.5 (translator contract).

## Consequences

**Positive:**
- One Dart code path renders any agent. Adding a new CLI (e.g. `aider`) means adding a new `translate_<cli>` and zero changes to the viewer.
- Translator changes are retroactive: backend re-translates from `raw_events` on read, so a bug fix or a new variant in `UiEventMessage` lights up old threads automatically.
- The `Raw` escape hatch is forward-compatible: a host running an older `minos-ui-protocol` than the backend sees unknown methods as `Raw` instead of erroring.
- Tests live close to the data: `crates/minos-ui-protocol/tests/golden/codex/*` are 12 input/expected pairs covering every codex method we currently translate. Adding a fixture is the diff a new contributor would expect.

**Negative:**
- Per-thread translator state (`CodexTranslatorState`) is mutable and lives in a `DashMap` on the backend. A bug there manifests as wrong UI events for one thread without a per-frame error signal. Mitigated by: history reads use a *fresh* state, so every read is deterministic regardless of live-stream state corruption (see plan §C2).
- Two separate `UiEventMessage` deserializers exist (Rust `serde` + Dart frb mirror). Drift between them is caught by frb codegen drift in `cargo xtask check-all`, but only structurally — semantic drift in a new variant requires a deliberate mirror update.
- The codex translator depends on the codex app-server's ordering guarantees (`item/started` before `item/.../delta`, `argumentsCompleted` before `completed`). A future codex update that relaxes ordering would force the translator to buffer differently. Out of scope for this ADR.

**Out of scope:**
- A real chat UI on top of `UiEventMessage`. The current viewer is intentionally plain (one tile per event). When a chat design lands, the protocol does not need to change — only the viewer.
- Translator implementations for claude and gemini. Both are `NotImplemented` stubs; `MinosError::TranslationNotImplemented { agent }` is the surface a future CLI bring-up will turn green.
