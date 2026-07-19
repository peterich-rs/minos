# Method Catalog for JSON v2 App Server Clients

Use this catalog to decide which method family to implement. Verify exact fields against generated schema for the installed Codex version.

## Core lifecycle

- `initialize`: first request on every connection.
- `initialized`: notification immediately after successful initialize request is sent.
- `model/list`: list picker-visible models and capabilities; use before rendering model selectors.

## Threads

- `thread/start`: create a new conversation and subscribe to events.
- `thread/resume`: reopen an existing thread id.
- `thread/fork`: branch an existing thread into a new thread id.
- `thread/read`: read stored thread without subscribing; `includeTurns` can return full history.
- `thread/list`: page stored thread logs with filters.
- `thread/turns/list`: page a stored thread's turn history.
- `thread/loaded/list`: list in-memory loaded thread ids.
- `thread/name/set`: set/update user-facing thread name.
- `thread/archive` / `thread/unarchive`: manage persisted archived state.
- `thread/unsubscribe`: unsubscribe current connection from thread events.
- `thread/compact/start`: trigger history compaction and stream progress.
- `thread/rollback`: remove recent turns from in-memory context and persist rollback marker.
- `thread/inject_items`: append Responses API items to model-visible history without starting a user turn.
- `thread/shellCommand`: explicit user-initiated shell command outside sandbox; use with caution.

## Turns

- `turn/start`: begin agent work for a thread; response returns initial turn and notifications stream progress.
- `turn/steer`: append input to an active turn; no turn-level overrides accepted.
- `turn/interrupt`: cancel active turn.
- `review/start`: start Codex reviewer for a thread.

## Events and item notifications

- `turn/started`, `turn/completed`, `turn/diff/updated`.
- `item/started`, `item/completed`.
- `item/agentMessage/delta`, `item/plan/delta`, `item/reasoning/summaryTextDelta`, `item/reasoning/summaryPartAdded`, `item/commandExecution/outputDelta`, `item/fileChange/outputDelta`.
- `serverRequest/resolved`.

## Command execution

- `command/exec`: run a single sandboxed command without starting a thread/turn.
- `command/exec/write`: write stdin bytes or close stdin.
- `command/exec/resize`: resize PTY session.
- `command/exec/terminate`: stop session.
- `command/exec/outputDelta`: notification for base64 stdout/stderr chunks.

## Filesystem v2

All operate on absolute paths:

- `fs/readFile`
- `fs/writeFile`
- `fs/createDirectory`
- `fs/getMetadata`
- `fs/readDirectory`
- `fs/remove`
- `fs/copy`
- `fs/watch`
- `fs/unwatch`
- `fs/changed` notification

Use `fs/watch` to invalidate UI state after file/directory changes.

## Skills

- Invoke a skill by including `$<skill-name>` in text input.
- Recommended: add a `skill` input item with `name` and `path` to inject full instructions and reduce latency.
- `skills/list`: fetch available skills scoped by `cwds`, with optional `forceReload` and `perCwdExtraUserRoots`.
- `skills/changed`: invalidate cached skill lists.
- `skills/config/write`: enable/disable a skill by path.

Example input:

```json
{
  "method": "turn/start",
  "id": 101,
  "params": {
    "threadId": "thread-1",
    "input": [
      { "type": "text", "text": "$skill-creator Add a new skill for triaging flaky CI." },
      { "type": "skill", "name": "skill-creator", "path": "/Users/me/.codex/skills/skill-creator/SKILL.md" }
    ]
  }
}
```

## Apps/connectors

- `app/list`: list available apps with accessibility and enabled status.
- Invoke an app by inserting `$<app-slug>` in text and adding a `mention` input item with `app://<id>` path.
- `config/read`, `config/value/write`, `config/batchWrite`: inspect/update app controls in `config.toml`.

## Experimental

Methods/fields marked experimental require `initialize.params.capabilities.experimentalApi = true`. Examples include thread goal methods, dynamic tools, some model provider capability calls, and selected feature controls. Do not enable globally unless the feature requires it.
