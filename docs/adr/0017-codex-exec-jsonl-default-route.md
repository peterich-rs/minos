# 0017 · codex exec/jsonl default route

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-28 |
| Deciders | fannnzhang |
| Supersedes | 0009 |

## Context

Minos originally integrated Codex through `codex app-server` over a private
loopback WebSocket. That worked for the initial daemon-side bridge, but the
runtime contract evolved in two ways:

1. `start_agent` now returns before the first user prompt is sent, so Minos
   needs a route that can mint a session immediately and defer the first turn.
2. The desired "remodex bridge" behavior is turn-oriented: start one local
   Codex execution, normalize streamed output into Minos's existing raw event
   shape, then resume the same Codex session on the next user turn.

Local Codex CLI inspection confirmed that `codex exec --json` and
`codex exec resume --json` are stable, available primitives. By contrast,
there is no stable Rust-consumable local IPC contract for the vendored
JavaScript remodex bridge, and `codex exec-server` remains experimental.

## Decision

- `minos-agent-runtime` uses `codex exec --json` for the first turn and
  `codex exec resume --json` for subsequent turns as the default production
  transport.
- `start_agent` mints a synthetic Minos thread id (`thr-exec-*`) immediately,
  transitions runtime state to `Running`, and emits `thread/started` without
  waiting for a prompt-bearing Codex subprocess.
- The runtime captures the real Codex session id from `session_meta` JSONL
  output and stores it only for future `resume` turns.
- Exec JSONL output is normalized back into Minos's existing raw Codex event
  methods (`thread/started`, `item/started`, `item/agentMessage/delta`,
  `item/reasoning/delta`, `item/toolCall/*`, `turn/completed`, plus synthetic
  `item/userMessage/delta` and `error`) so backend/mobile contracts do not
  change.
- The app-server WebSocket path remains only as a test seam for fake Codex
  integration tests.

## Consequences

**Positive**
- The default route now matches the available local Codex CLI primitives for
  prompt-oriented, resumable execution.
- Minos preserves its existing backend ingest shape and mobile UI contract
  without requiring a JavaScript bridge or a new externally visible protocol.
- Runtime startup is simpler: no loopback port probing or app-server child is
  required for normal production turns.

**Neutral**
- Minos thread ids are now runtime-generated identifiers distinct from the
  underlying Codex session id.
- A live session may have no child process between turns; only active turns own
  a running Codex subprocess.

**Negative**
- The runtime must synthesize a small compatibility layer to translate JSONL
  output back into the existing raw event vocabulary.
- Immediate follow-up sends need a short cleanup window while the previous
  `exec` child exits before `resume` can start.

## Alternatives Rejected

### Keep `codex app-server` as the default route

Rejected.

- It depends on a promptless `thread/start` session primitive that does not map
  cleanly to the current Minos `start_agent` / `send_user_message` split.
- It keeps loopback WS process management in the hot path even though the local
  CLI already exposes turn-oriented JSONL execution primitives.

### Reuse the vendored remodex JavaScript bridge as a black box

Rejected.

- The vendored bridge does not expose a stable, documented local IPC contract
  that the Rust runtime can consume directly.
- Its synthesized notifications do not match Minos's current raw Codex event
  shape closely enough to remove the need for a Rust-side normalization layer.