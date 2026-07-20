---
name: grok-agent-acp
description: use when building, reviewing, debugging, or documenting clients that integrate with Grok Build agent mode (ACP) via `grok agent stdio`, `grok agent serve`, or leader mode. applies to Minos TUI/runtime, custom product integrations, JSON-RPC ACP message design, session/prompt lifecycle, streaming session/update events, tool calls, permissions, x.ai extensions, and transport choice (stdio vs WebSocket serve vs leader UDS). do not use for plain `grok -p` headless streaming-json only.
---

# Grok Agent ACP Development

Use this skill to design, implement, review, or debug a client that drives **Grok Build** through its **Agent Client Protocol (ACP)** surface.

Primary docs (installed CLI):

- `grok agent --help`
- `grok agent stdio --help`
- `grok agent serve --help`
- `grok agent leader --help`
- User guide: `~/.grok/docs/user-guide/15-agent-mode.md`
- ACP standard: <https://agentclientprotocol.com>

Minos primary integration path (TUI / agent-runtime):

```bash
grok agent --no-leader stdio
```

Spawned by `minos-agent-runtime::grok_driver` over **JSON-RPC 2.0 newline-delimited frames on stdio**.

## Non-negotiable rules

1. Prefer **ACP agent mode**, not `grok -p --output-format streaming-json`, when the product needs tool visibility, permissions, multi-turn sessions, or resume.
2. On the wire use **JSON-RPC 2.0** with the `jsonrpc: "2.0"` field (unlike Codex app-server v2 which omits it).
3. Start every connection with exactly one `initialize` request before any other method. Then create or resume a session.
4. Distinguish:
   - Request: `method`, `params`, `id`
   - Response: `id` + exactly one of `result` / `error`
   - Notification: `method`, `params`, no `id`
5. Progress streams as **notifications** (`session/update` and `x.ai/*`). Keep reading after the prompt request is in flight; do not assume the only reply is the final `session/prompt` result.
6. Never silently auto-accept tool execution / destructive permissions in production clients. Surface `session/request_permission` to the user (or an explicit always-approve mode). **Minos** registers these on the shared approval pipeline (`approval/request` ingest → TUI overlay → `resolve_approval` → ACP `RequestPermissionResponse`). Host does **not** auto-timeout approvals (native CLI has none); pending waits until the user decides or the agent process dies.
7. For Minos-owned children, pass **`--no-leader`** so the agent process does not attach to a shared leader socket and steal another session’s backend.
8. Prefer **stdio** for local embedded integrations. Treat `agent serve` (WebSocket) as the multi-client / remote companion transport; bind loopback + secret.
9. Leader mode (`~/.grok/leader.sock` or `--leader-socket`) is a **shared backend multiplex**, not a public product API. Product bridges should speak ACP over stdio/serve, not raw leader control frames.

## Transports

| Transport | Command | When to use |
|-----------|---------|-------------|
| **stdio (recommended for Minos)** | `grok agent --no-leader stdio` | Local TUI/daemon spawn; process lifecycle = session backend |
| **serve (WebSocket)** | `grok agent serve --bind 127.0.0.1:PORT --secret TOKEN` | Long-lived server; multiple clients; IDE/remote bridge |
| **leader (UDS)** | `grok agent leader` / auto-spawn | Share one backend among TUI + headless + IDE; default sock `~/.grok/leader.sock` |
| **headless relay** | `grok agent headless --grok-ws-url wss://…` | Outbound relay for browser/remote UIs |

### stdio

```bash
grok agent --no-leader stdio
# optional:
#   -m <model>
#   --always-approve   # auto-approve tool executions (alias --yolo)
#   --agent-profile <PATH>
#   --leader-socket <PATH>   # only when intentionally using leader
```

Framing: one JSON object per line on stdin/stdout.

### serve (Codex app-server analogue)

```bash
grok agent serve --bind 127.0.0.1:2419 --secret "$GROK_AGENT_SECRET"
```

- Clients connect over WebSocket and authenticate with the secret.
- Agent can persist across client reconnects.
- Env: `GROK_AGENT_SECRET`.
- Loopback-only for untrusted networks unless you terminate TLS/auth elsewhere.

Compared to Codex:

| | Codex app-server | Grok agent serve |
|--|------------------|------------------|
| Protocol | Codex JSON-RPC v2 (no `jsonrpc` field) | ACP JSON-RPC 2.0 |
| Default local | often stdio child | Minos uses stdio; serve is optional |
| Session model | `thread/*` + `turn/*` | `session/*` |
| Stream unit | `item/*` notifications | `session/update` variants |
| Shared daemon | optional unix control socket | optional **leader** UDS |

## Minimal lifecycle

1. Spawn or connect.
2. `initialize` with `protocolVersion`, `clientCapabilities`, `clientInfo`.
3. Optionally `authenticate` if required by agent capabilities.
4. `session/new` with `cwd` + `mcpServers` **or** `session/load` / `session/resume`.
5. `session/prompt` with `sessionId` + `prompt: ContentBlock[]`.
6. Read `session/update` notifications until the `session/prompt` response (`stopReason`).
7. Handle `session/request_permission` (and optional `fs/*` / `terminal/*` server requests).
8. `session/cancel` to interrupt; `session/close` on teardown.
9. Persist `sessionId` for resume.

### Streaming `session/update` values

| `sessionUpdate` | Meaning | Minos UI mapping |
|-----------------|---------|------------------|
| `agent_message_chunk` | Assistant text chunk | `TextDelta` (new `message_id` after tool / `streamStartMs` change) |
| `agent_thought_chunk` | Reasoning chunk | `ReasoningDelta` (no Raw; thought does not close the open assistant message — same `message_id` is reused) |
| `tool_call` | Tool started | `ToolCallPlaced` (or suppressed for todo/wait/task plumbing → `Raw(grok/turn_activity)` activity only) |
| `tool_call_update` | Tool progress/result | `ToolCallCompleted` when completed/failed; orphan updates buffered (failed/cancelled orphans replay with `is_error: true`). **Tool body projection** prefers typed `raw_output` (pager parity) over model-noisy `content`: Edit→unified patch, Read→plain/densify line nos (not sparse `N→`), Bash→`output_for_prompt`+ANSI strip, Grep→`file_matches`, ListDir listing, etc. Never dump ToolOutput JSON. See `docs/architecture-grok-acp-projection.md`. |
| `plan` | Plan payload | `Raw` (`grok/plan`) |
| `current_mode_update` / `available_commands_update` / `session_info_update` | Meta | `Raw` |
| `params._meta` | `streamStartMs`, timestamps, `promptId`, tokens | Applied internally for segmentation only (`streamStartMs` change closes the open assistant message). **No `Raw(grok/notification_meta)` is emitted** — Grok stamps `_meta` on nearly every `session/update`, and emitting per-notification Raw events would flood ingest frames and break Desktop live-merge scroll. |

Grok projection lives in `minos-ui-protocol/src/grok.rs` and mirrors grok-build `AcpUpdateTracker`: close assistant text on tools and stream boundaries; prefer `rawInput.description` for tool titles.

### Core ACP methods

**Client → Agent**

| Method | Purpose |
|--------|---------|
| `initialize` | Handshake |
| `authenticate` | Auth method |
| `session/new` | Create session |
| `session/load` | Load history |
| `session/resume` | Resume session |
| `session/prompt` | User turn |
| `session/close` | Close session |
| `session/set_mode` / `session/set_config_option` | Session config |
| `session/list` | List sessions |
| `logout` | Logout |

**Client → Agent notifications**

| Method | Purpose |
|--------|---------|
| `session/cancel` | Cancel in-flight turn |

**Agent → Client requests**

| Method | Purpose |
|--------|---------|
| `session/request_permission` | Approve/deny tool |
| `fs/read_text_file` / `fs/write_text_file` | FS proxy |
| `terminal/*` | Terminal proxy |

### Grok extensions (`x.ai/*`)

Grok advertises extra methods under `x.ai/` (filesystem helpers, git/worktree, search, terminal, session fork/rewind/compact, auth, telemetry). Treat the set as **non-exhaustive** and discover via `initialize` capabilities / agent docs. Unknown methods should be answered with JSON-RPC `-32601` or ignored as notifications without crashing the client.

#### Plan mode reverse-requests (critical)

`enter_plan_mode` / `exit_plan_mode` are **GrokBuild tools** (namespace `grok_build`). They appear as normal `session/update` `tool_call` notifications (`read_only: true` in `_meta`), **but the shell parks the tool loop** until the client answers an ACP **extension reverse-request**.

ACP encodes extension methods with a **leading underscore** on the wire (see `agent-client-protocol` `ext_method` impl: `format!("_{}", method)`), and `ExtRequest` serializes with `#[serde(skip)] method` + `#[serde(transparent)]` so **params are the flat payload, not a nested envelope**. Concretely:

```json
{
  "jsonrpc": "2.0",
  "id": "<id>",
  "method": "_x.ai/exit_plan_mode",
  "params": {
    "sessionId": "...",
    "toolCallId": "...",
    "planContent": "# Plan ..."
  }
}
```

The JSON-RPC `result` is likewise the flat `ExtResponse` body (no wrapper):

```json
{ "jsonrpc": "2.0", "id": "<id>", "result": { "outcome": "approved" } }
```

| Wire JSON-RPC `method` | Flat `params` keys | Meaning | Reply `result` |
|------------------------|--------------------|---------|-----------------|
| `_x.ai/exit_plan_mode` | `sessionId`, `toolCallId`, `planContent?` | Present plan for approve / revise / abandon | `{ "outcome": "approved" \| "cancelled" \| "abandoned", "feedback"?: string }` |
| `_x.ai/ask_user_question` | `sessionId`, `toolCallId`, `questions`, `mode?` | Structured questions | `{ "outcome": "accepted", "answers": { "0": ["label"] } }` / `cancelled` / plan-mode extras |

`outcome` semantics for `exit_plan_mode`: `approved` → exit plan mode and implement; `cancelled` + optional `feedback` → stay in plan mode, feed revision notes back; `abandoned` → close plan mode. Malformed/transport errors fail **closed** (stay in plan mode).

**Do not** match the bare `x.ai/...` name or a synthetic `ext_method` string — the wire method always carries the `_` prefix. **Do not** unwrap params as `{method, params}` — they are flat. **Do not** reply with `-32601` to these: the shell treats unknown-extension errors as mid-approval disconnect, cancels the turn, and leaves plan mode active — the UI then hangs on `Running {exit_plan_mode}` until the user interrupts (`UserStopped`).

Minos path:

1. `grok_driver` pump strips the `_` prefix, dispatches `x.ai/exit_plan_mode` to `handle_grok_ext_method` → `register_grok_ext_method_approval`
2. Emit `approval/request` ingest (`method: x.ai/exit_plan_mode`) + track `PendingApprovalTarget::GrokExtMethod`
3. TUI `PendingAgentRequestKind::GrokPlanApproval` + approval overlay
4. `send_approval_decision` → `manager::resolve_approval` → `approvals::validate_grok_ext_method_decision` → ACP `reply({outcome, feedback?})`

## Minos wiring (source of truth in-repo)

| Layer | Path |
|-------|------|
| Domain enum | `crates/minos-domain/src/agent.rs` → `AgentName::Grok` |
| ACP types | `crates/minos-acp-protocol` (shared with Gemini) |
| Transport client | `crates/minos-agent-runtime/src/acp_client.rs` |
| Grok driver | `crates/minos-agent-runtime/src/grok_driver.rs` (`agent --no-leader stdio`) |
| Manager | `crates/minos-agent-runtime/src/manager.rs` (`start_grok_agent`, prompt pump) |
| UI translate | `crates/minos-ui-protocol/src/grok.rs` |
| TUI projection | `crates/minos-tui/src/translation/agent.rs` |
| Daemon projection | `crates/minos-daemon/src/store/event_writer.rs` |

Raw ingest kinds emitted by the runtime (same envelope as Gemini ACP):

- `user_message` — synthetic user text for persistence
- `acp_notification` — `{ method, params }`
- `acp_server_request` — `{ id, method, params }`
- `acp_prompt_response` — `{ stopReason }`
- `acp_error` / `acp_closed`

## MCP injection notes

Minos injects teamwork MCP as an ACP stdio server (`name`, `command`, `args`, `env: [{name,value}]`) using the untagged stdio shape (no Codex-style `transportType` field). Keep Grok MCP config in that shape.

## Implementation checklist

When adding or reviewing a Grok client:

- [ ] Uses `initialize` once per connection
- [ ] Creates/resumes `session` before prompt
- [ ] Correlates request `id`s; never blocks forever on a single read if notifications arrive interleaved
- [ ] Renders `agent_message_chunk` / `agent_thought_chunk` / `tool_call*` live
- [ ] Surfaces permissions (or documents `--always-approve` risk)
- [ ] Uses `--no-leader` for Minos-owned processes
- [ ] Persists `sessionId` for resume
- [ ] Tears down with `session/close` + child kill
- [ ] Does not hand-edit generated protocol crates incorrectly; extend `minos-acp-protocol` deliberately

## Local smoke

```bash
# Detect binary
which grok && grok --version

# Manual ACP stdio (type JSON lines carefully)
grok agent --no-leader stdio

# Minos TUI
cargo run -p minos-tui -- --agent grok --workspace /path/to/repo

# Runtime unit test (fake stdio agent)
cargo test -p minos-agent-runtime grok_send_user_message_runs_acp_prompt_and_returns_idle
```

## References in this skill

- `references/protocol-lifecycle.md` — request order and stop reasons
- `references/transports.md` — stdio / serve / leader comparison
- `assets/node-stdio-client.ts` — minimal Node ACP client skeleton
