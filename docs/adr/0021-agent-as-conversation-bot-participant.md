# 0021 · Agent as Conversation Bot Participant (Message-Driven)

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-09 |
| Deciders | fannnzhang |
| Supersedes | Product framing that treats `@agent` as a **command-orchestration primary path** parallel to IM; does **not** supersede dual Account/Host principals ([0020](0020-server-centric-auth-and-account-pairs.md)) |
| Normative detail | [architecture-messaging.md](../architecture-messaging.md) |
| Global bot identity | bot 是全局唯一数字身份；conversation 只持 membership；session 是 per-conversation 执行上下文 |
| Bot mailbox + WS IM bus | Account WS 写消息；Bot 逻辑邮箱；Host 共享执行连接 |

## Context

Minos is **Conversation-first collaboration IM** (Slack/企微-shaped): multi-end humans chat on a Hub timeline. Agents execute on a user Host (Cloud does not run CLIs).

The implementation already stores agent roster rows (`conversation_agent_members`), agent-authored bubbles (`sender_type=agent`), and an async `AgentDispatchQueue`. Product and docs, however, still describe `@agent` as a **special post-send command branch**:

```
message lands → if @agent → HostCommand start/send_input → projector invents bubble
```

while `@human` is a normal mention → unread/Push path. That splits the collaboration model:

1. Timeline *looks* like chat; activation *behaves* like RPC.
2. Human mentions are durable SSOT; agent @ is ephemeral text routing.
3. Desktop Online and “remote collab” narratives over-weight `/ws/host` commands instead of Hub messages.
4. Readers infer agents need Account login or that remote collab *is* host command delivery.

We need a single product decision: **agents are bot participants on the conversation bus**, not a second collaboration protocol.

## Decision

1. **Agent is a first-class conversation participant (bot), not a human Account.**
   - **Global bot identity**: one stable `agent_id` (数字肉身：name / model / reasoning / system prompt / runtime 等) is reused across conversations; joining a conversation is **membership only**, not creating a new bot. Per-conversation **sessions** are execution context, not identity. The current contract lives in [architecture-messaging.md](../architecture-messaging.md).
   - Appears in conversation roster / participants API.
   - Can be @-mentioned with the **same mention semantics** as humans (structured targets).
   - Can author timeline messages (`sender_type=agent`).
   - **Must not**: Supabase login, human access/refresh tokens, or share a human `account_id` as identity.

2. **Collaboration is message-driven only.**
   - Sole collaboration primitives: Conversation Message (+ reaction / read / recall / mention).
   - `@人` and `@agent` both: commit message → durable mentions → **participant delivery**.
   - Human delivery → devices (Account realtime + Push).
   - Agent delivery → **Agent inbox** → runtime consumer on bound Host → **reply as agent message**.
   - HostCommand / CLI invocation is a **private runtime adapter**, not the product collaboration model.

3. **Host remains the execution body, not the IM principal.**
   - `/ws/host` + `hit_*` stay for machine control, ingest, and runtime port.
   - `/ws/client` (Account) remains the human IM plane.
   - Dual principals from formal backend design are **unchanged**; this ADR only re-frames agent *activation* as delivery, not as “IM needs two chat sockets.”

4. **Online product semantics (Desktop).**
   - Primary “can chat” Online = **Account sync live** (send/receive on Hub).
   - Host / agent runtime readiness is secondary (“This Mac / agent available”).
   - Must not show full Online solely because `/ws/host` is up while Account auth is dead.

5. **Rejected alternatives**
   - **Login-as-agent / agent Account JWT** — permission blast radius; confuses audit.
   - **Command bus as collaboration primary** — permanently special-cases bots outside IM.
   - **Collapse Account + Host into one principal** — breaks multi-device remote execution isolation (see messaging architecture + 0020 lineage).
   - **Cloud runs CLI** — violates “Cloud does not run Agent.”

## Consequences

### Product / domain

- Composer @ picker lists **humans + bots** in one participant model.
- Unmatched `@codex` without membership is a delivery/membership error, not a silent second protocol (auto-attach policy becomes explicit and bounded).
- Multi-end continuity matches 企微: same Account writes Hub; agents reply onto the same timeline.

### Storage / API (directional; detail in normative spec)

- Mentions become polymorphic: `target_kind ∈ {account, agent}` (or equivalent).
- `agents` is the **global bot directory** (digital body SSOT on Hub); `conversation_agent_members` is membership only; `agent_sessions` is per-(conversation, agent) runtime context.
- Local daemon/Mobile “agent profiles” are **not** multi-end identity authority; they may cache Hub bots after sync.
- `AgentDispatchQueue` is the physical table for **Agent inbox** (rename semantic now; physical rename optional later).
- `message_source` remains **provenance + loop gate** (`host_projection`/`system` never re-deliver to agents), not “Desktop magic skip.”
- Wire author should stop abusing `UserSummary.account_id = agent_id` long-term (`SenderRef`-style); transitional clients may still branch on `sender_type`.

### Docs

- [architecture-messaging.md](../architecture-messaging.md) is SSOT for the message-driven participant model.
- Hub bubble write ownership **stays**; only the *trigger* wording moves from “dispatch special case” to “participant delivery.”
- Backend delivery orchestration renames AgentDispatch **semantics** to Agent inbox / participant delivery.

### Non-consequences

- Does **not** delete Host gateway or host installation tokens.
- Does **not** require a single physical WebSocket for Desktop dual role.
- Does **not** make agents project/account admins or friend-graph users.

## Alternatives considered

- **Keep command-special-case forever; only improve docs.** Rejected: docs and code would keep diverging; `@agent` remains second-class.
- **Full polymorphic `conversation_participants` table in one migration.** Deferred: dual tables + unified API (Phase A) is enough; merge tables only if query pain warrants.
- **Agent = human Account with a bot flag.** Rejected: login, refresh, and ACL surface are wrong for runtime-bound workers.

## References

- Current messaging, bot identity, mailbox, bubble ownership, and delivery contract: [architecture-messaging.md](../architecture-messaging.md)
- Auth principals: [0020](0020-server-centric-auth-and-account-pairs.md)
