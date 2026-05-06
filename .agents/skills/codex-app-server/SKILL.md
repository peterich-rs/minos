---
name: codex-app-server
description: use when building, reviewing, debugging, or documenting clients that integrate with codex app-server using the json v2 schema only. applies to custom product integrations, local/desktop/web clients, json-rpc message design, stdio or websocket transports, thread/turn lifecycle, streaming item events, approvals, skills/apps invocation, filesystem v2 APIs, schema generation, and type-safe client implementation. do not use for legacy v1 schemas or generic codex sdk automation.
---

# Codex App Server Development

Use this skill to help an agent design, implement, review, or debug a client that calls `codex app-server` using the JSON v2 schema only.

Primary source of truth: <https://developers.openai.com/codex/app-server>. The app-server schema is version-specific; always prefer artifacts generated from the exact installed `codex` binary over stale copied types.

## Non-negotiable rules

1. Cover JSON v2 only. Do not discuss or implement legacy v1 schema, migration paths from v1, or compatibility shims unless the user explicitly asks for a comparison; even then, keep implementation guidance v2-only.
2. Treat app-server as a JSON-RPC-like protocol where messages omit the `jsonrpc` field on the wire.
3. Generate schema/types for the user's installed Codex version before locking request/response shapes:
   - `codex app-server generate-ts --out ./schemas`
   - `codex app-server generate-json-schema --out ./schemas`
4. Start every client connection with exactly one `initialize` request followed by an `initialized` notification before any other method.
5. Distinguish requests, responses, and notifications:
   - Request: `method`, `params`, `id`.
   - Response: `id` plus exactly one of `result` or `error`.
   - Notification: `method`, `params`, no `id`.
6. Use stable APIs by default. Only set `capabilities.experimentalApi: true` when the requested feature is explicitly experimental and the user accepts that risk.
7. Never auto-accept command execution, file changes, destructive app/tool calls, or network access approvals in production client designs. Surface approval requests to the user with thread/turn scoped UI.
8. Prefer `stdio` for supported local integrations. Treat WebSocket as experimental/unsupported; if used, bind to loopback and configure auth.

## Workflow

### 1. Classify the integration

Before writing code, identify which client pattern the user needs:

- Local embedded client: spawn `codex app-server` over stdio.
- Desktop/web local companion: connect to loopback WebSocket only when explicitly needed.
- Product UI integration: render threads, turns, items, deltas, approvals, and file diffs.
- Automation/CI: redirect the user toward Codex SDK unless they specifically need app-server UI semantics such as history, approvals, or streamed agent events.

### 2. Generate and inspect v2 schemas

If the environment has Codex installed, run the schema generation commands. Use generated TypeScript types or JSON Schema names from that output. If not available, state that exact field names must be verified against generated schemas for the target Codex version and proceed with documented v2 examples.

Use `references/json-v2-protocol.md` for protocol constraints and message framing.
Use `references/client-implementation.md` for implementation patterns and state machines.
Use `references/method-catalog.md` for method families and when to use each.

### 3. Build the minimal lifecycle first

Implement and test this sequence before adding features:

1. Spawn or connect to app-server.
2. Send `initialize` with `clientInfo`.
3. Send `initialized` notification.
4. Optionally call `model/list` and render model options.
5. Call `thread/start` or `thread/resume`.
6. Call `turn/start` with text/image/localImage input items.
7. Read notifications until `turn/completed`.
8. Accumulate `item/agentMessage/delta` and finalize state from `item/completed`.
9. Handle `error` events and failed turns.
10. Persist `thread.id` for resume.

### 4. Add approvals and side effects

When app-server sends server-initiated approval requests, render a user decision UI and respond with the selected decision. Required approval flows include:

- `item/commandExecution/requestApproval`
- `item/fileChange/requestApproval`
- `item/tool/requestUserInput`
- app/MCP tool-call approval prompts that may arrive via `tool/requestUserInput`

Always scope approvals by `threadId` and `turnId`. Treat `item/completed` as the final authoritative state for command and file-change items.

### 5. Add advanced client features selectively

Only add these when needed:

- `turn/steer` for adding input to an active turn.
- `turn/interrupt` for cancellation.
- `thread/fork`, `thread/rollback`, `thread/compact/start`, and archive/unarchive for conversation management.
- `fs/*` v2 APIs for absolute-path filesystem UI operations and `fs/watch` invalidation.
- `skills/list`, skill input items, and `skills/config/write` for skill-aware clients.
- `app/list`, mention input items, and app config RPCs for connector-aware clients.
- `command/exec` for standalone sandboxed commands outside a thread/turn, not for routine agent turns.

## Output expectations

When asked to create implementation guidance, produce:

1. Architecture summary.
2. Transport choice and security assumptions.
3. Generated-schema step.
4. Message lifecycle/state machine.
5. TypeScript or relevant-language client skeleton.
6. Event handling table.
7. Approval handling strategy.
8. Error/retry handling.
9. Testing checklist.

When reviewing code, check:

- No `jsonrpc` property is sent.
- `initialize` precedes all other requests and is not repeated on the same connection.
- Requests use unique `id` values and correlate responses correctly.
- Notifications are accepted without `id`.
- The client keeps reading after request responses because progress arrives as notifications.
- Final item state comes from `item/completed`, not only deltas.
- WebSocket mode is loopback/authenticated and implements retry for server overload code `-32001`.
- Approval requests are not silently accepted.
- Experimental methods/fields require `capabilities.experimentalApi: true`.
- `fs/*` calls use absolute paths.

## Bundled helper

Use `scripts/validate_json_v2_messages.py` to sanity-check newline-delimited JSON examples or captured protocol logs for common JSON v2 mistakes. It does not replace generated JSON Schema validation.

Example:

```bash
python scripts/validate_json_v2_messages.py examples/app-server.jsonl
```
