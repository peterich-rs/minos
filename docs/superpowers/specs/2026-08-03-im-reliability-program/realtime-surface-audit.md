# Realtime Surface Audit (R0)

| Field | Value |
|-------|--------|
| Date | 2026-08-03 |
| Spec | [2026-08-03-realtime-surface-model.md](../2026-08-03-realtime-surface-model.md) |
| Status | **Audit + R1/R2/R4 implement** (R3 design BLOCKED — see §3) |

---

## 1. DurableEvent variants

| Event | Emit site? | Client arm (Mobile) | Client arm (Desktop) | Recommended Tier | Track |
|-------|------------|---------------------|----------------------|------------------|-------|
| `AccountRegistered` | ⚠️ auth path partial / not critical multi-end | unhandled ok | ignore | T4 / T3 | residual |
| `AccountPasswordChanged` | ⚠️ security path | unhandled | ignore | T2 force re-auth | residual |
| `HostLinked` | ✅ `host_link::link_host` same tx + outbox | ✅ `host_linked` → pairedMacs upsert | ✅ `onHostLinked` optional | **T2** account | **R1 DONE** |
| `HostUnlinked` | ✅ `host_link::unlink_host` same tx + outbox | ✅ remove | ✅ `onHostUnlinked` optional | **T2** account | **R1 DONE** |
| `FriendRequestUpdated` | ✅ friends create/resolve + outbox | ✅ refresh friend lists | advance cursor | **T2** account | **R2 DONE** |
| `AgentSessionStarted` | ✅ agent_sessions | partial (session path) | partial | T1 session | existing |
| `AgentSessionEnded` | ✅ lifecycle / completion | partial | partial | T1/T2 | existing |
| `AgentTurnAppended` | ✅ send_input | stream/ui path | — | T1 session | existing |
| `ApprovalRequested` | ✅ approvals | ✅ Raw UI | daemon/hub | T1 + Push | existing |
| `ApprovalResolved` | ✅ approvals | ✅ Raw UI | — | T1 | existing |
| `ConversationMessageAppended` | ✅ social delivery | ✅ SocialEvent message | ✅ onChatMessage | **T1** conversation | existing |
| `ConversationMessageRecalled` | ✅ social delivery | ✅ | ✅ | T1 | existing |
| `ConversationMessageReactionUpdated` | ✅ reaction delivery | ✅ | ✅ | T1 conversation-only | existing (B6) |
| `AccountConversationMessageAppended` | ✅ social delivery (**full body**) | ✅ same as message | ✅ same | **T2 digest** target | **R3 BLOCKED** |
| `AccountConversationMessageRecalled` | ✅ full body | ✅ | ✅ | T2 digest target | **R3 BLOCKED** |
| `ProjectConversationLinked` | ❌ dead enum | ❌ | ❌ | T2/T3 | residual |
| `ProjectArchived` | ❌ dead enum | ❌ | ❌ | T3/T4 | residual |
| `HostForceClose` | ✅ host security | force close path | host | T1 host | existing |
| `HostCommandIssued` | ✅ host_commands | host daemon | daemon | T1 host | existing |

---

## 2. Multi-end HTTP writes

| API | Durable? | Client depends on refresh? | Verdict |
|-----|----------|----------------------------|---------|
| `POST /v1/hosts/link` | ✅ HostLinked + wake_outbox | No (R1 arm upsert) | **OK** |
| `POST /v1/hosts/unlink` | ✅ HostUnlinked + wake | No (R1 remove) | **OK** |
| `GET /v1/hosts` | n/a read | cold hydrate | OK |
| `POST /v1/friends/requests` | ✅ FriendRequestUpdated | No (refresh on event) | **OK** |
| accept/reject friend request | ✅ FriendRequestUpdated | No | **OK** |
| send/recall/reaction social | ✅ conversation + account / reaction | live | OK (IM Reliability) |
| project archive / link | ❌ | yes if multi-end | residual T3 |
| agent config update | ❌ | T3/T4 ok if product accepts | residual |

---

## 3. R3 Account thin digest — design (BLOCKED for code)

**Problem:** `AccountConversationMessageAppended.message: ChatMessageSummary` carries full body + reactions on the always-on account topic.

**Target shape (latest-only breaking):**

```text
AccountConversationMessageAppended {
  account_id, conversation_id, message_id,
  sender, at_ms,
  preview: String,           // truncated
  sender_display_name: String,
  mentioned: bool,
  message_seq: Option<i64>,
}
// Full ChatMessageSummary ONLY on ConversationMessageAppended (open conversation topic).
```

**Prerequisites before shipping:**

1. Mobile must **subscribe `conversation:{id}` while chat open** (FRB `subscribe_conversation` / unsubscribe + Dart arm) — today Mobile relies on account frames for open-chat live.
2. Desktop already conversation-subscribes focused chat — can patch inbox from account digest only.
3. Push `notifications::decision` must read `preview` instead of `message.text`.
4. Delete dual full-body account emission in `social/delivery.rs` same PR.

**Status:** **BLOCKED** on Mobile conversation subscription FRB surface. Documented; not half-thinned with dual payload.

---

## 4. R4 Subscription hygiene

| Item | Status |
|------|--------|
| Desktop conversation LRU (`MAX_OPEN_CONVERSATION_SUBSCRIPTIONS = 16`) | ✅ `conversation-sub-lru.ts` + `HubRealtimeSession.subscribeConversation` |
| Mobile `SubscriptionLimitExceeded` surface | ✅ UiEvent raw `subscription_limit_exceeded` (not silent drop) |
| Hint coalesce | residual (P/R later) |

---

## 5. Emit template (frozen)

```
BEGIN
  business write
  durable_event_log.record_in_tx (deterministic event_id)
  outbox_events.enqueue_in_tx (social_durable)
COMMIT
wake_outbox()
```

Clients: account topic arms Host* / Friend* / AccountConversation*; conversation topic arms message/reaction full.
