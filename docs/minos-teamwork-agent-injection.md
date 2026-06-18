# Minos Teamwork Agent Injection

This note records how Minos exposes the current TUI chat room to CLI agents and
which instruction surfaces are available for Codex, Claude, Gemini, and
OpenCode.

## Diagnosis

`/tmp/minos-tui.log` only showed TUI logging initialization:

```text
minos-tui logging initialized path=/tmp/minos-tui.log
```

That log line does not prove MCP startup failure. In the repo, MCP is already
wired in several places:

- `crates/minos-agent-runtime/src/config.rs` locates the teamwork MCP command
  with `MINOS_TEAMWORK_MCP_BIN`, then the current executable's sibling
  `minos-teamwork-mcp`, then the current `minos-tui` / `minos-daemon`
  executable as a hidden `__minos-teamwork-mcp` sidecar, then `PATH`.
- `crates/minos-chat-store/src/mcp_server.rs` implements `minos-teamwork-mcp`
  with `list_room_messages`, `delegate_to_agent`, and `post_room_update`.
- `crates/minos-daemon/src/agent.rs` enables the default `minos-teamwork-mcp`
  configuration for daemon-managed agents.
- `crates/minos-agent-runtime/src/manager.rs` binds the MCP server to
  `room_id_for_workspace(workspace)` with `--room-id` and passes
  `--source-agent` so room commands are attributed to Codex, Claude, Gemini,
  or OpenCode.

The likely gap was discoverability, not just transport injection: an agent can
have the MCP server available and still fail to inspect the room if no skill,
system prompt, or MCP server instructions tell it when the Minos room matters.

## MCP Injection

All Minos MCP injections name the server `minos_teamwork`.

| Agent | Current injection path |
| --- | --- |
| Codex | `codex app-server` receives `-c mcp_servers.minos_teamwork.command=...`, `-c mcp_servers.minos_teamwork.args=[...]`, and `-c mcp_servers.minos_teamwork.enabled=true`. |
| Claude | `claude -p` receives `--mcp-config <json>` with `mcpServers.minos_teamwork` and `--strict-mcp-config`. |
| Gemini | ACP `session/new` and `session/resume` receive `mcpServers` containing `minos_teamwork` as a stdio server. |
| OpenCode | `opencode serve` receives `OPENCODE_CONFIG_CONTENT` containing a local enabled `mcp.minos_teamwork` server, unless the caller already supplied that environment variable. |

The command may be the standalone `minos-teamwork-mcp` binary or the
self-contained hidden sidecar form:

```text
minos-tui __minos-teamwork-mcp --room-id ... --source-agent ... --socket-path ...
minos-daemon __minos-teamwork-mcp --room-id ... --source-agent ... --socket-path ...
```

This keeps TUI development and packaged TUI usage self-contained. A missing
standalone sidecar no longer silently disables MCP injection when the current
binary can serve the MCP protocol itself. `minos_agent_runtime::config` logs the
resolved command, args, and socket path when injection is enabled.

The MCP server now also returns an `instructions` field during `initialize`.
Codex documents that it reads MCP server instructions and uses them as
server-wide guidance alongside the server tools. Other clients may expose this
less consistently, so Minos also uses skills and prompt injection where
available.

## Skill Locations

The repo carries the Minos teamwork skill inside the TUI crate:

- `crates/minos-tui/skills/minos-teamwork/SKILL.md`

On TUI startup, Minos installs that embedded skill into global skill folders:

- `~/.agents/skills/minos-teamwork/SKILL.md`
- `~/.claude/skills/minos-teamwork/SKILL.md`
- `~/.gemini/skills/minos-teamwork/SKILL.md`
- `~/.config/opencode/skills/minos-teamwork/SKILL.md`

The `.agents/skills` copy is the interoperable location. It is documented by
Codex, Gemini CLI, and OpenCode:

- Codex scans repo `.agents/skills`, user `~/.agents/skills`, admin
  `/etc/codex/skills`, and built-in system skills.
- Gemini CLI scans workspace `.gemini/skills` or `.agents/skills`, and user
  `~/.gemini/skills` or `~/.agents/skills`; within a tier, `.agents/skills`
  takes precedence over `.gemini/skills`.
- OpenCode scans `.opencode/skills`, `.claude/skills`, `.agents/skills`, and
  their global equivalents.

Claude Code documents project skills under `.claude/skills/<name>/SKILL.md`
and personal skills under `~/.claude/skills/<name>/SKILL.md`. It follows the
same open Agent Skills standard, but `.agents/skills` is not its documented
project discovery path, so Minos carries a Claude-native copy too.

## System Prompt Injection

System/developer prompt support differs by CLI:

| Agent | Minos support |
| --- | --- |
| Codex | `thread/start` now sends `developerInstructions` with Minos teamwork background. Codex MCP config is also injected at app-server spawn time. |
| Claude | `claude -p` now appends Minos teamwork background with `--append-system-prompt`. The installed CLI also supports `--mcp-config` and `--strict-mcp-config`. |
| Gemini | The current ACP structs expose `mcpServers` for `session/new` and `session/resume`, but no stable separate system prompt field. Use the skill plus MCP server instructions. |
| OpenCode | Local `opencode serve` and `opencode run` help do not expose a direct append-system-prompt flag. Use the skill plus MCP config; keep `OPENCODE_CONFIG_CONTENT` for MCP injection. |

## Operational Guidance

Agents working in Minos should:

- Use `minos_teamwork.list_room_messages` when room history, teammate output,
  mentions, current room state, or cross-agent coordination may affect the
  answer.
- Use `minos_teamwork.delegate_to_agent` for focused requests to another Minos
  agent in the same room; use `get_delegation_status` or `cancel_delegation`
  with the returned delegation id when tracking or stopping that work.
- Use `minos_teamwork.ask_user_question` for non-blocking clarification and
  `minos_teamwork.check_user_feedback` with the returned feedback id before
  relying on an answer.
- Use `minos_teamwork.post_room_update` only for concise user-visible updates that need
  to appear in the shared room.
- Use `minos_teamwork.react_to_message` for lightweight emoji acknowledgement
  on a specific room message.
- Avoid treating the direct prompt as a complete snapshot of the chat room when
  MCP is available.

Sources checked on June 8, 2026: current Codex manual sections for Agent Skills,
MCP, and app-server; Gemini CLI skills documentation from the official
`google-gemini/gemini-cli` repository; OpenCode skills documentation from
`anomalyco/opencode`; Claude Code skills and MCP Markdown docs; local
`claude --help`, `gemini --help`, `opencode --help`, `opencode serve --help`,
and `opencode run --help`.
