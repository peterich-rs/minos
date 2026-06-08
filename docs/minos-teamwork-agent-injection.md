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

- `crates/minos-tui/src/backend/embedded.rs` starts embedded agents with the
  current Minos binary plus the hidden `chat-mcp` subcommand.
- `crates/minos-chat-store/src/mcp.rs` implements `minos-chat-mcp` with
  `list_chat_messages`, `request_agent_help`, and `mention_user`.
- `crates/minos-daemon/src/agent.rs` enables the default `minos-chat-mcp`
  configuration for daemon-managed agents.
- `crates/minos-agent-runtime/src/manager.rs` binds the MCP server to
  `room_id_for_workspace(workspace)` and passes `--source-agent` so room
  commands are attributed to Codex, Claude, Gemini, or OpenCode.

The likely gap was discoverability, not just transport injection: an agent can
have the MCP server available and still fail to inspect the room if no skill,
system prompt, or MCP server instructions tell it when the Minos room matters.

## MCP Injection

All Minos MCP injections name the server `minos_chat`.

| Agent | Current injection path |
| --- | --- |
| Codex | `codex app-server` receives `-c mcp_servers.minos_chat.command=...`, `-c mcp_servers.minos_chat.args=[...]`, and `-c mcp_servers.minos_chat.enabled=true`. |
| Claude | `claude -p` receives `--mcp-config <json>` with `mcpServers.minos_chat` and `--strict-mcp-config`. |
| Gemini | ACP `session/new` and `session/resume` receive `mcpServers` containing `minos_chat` as a stdio server. |
| OpenCode | `opencode serve` receives `OPENCODE_CONFIG_CONTENT` containing a local enabled `mcp.minos_chat` server, unless the caller already supplied that environment variable. |

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

- Use `minos_chat.list_chat_messages` when room history, teammate output,
  mentions, current room state, or cross-agent coordination may affect the
  answer.
- Use `minos_chat.request_agent_help` for focused requests to another Minos
  agent in the same room.
- Use `minos_chat.mention_user` only for concise user-visible updates that need
  to appear in the shared room.
- Avoid treating the direct prompt as a complete snapshot of the chat room when
  MCP is available.

Sources checked on June 8, 2026: current Codex manual sections for Agent Skills,
MCP, and app-server; Gemini CLI skills documentation from the official
`google-gemini/gemini-cli` repository; OpenCode skills documentation from
`anomalyco/opencode`; Claude Code skills and MCP Markdown docs; local
`claude --help`, `gemini --help`, `opencode --help`, `opencode serve --help`,
and `opencode run --help`.
