# Minos Teamwork Agent Injection

This note records how Minos exposes the current conversation to CLI agents and
which instruction surfaces are available for Codex, Claude, Gemini, and
OpenCode.

## Current Model

All Minos MCP injections name the server `minos_teamwork`. The server is bound
to the conversation that started the agent with:

```text
--conversation-id <conversation_id> --source-agent <agent> --source-thread-id <session_id> --socket-path <path>
```

Agents started outside a conversation do not receive the teamwork MCP server.
Conversation-bound Codex and OpenCode runtime instances include the source
session id in their cache key so a process configured for one agent session is
not reused for another session in the same conversation. Persisted conversation
sessions restore the same conversation id when they are registered back into the
runtime, so resumed sessions keep their conversation-bound MCP context.

## Tool Set

The MCP server exposes only conversation-scoped tools:

- `list_conversation_messages`
- `delegate_to_agent`
- `get_delegation_status`
- `wait_delegation`
- `cancel_delegation`
- `post_conversation_update`

`list_conversation_messages` reads daemon conversation messages. Each message may
include structured `delegation_id`, `reply_to_message_id`, and `mentions` in
addition to `body`.
`post_conversation_update` appends a user-visible message to the same
conversation using the source agent/thread metadata from the MCP sidecar. If
the update body starts with `@agent` or `@agent#short_thread`, Minos also sends
the clean body to that target thread; `@agent` starts a new conversation-bound
session, while `@agent#short_thread` routes to the exact existing session.
Successful appends publish `ConversationMessageAppended` so TUI refreshes.
`delegate_to_agent` starts the target agent through the same conversation start
path as UI (`start_agent_in_conversation` + optional host profile launch options).
Tool args:

| Arg | Role |
| --- | --- |
| `prompt` | Required task text |
| `target_agent` | Runtime name (`codex`/`claude`/…); optional when `profile_id` or `target_profile` is set |
| `profile_id` | Preferred stable host profile id; daemon loads profile and sets model/effort/instructions |
| `target_profile` | Profile display name (unique case-insensitive match); ignored when `profile_id` is set |

When only `target_agent` is given, Minos applies the **newest** host profile for
that runtime if one exists (desktop bare-`@agent` convenience parity). Launch
fields are server-owned — the tool does not accept model/effort/instructions.

After start, Minos sends the clean prompt to the target thread, then writes a
visible source-agent message whose body is
`@target_agent#short_thread <prompt>` (with `delegation_id` + target mention
metadata) and records delegation state. A thread that was itself created by
delegation can only delegate back to the agent that delegated it.
`wait_delegation` blocks until the delegation reaches `completed` /
`cancelled` / `failed` or `timeout_ms` elapses (default 30000). It returns
durable status, optional `result_text`, and `source_delivery`
(`pending`/`delivered`/`failed`).

## Completion ownership (daemon)

Agent final-result writeback and delegation completion are owned by
`minos-daemon` (`conversation_completion`), not TUI. Writeback is **turn-boundary
latched**, not message-boundary:

- `ThreadState::Idle` / `Closed` sets `pending_boundary` and tries to record.
- Ingest `MessageCompleted` only **accumulates** for non-Opencode agents unless
  `pending_boundary` (or `SessionClosed`) is already set — so mid-turn completes
  never upsert `agent-result:…` or `complete_delegation`.
- Opencode may record on terminal `MessageCompleted` (finish:stop) without Idle,
  because its Idle can race ahead of projected text.
- **Single-flight claim**: `try_record` sets `write_in_flight` under the projection
  mutex *before* the async SQLite write. Concurrent Idle + MessageCompleted
  (especially Opencode, which can take both paths) cannot insert two
  `agent-result:…` rows. The durable id is turn-scoped (`turn_write_id`) so a
  retry after a failed write upserts the same row.
- `ThreadState::Running` / user `MessageStarted` resets turn-scoped projection
  fields (`completed`, `last_error`, `turn_recorded`, `write_in_flight`,
  `pending_boundary`, `turn_write_id`, …)
  so a cancelled/failed turn cannot resurrect the previous turn's text.

On successful record for a top-level conversation thread the daemon:

1. Upserts `agent-result:{conversation}:{thread}:{key}` into conversation
   messages (delegation results use `@source#short <result>` body + metadata).
   Body text is the **last open assistant segment** after tools/reasoning
   (aligned with session `ChatState`), not a concatenation of mid-turn
   progress `agent_message_chunk`s.
2. Marks the matching teamwork delegation completed.
3. Delivers `[target#short] @source#short <result>` to the source thread using
   the busy-delivery policy (Codex steers while running; Gemini/Grok queue until
   Idle).

TUI only subscribes to conversation events for display.

`TeamworkStore` schema is latest-only（与 daemon 本地库分离，属 minos-chat-store）。
开发态若列形状变更，清库/重建即可；不维护对旧 `daemon.sqlite` 的 dual-read ALTER 链。

## Injection Paths

| Agent | Injection path |
| --- | --- |
| Codex | `codex app-server` receives `-c mcp_servers.minos_teamwork.command=...`, `-c mcp_servers.minos_teamwork.args=[...]`, and `-c mcp_servers.minos_teamwork.enabled=true`. Teamwork guidance is also set via `thread/start.developerInstructions`. |
| Claude | `claude -p` receives `--mcp-config <json>` with `mcpServers.minos_teamwork` and `--strict-mcp-config`, plus `--append-system-prompt` with teamwork guidance. |
| Gemini | ACP `session/new` and `session/resume` receive `mcpServers` containing `minos_teamwork` as a stdio server. |
| Grok | Spawned as `grok --rules <teamwork guidance> agent --no-leader stdio`. ACP `session/new` and `session/resume` receive `mcpServers` containing `minos_teamwork` (same stdio shape as Gemini). `--rules` is a top-level `grok` flag that appends to the system prompt. Grok `exit_plan_mode` parks on ACP `ext_method` → Minos approval overlay (`a`/`s`/`q`); `ask_user_question` still auto-cancels until a question UI lands. Grok ACP projection (`translate_grok`) parses `SessionNotification._meta` (`streamStartMs` / timestamps / `promptId`), closes assistant text on tool + **agent_message_chunk** `streamStartMs` boundaries (not thought — concurrent thought stream has a different `streamStartMs` and must not shatter text into per-token bubbles), suppresses plumbing tools (todo/wait/task-output), and prefers `rawInput.description` for tool titles — aligned with grok-build `AcpUpdateTracker`. |
| OpenCode | `opencode serve` receives `OPENCODE_CONFIG_CONTENT` containing a local enabled `mcp.minos_teamwork` server. |

The command may be the standalone `minos-teamwork-mcp` binary or the hidden
sidecar form:

```text
minos-tui __minos-teamwork-mcp --conversation-id ... --source-agent ... --source-thread-id ... --socket-path ...
minos-daemon __minos-teamwork-mcp --conversation-id ... --source-agent ... --source-thread-id ... --socket-path ...
```

## Skill Locations

The repo carries the Minos teamwork skill inside the TUI crate:

- `crates/minos-tui/skills/minos-teamwork/SKILL.md`

On TUI startup, Minos installs that embedded skill into global skill folders:

- `~/.agents/skills/minos-teamwork/SKILL.md`
- `~/.claude/skills/minos-teamwork/SKILL.md`
- `~/.gemini/skills/minos-teamwork/SKILL.md`
- `~/.config/opencode/skills/minos-teamwork/SKILL.md`

## Prompt Guidance

`minos-agent-runtime` also injects Minos teamwork guidance:

- Codex: `thread/start.developerInstructions`
- Claude: `--append-system-prompt`
- Gemini/OpenCode: skill plus MCP server instructions

Agents should use `minos_teamwork.list_conversation_messages` when conversation
history, teammate output, mentions, or coordination state may affect the answer.
They should use `delegate_to_agent` for focused requests to another agent and
track those requests with `get_delegation_status` or `cancel_delegation`. A
delegated session should only delegate back to its source agent, not to a third
agent.
`post_conversation_update` is only for concise updates that should appear in
the shared conversation.
