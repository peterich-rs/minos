# Realtime Surface Audit (R0)

| Field | Value |
|-------|--------|
| Date | 2026-08-03 |
| Spec | [2026-08-03-realtime-surface-model.md](../2026-08-03-realtime-surface-model.md) |
| Status | **Audit + R1/R2/R3/R4 implement** (Layer R complete) |

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
| `ConversationMessageAppended` | ✅ social delivery (full) | ✅ SocialEvent `message` | ✅ onChatMessage | **T1** conversation | existing |
| `ConversationMessageRecalled` | ✅ social delivery (full) | ✅ | ✅ | T1 | existing |
| `ConversationMessageReactionUpdated` | ✅ reaction delivery | ✅ | ✅ | T1 conversation-only | existing (B6) |
| `AccountConversationMessageAppended` | ✅ social delivery (**thin digest**) | ✅ `inbox_digest` → rail only | ✅ `onAccountInboxDigest` | **T2 digest** | **R3 DONE** |
| `AccountConversationMessageRecalled` | ✅ thin digest | ✅ `inbox_recall` | ✅ digest recall | T2 digest | **R3 DONE** |
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
| send/recall/reaction social | ✅ conversation full + account thin / reaction | live | OK (IM Reliability + R3) |
| project archive / link | ❌ | yes if multi-end | residual T3 |
| agent config update | ❌ | T3/T4 ok if product accepts | residual |

---

## 3. R3 Account thin digest — **DONE**

**Problem (was):** `AccountConversationMessageAppended.message: ChatMessageSummary` carried full body + reactions on the always-on account topic.

**Shipped shape (latest-only breaking):**

```text
AccountConversationMessageAppended {
  account_id, conversation_id, message_id,
  sender, at_ms,
  preview: String,              // truncated (~120 chars)
  sender_display_name: String,
  mentioned: bool,              // viewer-relative
  message_seq: Option<i64>,
}
AccountConversationMessageRecalled {
  account_id, conversation_id, message_id, at_ms,
  preview: Option<String>,      // e.g. "Message recalled"
  message_seq: Option<i64>,
}
// Full ChatMessageSummary ONLY on ConversationMessageAppended / Recalled
// (conversation topic).
```

**Shipped sequence (R3a–R3d together):**

| Step | Status | Notes |
|------|--------|-------|
| **R3a** Mobile conversation subscribe | ✅ | FRB `subscribe_conversation` / `unsubscribe_conversation`; `SocialConversation` open → subscribe, dispose → unsubscribe |
| **R3b** Wire thin account | ✅ | `minos-protocol` DurableEvent; no dual payload |
| **R3c** Clients | ✅ | Desktop: `onAccountInboxDigest` rail-only; Mobile: `inbox_digest`/`inbox_recall` for inbox; open chat uses T1 full only |
| **R3d** Backend delivery + push | ✅ | `social/delivery.rs` builds digest; push uses `preview` |

**Evidence:** `store::social::delivery::tests::account_fanout_is_thin_digest_conversation_is_full`; protocol round-trip; Mobile parse tests; Desktop tests green.

---

## 4. R4 Subscription hygiene

| Item | Status |
|------|--------|
| Desktop conversation LRU (`MAX_OPEN_CONVERSATION_SUBSCRIPTIONS = 16`) | ✅ `conversation-sub-lru.ts` + `HubRealtimeSession.subscribeConversation` |
| Mobile conversation subscribe/unsubscribe (R3a) | ✅ FRB + SocialConversation lifecycle |
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

Clients: account topic arms Host* / Friend* / **AccountConversation* digests**; conversation topic arms message/reaction **full**.
