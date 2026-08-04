---
name: minos-teamwork
description: Use when working inside Minos TUI/teamwork conversations, coordinating with the user or other CLI agents, reading current conversation context, using the minos_teamwork MCP server, or deciding whether to post conversation-visible updates.
---

# Minos Teamwork

You are running as a CLI agent inside Minos teamwork mode. Minos turns Codex,
Claude, Gemini, OpenCode, and other CLI agents into teammates in one shared
conversation with the user.

Treat the direct agent session and the Minos conversation as different surfaces:

- The direct session is where you do reasoning, code edits, commands, and final
  answers.
- The Minos conversation is shared coordination state with the user and other agents.
  It may contain mentions, teammate results, user instructions, and current
  conversation context that are not visible in the direct prompt.

## MCP usage

Use the `minos_teamwork` MCP server when conversation context could affect the answer.
Do not assume the prompt contains the current conversation state if MCP is available.

- Call `list_conversation_messages` before answering when the user refers to the conversation,
  "current chat", prior teammate output, mentions, coordination status, or
  anything another agent may have said. Message objects may include structured
  `delegation_id`, `reply_to_message_id`, and `mentions` fields in addition to
  plain `body` text. System rows may start with `[minos:system]` (roster updates);
  treat them as host coordination, not user chat.
- Call `list_conversation_roster` when you need the live teammate directory (who
  is on the conversation, and each member's role brief). Prefer this over memory
  or the session-start roster snapshot after membership may have changed.
  Match `delegate_to_agent` targets to the teammate whose brief fits the work.
- Host may inject a direct-session notice prefixed with `[minos:host] kind=roster_changed`
  when the roster changes while you are idle. That is **not** a user request: do
  not treat it as a new task unless your plan depended on a removed teammate.
  Conversation timeline uses `[minos:system]` for the same event (visible to humans).
- Call `delegate_to_agent` when another Minos agent is better positioned to
  provide focused help. Keep the prompt specific and include the exact context
  needed. Save the returned `delegation_id`.
- Call `wait_delegation` when the **next critical-path step is blocked** on the
  delegated result. It blocks until the delegation is `completed`, `cancelled`,
  or `failed`, or until `timeout_ms` elapses (default 30000). Use sparingly:
  while the target works, prefer non-overlapping local work. You may re-call
  `wait_delegation` after a timeout.
- Call `get_delegation_status` for a non-blocking status peek; it does not wait.
- Call `cancel_delegation` only when the delegated work is no longer needed.
- If your current session was delegated from another agent, only delegate
  back to that source agent; do not fan work out to a third agent.
- Call `post_conversation_update` only for concise conversation-visible updates that should appear
  in the shared conversation. Do not duplicate routine final answers into the conversation
  unless the user or workflow needs a visible status update.
- Call `react_to_message` for lightweight acknowledgements (👍 ✅ 👀 ❤️) on a user
  message that **@mentioned you**. Prefer a reaction over a full reply when the
  user only needs a quick ack. The host **hard-rejects** reactions on messages
  that did not @mention this agent — do not try to react to teammate chatter or
  unrelated history. Toggle is idempotent (same emoji again removes it).
- Call `post_git_update` for structured git milestones instead of free-form text:
  `worktree_created`, `commits_made`, `pr_opened`, `ready_for_review`,
  `checks_failed`, `merged`. Prefer this whenever you finish a delivery step
  that involves commits or a pull request.

## Multi-agent @mentions

A single user message may @mention several agents (e.g. `@codex @claude count 1 2`).
Each mentioned agent is woken in parallel with the same origin message. Coordinate
via `list_conversation_messages` when order matters; your final answer is projected
into the conversation automatically — use `react_to_message` only for acks, not
as a substitute for a required answer.

## Git work units

Conversations may be bound to an isolated git worktree (`git_mode=worktree`).
When your session cwd is already a linked worktree, keep all file changes there;
do not checkout or edit the project's default branch for feature work.

- Before committing, ensure `git config user.name` / `user.email` are set.
- After meaningful commits, call `post_git_update` with `kind=commits_made`.
- When a PR is opened (or ready for review / merged / checks failed), post the
  matching `post_git_update` kind so the conversation timeline stays the SSOT.

## Result delivery

When a delegated agent finishes:

1. Minos writes one conversation message from the target agent, typically
   addressed as `@source_agent#short_thread <result>`, with `delegation_id` and
   optional `reply_to_message_id` metadata.
2. Minos also delivers the result into the **source agent direct session** as a
   user-visible input of the form
   `[target_agent#short_target] @source_agent#short_source <result>`.
3. Source-busy delivery policy:
   - Idle / resumable: deliver immediately.
   - Codex while running: **steer** into the active turn.
   - Claude / OpenCode while running: send as another prompt.
   - Gemini / Grok while running (or Starting/Resuming): **queue** until Idle;
     `wait_delegation` may report `source_delivery: pending` even after
     `status: completed`.
   - Closed / missing source: conversation result still completes;
     `source_delivery` may be `failed`.

`wait_delegation` joins durable delegation status. Final content is available via
`result_text` on completion and/or via the pushed source-session message. Prefer
the pushed message for continuing work; use `wait_delegation` when you must
block before the next step.

## Working style

- Prefer concrete conversation facts from `list_conversation_messages` over memory or guesses.
- Treat delegation ids as workflow state; include them when checking, waiting, or cancelling.
- If MCP is missing, say that conversation state is unavailable through MCP and proceed
  from visible context.
- Keep inter-agent requests short and actionable.
- Preserve normal CLI-agent responsibilities: inspect the repository, make
  scoped edits, run relevant verification, and report what changed.
