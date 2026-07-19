# Client Implementation Guide

## Minimal stdio TypeScript pattern

Use `assets/node-stdio-client.ts` as a starting point. The pattern is:

1. `spawn("codex", ["app-server"])`.
2. Read stdout line by line.
3. Write one JSON message plus `\n` per request/notification.
4. Correlate responses by `id`.
5. Dispatch notifications by `method`.
6. Keep listening until the process exits.

## State model

Maintain these maps:

- `pendingRequests: Map<number, {resolve, reject, method}>`
- `threads: Map<string, ThreadState>`
- `turns: Map<string, TurnState>` keyed by `turnId`
- `items: Map<string, ItemState>` keyed by `item.id`
- `approvalRequests: Map<string, ApprovalRequest>` keyed by request id or item id

Recommended states:

- Connection: `starting -> initialized -> closing -> closed`.
- Thread: `notLoaded | loaded | active | archived | closed` where server notifications are authoritative.
- Turn: `inProgress | completed | interrupted | failed`.
- Item: `inProgress | completed | failed | declined`.

## Thread and turn lifecycle

Start a new thread:

```json
{ "method": "thread/start", "id": 10, "params": { "model": "gpt-5.4", "cwd": "/Users/me/project", "approvalPolicy": "never", "sandbox": "workspaceWrite", "serviceName": "my_client" } }
```

Start a turn:

```json
{ "method": "turn/start", "id": 30, "params": { "threadId": "thr_123", "input": [{ "type": "text", "text": "Run tests" }], "cwd": "/Users/me/project", "approvalPolicy": "unlessTrusted", "sandboxPolicy": { "type": "workspaceWrite", "writableRoots": ["/Users/me/project"], "networkAccess": true }, "model": "gpt-5.4", "effort": "medium", "summary": "concise" } }
```

Input item types documented for turn input include:

- `{ "type": "text", "text": "..." }`
- `{ "type": "image", "url": "https://..." }`
- `{ "type": "localImage", "path": "/tmp/screenshot.png" }`

Use `outputSchema` only for the current turn when structured output is required.

## Streaming and items

The response to `turn/start` only confirms that the turn started. The actual work streams via notifications. Common notifications:

- `turn/started`
- `turn/completed`
- `turn/diff/updated`
- `item/started`
- `item/completed`
- `item/agentMessage/delta`
- `item/plan/delta`
- `item/reasoning/summaryTextDelta`
- `item/commandExecution/outputDelta`
- `item/fileChange/outputDelta`
- `serverRequest/resolved`

Treat `item/completed` as authoritative for final item data. Deltas are useful for live UI but may not exactly equal final item content for all item types.

## Approvals

App-server can send server-initiated requests to the client. The client must reply with a decision.

Command approval decisions:

- `accept`
- `acceptForSession`
- `decline`
- `cancel`
- `{ "acceptWithExecpolicyAmendment": { "execpolicy_amendment": ["cmd", "..."] } }`

File change approval decisions:

- `accept`
- `acceptForSession`
- `decline`
- `cancel`

Approval UX rules:

- Show command, cwd, reason, proposed policy amendment, and available decisions when present.
- When `networkApprovalContext` is present, render it as network access to host/protocol/port, not as a generic shell command.
- Scope prompts by `threadId` and `turnId`.
- Resolve or clear stale prompts when `serverRequest/resolved`, `turn/completed`, or `turn/interrupt` arrives.

## Error handling

Turn failures emit an `error` event and then `turn/completed` with `status: "failed"`. Common `codexErrorInfo` values include:

- `ContextWindowExceeded`
- `UsageLimitExceeded`
- `HttpConnectionFailed`
- `ResponseStreamConnectionFailed`
- `ResponseStreamDisconnected`
- `ResponseTooManyFailedAttempts`
- `BadRequest`
- `Unauthorized`
- `SandboxError`
- `InternalServerError`
- `Other`

For WebSocket overload code `-32001`, retry with exponential backoff and jitter. For stdio process exit, surface stderr and require explicit restart unless the product owns restart policy.

## Security defaults

- Do not expose unauthenticated non-loopback WebSocket listeners.
- Prefer token files or secret stores over raw tokens in command lines.
- Use restrictive `sandboxPolicy` and `approvalPolicy` unless the user asks otherwise.
- Do not call `thread/shellCommand` for hidden or automatic operations; it runs outside the sandbox with full access and should only be explicit user-initiated action.
- Use absolute paths for filesystem v2 calls and validate paths against product-controlled roots.
