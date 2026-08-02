# Review: Hub collaboration message SSOT Phase 0–5

| Field | Value |
|-------|--------|
| Date | 2026-08-02 |
| Branch | `feat/mobile-messages-ios-minos-ds` |
| Spec | [2026-08-02-hub-collaboration-message-ssot.md](../specs/2026-08-02-hub-collaboration-message-ssot.md) |
| Also | [architecture-messaging.md](../../architecture-messaging.md) §1.2 / §7.4 |
| Scope | Uncommitted Phase 0–5 Hub IM SSOT work + fixes applied during review |

---

## Verdict

**Phase 0–5 direction is correct and largely landed.** Multi-end chat bubbles treat Hub as SSOT; Desktop Linked path is Hub-first (Outbox + client_live); agent final bubbles go through Hub `TurnCompletionProjector` (not Desktop dual-write); Mobile activity ticker treats `idle` as non-runnable; Sync has conversation subscribe, cursors, SnapshotRequired, and Hub `before_ts_ms` loadOlder.

**Three real correctness bugs were fixed in this review** (see below). Remaining gaps are known product/API holes (reactions cloud, account-topic resume, Postgres schema drift) rather than dual-write regressions.

---

## Focus-area checklist

| Area | Status | Notes |
|------|--------|-------|
| **1. Agent bubble single writer** | ✅ + fixed | `TurnCompletionProjector` + group completion watcher; Desktop `syncAgentMessageToCloud` / full timeline project are no-ops; daemon `conversation_completion` documented local-only for Linked |
| **2. message_source** | ✅ | `MessageSource` in protocol; dispatch only when `allows_agent_dispatch()` (`client_live`); pure `client_message_id` does not skip dispatch; `agents/message` rejects `client_live` |
| **3. Mobile activity bar** | ✅ | `_isRunnableStatus`: `idle` / terminal / `pending` excluded; allow-list for running-like statuses; tests cover idle |
| **4. Desktop Hub-first** | ✅ + fixed | Linked timeline merge prefers Hub bubbles; `im-cloud-inbound` does not append daemon; Linked send POSTs Hub only; **retry now preserves `@runtime` dispatch text** |
| **5. Outbox** | ✅ | localStorage outbox + worker; toast on delayed/terminal failure; `throwOnTerminal` surfaces send failure on Linked path |
| **6. Sync** | ✅ (partial) | State machine + per-topic cursors + conversation Subscribe + SnapshotRequired cold rebuild + loadOlder `before_ts_ms`; **account auto-sub still replays from 0** (gap) |
| **7. Compile / tests** | ✅ | `cargo check -p minos-backend -p minos-protocol`; desktop `tsc --noEmit`; unit tests listed below |

---

## Issues fixed in this review

### F1. TurnCompletionProjector: Idle race → premature `DoneWithoutText`

**Problem:** Formal session status `idle` (and other terminal statuses) made `boundary=true` immediately. With no clean final text yet, probe returned `DoneWithoutText` and the watcher exited — even when the final `MessageCompleted` was still mid-ingest (same race daemon solves with `pending_boundary`).

**Fix:** In `crates/minos-backend/src/turn_completion.rs`:
- Ready still allowed when clean last segment exists at a turn boundary.
- `DoneWithoutText` requires `SessionClosed` **or** `(session_terminal && seq_stable)`.
- Soft idle alone no longer abandons before the quiet window.

**Tests:** `idle_without_seq_stable_stays_pending_when_no_text`, `idle_with_seq_stable_and_no_text_is_done`.

### F2. Group completion watcher: post failure exited forever

**Problem:** On `CompletionProbe::Ready`, watcher always `return`ed after `post_agent_social_message`, including on `Err`. Transient insert/fanout failure permanently dropped the multi-end agent bubble.

**Fix:** In `crates/minos-backend/src/http/v1/social.rs`:
- `return` only on successful post.
- On failure: warn, sleep idle poll interval, `continue` (idempotent `client_message_id` on retry).
- Bound overall by existing `should_stop_after_long_idle` (~5m without raw activity).

### F3. Desktop Linked retry overwrote Hub dispatch text

**Problem:** Linked bare send prefixes `@{runtime}` for Hub multi-agent dispatch, but optimistic UI keeps display body without `@`. Retry used `failed.body` and re-enqueued bare text, overwriting a pending outbox entry that may already have had the correct `@runtime` text → no agent dispatch on retry in multi-agent rooms.

**Fix:**
- Pure helper `apps/desktop/src/store/workspace/hub-dispatch-text.ts` (`hubDispatchText`).
- Linked send + retry both recompute dispatch text from roster + routing.
- Unit tests in `hub-dispatch-text.test.ts`.

---

## What already matched the spec (no change)

- **Agent dual-write removed:** `projectTimelineMessagesToCloud` only flushes user Outbox; agent sync is intentional no-op.
- **Hub fanout:** `fan_out_social_message` publishes both `conversation:{id}` and `account:{id}` durable events.
- **Desktop read path:** `mergeHubAndLocalTimeline` drops local user/agent chat bubbles in favor of Hub; keeps tool/git/approval local.
- **Outbox reliability UX:** toast + failed delivery on Linked first-attempt failure.
- **host-runtime identity:** `agents.source` + unique `(owner, runtime)` for `host_runtime` (SQLite schema + store).
- **Mobile ticker:** idle not runnable; hide after assistant `MessageCompleted`.

---

## Remaining gaps (not fixed)

| ID | Severity | Gap |
|----|----------|-----|
| R1 | ~~Medium~~ **Fixed Phase 6.0** | Postgres latest-only schema aligned: social `agents` + `source`/`host_runtime`, `chat_messages`, friends, `raw_events`/`sessions`. Wipe local PG DBs on upgrade. |
| R2 | Low–Med | **Account topic resume:** Gateway `auto_subscribe_default_topic` always replays account topic from seq `0`. Desktop persists account cursors when events arrive but never Subscribe(account) with `resume_after`. Reconnect may re-apply full account history (upsert is mostly idempotent; waste + SnapshotRequired risk on large accounts). |
| R3 | Product | **Reactions still local-only** (Phase 5.2 as documented). No cloud API; Desktop isolates in reaction-store — correct honesty, not multi-end. |
| R4 | Product | **messages/query `after_seq`:** Spec cold path mentions after_seq; Desktop loadOlder uses `before_ts_ms` only (spec Phase 4.4 notes after_seq still not online). |
| R5 | Low | **Watcher post retry vs 5m idle stop:** After Ready, post failures retry until raw-seq idle window elapses (~5m), then give up. Acceptable bound; no durable Hub-side outbox for projector posts. |
| R6 | Low | **Linked Hub send does not start local daemon session:** Correct for multi-end (Host command path). Local tool cards only appear after Host runs; offline Host ⇒ user bubble on Hub, no agent bubble until Host recovers (projector only watches live dispatch). |
| R7 | ~~Nit~~ **Fixed Phase 6.0** | Removed residual dual-write APIs (`daemon_append_conversation_message`, timeline project no-ops, hub→daemon append). Rebuild paths use `mergeHubAndLocalTimeline`. |

---

## Tests run

```text
cargo check -p minos-backend -p minos-protocol          # ok
cargo test -p minos-backend --lib turn_completion       # 11 passed
cargo test -p minos-backend --lib (social store + watcher unit)  # 13 passed
cargo test -p minos-protocol --lib messages / realtime  # ok (prior)

apps/desktop:
  npx tsc --noEmit                                      # ok
  node --experimental-strip-types --test \
    src/shared/lib/hub-*.test.ts \
    src/shared/lib/im-*.test.ts \
    src/store/workspace/hub-dispatch-text.test.ts       # 28 passed
```

Mobile `agent_activity_provider` idle tests exist in-tree; Flutter test runner not re-executed in this pass (logic reviewed + existing unit coverage).

---

## Files touched by review fixes

| File | Change |
|------|--------|
| `crates/minos-backend/src/turn_completion.rs` | Idle/`DoneWithoutText` seq_stable latch + tests |
| `crates/minos-backend/src/http/v1/social.rs` | Post-failure retry instead of exit |
| `apps/desktop/src/store/workspace/hub-dispatch-text.ts` | Pure dispatch text helper |
| `apps/desktop/src/store/workspace/hub-dispatch-text.test.ts` | Unit tests |
| `apps/desktop/src/store/workspace/use-cases.ts` | Linked send + retry use `hubDispatchText` |

---

## Recommendation

Safe to continue product validation on Mobile `@agent` → Hub agent bubble and Desktop Linked send/receive with the three fixes above. Before production Postgres cutover, schedule **R1** schema convergence. Optionally harden **R2** account resume in a small Phase 4 follow-up.
