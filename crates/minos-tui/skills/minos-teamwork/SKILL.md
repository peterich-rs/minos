---
name: minos-teamwork
description: Use when working inside Minos TUI/teamwork chat rooms, coordinating with the user or other CLI agents, reading current room context, using the minos_teamwork MCP server, or deciding whether to post room-visible updates.
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

Use the `minos_teamwork` MCP server when chat room context could affect the answer.
Do not assume the prompt contains the current room state if MCP is available.

- Call `list_room_messages` before answering when the user refers to the room,
  "current chat", prior teammate output, mentions, coordination status, or
  anything another agent may have said.
- Call `delegate_to_agent` when another Minos agent is better positioned to
  provide focused help. Keep the prompt specific and include the exact context
  needed. Save the returned `delegation_id`; use `get_delegation_status` to
  check it and `cancel_delegation` only when the delegated work is no longer
  needed.
- Call `ask_user_question` for concise clarification that must be visible in
  the room. It is non-blocking; use `check_user_feedback` with the returned
  `feedback_id` before relying on the answer.
- Call `post_room_update` only for concise room-visible updates that should appear
  in the shared chat. Do not duplicate routine final answers into the room
  unless the user or workflow needs a visible status update.
- Call `react_to_message` to acknowledge a specific room message with an emoji
  instead of posting a new message when a lightweight reaction is enough.

## Working style

- Prefer concrete room facts from `list_room_messages` over memory or guesses.
- Treat delegation and feedback ids as workflow state; include them when
  checking or cancelling existing work.
- If MCP is missing, say that room state is unavailable through MCP and proceed
  from visible context.
- Keep inter-agent requests short and actionable.
- Preserve normal CLI-agent responsibilities: inspect the repository, make
  scoped edits, run relevant verification, and report what changed.
