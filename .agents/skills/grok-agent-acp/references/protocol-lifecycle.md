# Grok ACP Protocol Lifecycle

## Connection bootstrap

```text
Client                              Grok agent
  |                                      |
  |---- initialize(id=1) --------------->|
  |<--- result{protocolVersion,...} -----|
  |                                      |
  |---- session/new(id=2) -------------->|
  |<--- result{sessionId} ---------------|
  |                                      |
  |---- session/prompt(id=3) ----------->|
  |<--- session/update (notif)* ---------|  (message/thought/tool chunks)
  |<--- session/request_permission(id=s) |  (optional server request)
  |---- result outcome ----------------->|
  |<--- session/update ... --------------|
  |<--- result{stopReason} (id=3) -------|
```

`*` Notifications may arrive **before** the matching response. Clients must multiplex by `id` for responses and by `method` for notifications.

## stopReason values (typical)

| Value | Meaning |
|-------|---------|
| `end_turn` | Normal completion |
| `cancelled` | User/client cancelled |
| `max_tokens` / `max_turn_requests` / `refusal` | Terminal conditions |

Minos maps `end_turn` → complete open assistant message; `cancelled` → complete + `ThreadClosed(UserStopped)`.

## Permission outcome

Respond to `session/request_permission` with:

```json
{
  "jsonrpc": "2.0",
  "id": "<server-request-id>",
  "result": {
    "outcome": { "outcome": "selected", "optionId": "<option>" }
  }
}
```

or cancel:

```json
{
  "jsonrpc": "2.0",
  "id": "<server-request-id>",
  "result": {
    "outcome": { "outcome": "cancelled" }
  }
}
```

Minos Grok driver registers `session/request_permission` on the shared approval pipeline (`PendingApprovalTarget::Acp`) and **waits for the user** (TUI overlay → `resolve_approval`). It does **not** auto-timeout approvals; the pending entry stays until the user decides or the agent process dies. Product-grade clients should wire selected outcomes to user input rather than auto-replying.

## Plan-mode reverse-requests (`_x.ai/exit_plan_mode`)

`exit_plan_mode` (after its read-only permission passes) triggers a **blocking ACP extension reverse-request**. ACP serializes extension methods with a leading underscore and flat params (no nested envelope):

```json
{
  "jsonrpc": "2.0",
  "id": "<id>",
  "method": "_x.ai/exit_plan_mode",
  "params": { "sessionId": "…", "toolCallId": "…", "planContent": "# Plan …" }
}
```

Reply with the flat `ExtResponse` body:

```json
{ "jsonrpc": "2.0", "id": "<id>", "result": { "outcome": "approved" } }
```

`outcome` ∈ `approved` (exit + implement) / `cancelled` + optional `feedback` (revise plan, stay in plan mode) / `abandoned` (close plan mode). Malformed replies or `-32601` fail **closed** (shell treats as mid-approval disconnect → turn cancelled, plan mode stays active, UI hangs on `Running {exit_plan_mode}`). See `SKILL.md` "Plan mode reverse-requests".

## Content blocks in prompt

Minimal text prompt:

```json
{
  "jsonrpc": "2.0",
  "id": "3",
  "method": "session/prompt",
  "params": {
    "sessionId": "…",
    "prompt": [{ "type": "text", "text": "hello" }]
  }
}
```

Images/resources use other `ContentBlock` variants from ACP (`image`, `resource`, …).
