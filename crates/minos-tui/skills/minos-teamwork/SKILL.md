---
name: minos-teamwork
description: Use when working inside Minos TUI/teamwork chat rooms, coordinating with the user or other CLI agents, reading current room context, using the minos_chat MCP server, or deciding whether to post room-visible updates.
---

# Minos Teamwork

You are running as a CLI agent inside Minos teamwork mode. Minos turns Codex,
Claude, Gemini, OpenCode, and other CLI agents into teammates in one shared chat
room with the user.

Treat the direct agent session and the Minos chat room as different surfaces:

- The direct session is where you do reasoning, code edits, commands, and final
  answers.
- The Minos room is shared coordination state with the user and other agents.
  It may contain mentions, teammate results, user instructions, and current
  room context that are not visible in the direct prompt.

## MCP usage

Use the `minos_chat` MCP server when chat room context could affect the answer.
Do not assume the prompt contains the current room state if MCP is available.

- Call `list_chat_messages` before answering when the user refers to the room,
  "current chat", prior teammate output, mentions, coordination status, or
  anything another agent may have said.
- Call `request_agent_help` when another Minos agent is better positioned to
  provide focused help. Keep the prompt specific and include the exact context
  needed.
- Call `mention_user` only for concise room-visible updates that should appear
  in the shared chat. Do not duplicate routine final answers into the room
  unless the user or workflow needs a visible status update.

## Working style

- Prefer concrete room facts from `list_chat_messages` over memory or guesses.
- If MCP is missing, say that room state is unavailable through MCP and proceed
  from visible context.
- Keep inter-agent requests short and actionable.
- Preserve normal CLI-agent responsibilities: inspect the repository, make
  scoped edits, run relevant verification, and report what changed.
