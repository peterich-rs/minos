# IM Reliability — Layer V Evidence

| Field | Value |
|-------|--------|
| Date | 2026-08-03 |
| Agent | Verification (Layer V1→V4) |
| Spec | [closeout-and-backlog.md](closeout-and-backlog.md) Layer V |
| Rule | Honest evidence only; no checkbox without proof |

---

## Status summary

| Layer | Scope | Result |
|-------|--------|--------|
| **V1** | B7.1 scenarios + B7.2/G2 rg gates | **PASS** |
| **V2** | G3 unit suites | **PASS** — backend lib 270; Desktop IM 78; Mobile flutter 17 (via `PUB_HOSTED_URL=https://pub.flutter-io.cn`) |
| **V3** | C6.4 nine bars + C6.5 multi-end matrix | **PARTIAL** — more AUTOMATED rows; device matrix still **MANUAL_REQUIRED** (runbook below) |
| **V4** | README §3 DoD full check | **PARTIAL** — mobile outbox DoD checked; kill-process multi-end still open |

**Blockers for V4 full closeout**

1. Multi-device / multi-client **live** matrix (C6.5) — use §V3 runbook; not executed end-to-end in automation.
2. Device kill-process / offline reaction observation still **MANUAL_REQUIRED**.
3. Layer P residuals (Hub digest dual-track, Desktop Hub approval HTTP) remain deferred — not V code blockers for B1–B6/C1–C6.3.

### Layer R (Realtime Surface) — this delivery

| Track | Result | Evidence |
|-------|--------|----------|
| **R0** | **PASS** | [realtime-surface-audit.md](realtime-surface-audit.md) |
| **R1** | **PASS** | host_link same-tx HostLinked/Unlinked; Mobile + Desktop arms; unit tests |
| **R2** | **PASS** | FriendRequestUpdated emit + Mobile refresh arm |
| **R3** | **PASS** | Thin account digest + Mobile conversation subscribe FRB; see audit §3 |
| **R4** | **PASS** | Desktop LRU + Mobile SubscriptionLimitExceeded UiEvent |

### R3 evidence (commands)

```text
cargo test -p minos-protocol --lib thin_digest
# → account_conversation_message_appended_is_thin_digest ok

cargo test -p minos-backend --lib account_fanout_is_thin
# → account_fanout_is_thin_digest_conversation_is_full ok

cargo test -p minos-backend --lib account_message
# → decision tests (push uses preview) ok

cargo test -p minos-mobile --lib parse_account_inbox
# → digest + recall stub frames ok

cargo test -p minos-backend --lib
# → 272 passed (includes delivery + notifications)

# Desktop
cd apps/desktop && pnpm test
# → 381+ passed
```

---

## V1 — Backend Success Definition (B7.1)

Mapped from [backend-im-delivery-orchestration.md](../2026-08-03-backend-im-delivery-orchestration.md) §Success + closeout B7.1 table.

| # | Scenario | Proof | Result |
|---|----------|-------|--------|
| 1 | Outbox crash after publish → re-publish; push no double success | `store::outbox_events::tests::requeue_stale_claims_restores_abandoned_claims`; `store::push_dispatch_log::tests::record_and_has_sent_round_trip`; `notifications::use_case::tests::event_id_idempotency_prevents_second_send`; `notifications::decision::tests::already_pushed_skips_idempotent`; `store::social::delivery::tests::client_message_id_retry_repairs_missing_durable` | **PASS** (lib) |
| 2 | social / host_command lanes do not starve each other | `store::outbox_events::tests::claim_available_isolates_lanes`; `requeue_stale_claims_is_lane_scoped` | **PASS** (lib) |
| 3 | host_command expire → dead_letter; observed ack wins over expire | `expired_host_command_refuses_success_ack_and_dead_letters`; `observed_host_ack_past_deadline_settles_outbox_acked_not_dead`; `expire_with_host_ack_settles_outbox_acked_and_times_out_command`; `expire_deadline_dead_letters_outbox_before_mark_timed_out`; `backend_timeout_finished_does_not_unlock_outbox_ack_or_ack_pending` | **PASS** (lib) |
| 4 | No host → send 200 + dispatch pending; host online force-due | integration: `agent_dispatch_queues_when_host_offline`; `agent_dispatch_drains_when_host_comes_online`; unit: `store::agent_dispatch_queue::tests::force_due_for_accounts_makes_backoff_rows_claimable` | **PASS** (lib + integration) |
| 5 | Same session two dispatches → two agent bubbles; origin formula | integration: `two_rapid_dispatches_project_two_agent_bubbles`; unit: `completion_watch::tests::two_origins_same_session_do_not_overwrite`; `turn_completion::tests::projector_idempotency_key_is_stable` | **PASS** (lib + integration) |
| 6 | SessionLifecycle dead host; watch TTL | `jobs::stale_session_sweeper::tests::ends_open_session_when_host_offline_and_stale`; `skips_session_when_host_still_live_in_registry`; `does_not_end_recently_seen_offline_host`; `completion_watch::tests::drain_expired_removes_past_deadline_only` | **PASS** (lib) |
| 7 | Reaction same `client_op_id` idempotent; conversation-only | `store::social::delivery::tests::reaction_same_client_op_id_is_idempotent`; `reaction_different_client_op_id_creates_distinct_events`; `reaction_delivery_is_conversation_only`; `reaction_event_id_formula_is_stable` | **PASS** (lib) |
| 8 | Approval `client_request_id` idempotent | `store::approval_requests::tests::resolve_stamps_client_request_id_and_lookup_is_idempotent`; `different_requests_cannot_reuse_same_client_request_id` | **PASS** (lib) |
| 9 | UserOnline skip; event_id push idempotent | `notifications::decision::tests::account_message_skipped_when_user_online`; `notifications::use_case::tests::online_presence_skips_push`; `offline_presence_sends_push`; `event_id_idempotency_prevents_second_send`; `approval_targets_conversation_members` | **PASS** (lib) |

### Backend Success Definition (spec §Success 1–7)

| Spec # | Invariant | Evidence |
|--------|-----------|----------|
| 1 | durable event_id replay → no double push success | V1 #1, #9 |
| 2 | online + approval push consistent; no dead branches | V1 #9 + decision/use_case tests |
| 3 | send HTTP not bound to host RPC; offline dispatch recoverable | V1 #4; `try_agent_dispatch` only enqueues (`http/v1/social.rs`) |
| 4 | N @agent → N bubbles origin 1:1 | V1 #5 |
| 5 | dead host → session + watch terminal | V1 #6 |
| 6 | Outbox lanes no starve; no fake expire ack | V1 #2, #3 |
| 7 | Delete list cleared | V1 G2 gates below |

### Commands (B7.1)

```text
# Lib suite (includes all unit-level rows above)
cargo test -p minos-backend --lib
# → 267 passed; 0 failed; exit 0  (2026-08-03)

# Critical integration filters
cargo test -p minos-backend --test v1_social -- \
  agent_dispatch_queues_when_host_offline \
  agent_dispatch_drains_when_host_comes_online \
  two_rapid_dispatches_project_two_agent_bubbles
# → 3 passed; 0 failed; exit 0  (2026-08-03)
```

---

## V1 — Delete list / rg gates (B7.2 + G2)

### Automated gate script

```text
./scripts/im-reliability-gates.sh
# exit 0 — ALL PASS (2026-08-03)
```

| Gate | Result | Notes |
|------|--------|-------|
| `client_message_id: None` in production | **PASS** | Mobile `send_chat_message` takes `Option<String>` and passes through (`crates/minos-mobile/src/client.rs`) |
| hub soft-dedupe 120s | **PASS** | No `120_000` in `hub-timeline.ts`; merge is id-equality only |
| SessionLifecycle COUNT-only stub | **PASS** | Job calls `end_stale_host_sessions` + `expire_completion_watches`; DidWork only when `n > 0` |
| presence “callers should” lies | **PASS** | No matches in crates/apps/architecture docs |
| reaction `event_id` Uuid | **PASS** | `reaction_event_id` formula; `Uuid::new_v4` only for unrelated row/outbox ids |
| `messageSeq ?? 0` | **PASS** | Desktop parses missing seq as `undefined` |

### Backend Delete list (code inspection)

| Delete item | Status | Path / note |
|-------------|--------|-------------|
| `UserOnline` dead / caller lies | **CLEARED** | `DecisionReason::UserOnline` applied in `decide()` when `presence.suppresses_push()` |
| Approval/SessionEnded empty targets but decide branch | **CLEARED** | `approval_targets_conversation_members` test; resolve path real |
| COUNT-only sweeper | **CLEARED** | `stale_session_sweeper.rs` SessionLifecycleJob |
| completion arm by session overwrite | **CLEARED** | keyed by `origin_message_id` + session secondary index |
| `watcher_from_seq = last_seq` as turn primary | **CLEARED** | turn write id = origin formula; `watcher_from_seq` is probe floor only |
| sync `try_agent_dispatch` host RPC in HTTP | **CLEARED** | enqueue only; host RPC in worker batch |
| reaction event_id Uuid | **CLEARED** | formula in `delivery.rs` |
| host_command expire fake success ack | **CLEARED** | dead_letter path + unit tests |
| cooldown-only push dedupe | **CLEARED** | `push_dispatch_log` event_id idempotency |

### Client Delete list (code inspection + gates)

| Delete item | Status | Evidence |
|-------------|--------|----------|
| `client_message_id: None` hardcode | **CLEARED** | G2 + client.rs pass-through |
| per-event `invalidateSelf` Conversations hot path | **CLEARED** | `ConversationsController` hot path: single-row patch comment + `_onSocialEvent` |
| `DELETE FROM cached_social_conversations` hot path | **CLEARED** | `saveConversations` documented hydrate-only; hot path `upsertConversation` |
| mark-read every inbound | **CLEARED** | Desktop `im-mark-read-debounce` 400ms contract test |
| `COALESCE(seq, ms)` order | **CLEARED** | Mobile index comment forbids COALESCE with ms; sorts by seq nulls last |
| `messageSeq ?? 0` | **CLEARED** | G2 + minos-cloud.ts |
| body+120s soft dedupe | **CLEARED** | G2 + hub-timeline tests |
| live-ingress 0/400/1200 burst | **CLEARED** | comment C2; no burst timers |
| Timeline 2s completion trail poll | **CLEARED** | Timeline.tsx C2 comment |
| `loadTimeline` writes focused | **CLEARED** | timeline.ts hydrate-only docs + mark-read debounce test |
| outbox listDue only pending | **CLEARED** | im-outbox reclaim tests |

### Whitelist (acceptable residuals, not gate failures)

| Item | Why allowed |
|------|-------------|
| `Uuid::new_v4` for `reaction_id` row PK / outbox_id | Not durable `event_id` |
| `origin_message_id: None` in TUI local daemon RPC | Documented residual; non-collab local workbench |
| `saveConversations` full-table replace | Hydrate/refresh only, not inbound hot path |
| `invalidateSelf` on non-inbox stream error handlers | Not ConversationsController per-event wipe |
| `120_000` as RPC/delegation timeouts | Unrelated to soft-dedupe |
| Layer P: digest dual-track, LRU, Desktop Hub approval | Explicit residuals; not Delete list |

---

## V2 — G3 test suites

### Backend lib

```text
cargo test -p minos-backend --lib
# 270 passed; 0 failed; 0 ignored
# exit 0  (2026-08-03 re-run; +host_link durable tests)
```

### Backend host roster (R1)

```text
cargo test -p minos-backend --lib host_link
# link_host_enqueues_host_linked_durable PASS
# unlink_host_enqueues_host_unlinked_durable PASS
# exit 0
```

### Backend integration (critical IM)

```text
cargo test -p minos-backend --test v1_social -- \
  agent_dispatch_queues_when_host_offline \
  agent_dispatch_drains_when_host_comes_online \
  two_rapid_dispatches_project_two_agent_bubbles
# 3 passed; exit 0
```

### Desktop IM unit tests

```text
cd apps/desktop && node --experimental-strip-types --test \
  src/shared/lib/im-outbox.test.ts \
  src/shared/lib/hub-timeline.test.ts \
  src/shared/lib/im-timeline-sync.test.ts \
  src/shared/lib/timeline-order.test.ts \
  src/features/chat/lib/reactions.test.ts \
  src/shared/lib/im-cloud-sync.test.ts \
  src/shared/lib/im-mark-read-debounce.test.ts \
  src/shared/lib/hub-realtime.test.ts \
  src/shared/lib/hub-cursors.test.ts \
  src/shared/lib/message-history.test.ts
# 78 passed; 0 failed; exit 0  (+ R4 LRU pure tests)
```

Key coverage: outbox reclaim/agent_result/reaction/approval kinds; no soft-dedupe; canonical agent-result id; timeline order by seq; Snapshot range helpers; mark-read debounce; conversation subscription LRU.

### Mobile unit tests

```text
# If pub.dev TLS fails, use China mirror (or any reachable mirror):
export PUB_HOSTED_URL=https://pub.flutter-io.cn
cd apps/mobile && flutter pub get
flutter test \
  test/infrastructure/im_outbox_store_test.dart \
  test/application/conversations_sort_test.dart \
  test/domain/social_message_order_test.dart
# 17 passed; 0 failed; exit 0  (2026-08-03)
```

**G3 overall:** **PASS** (backend + desktop + mobile unit green).

---

## V3 — Client Success 9 bars (C6.4)

From [client-im-sync-engine.md](../2026-08-03-client-im-sync-engine.md) §Success Definition.

| # | Invariant | Proof class | Evidence |
|---|-----------|-------------|----------|
| 1 | At-most-once visible / idempotent send | **AUTOMATED** (backend + Desktop + Mobile outbox) | Hub idempotency + Desktop im-outbox + Mobile `ImOutboxMemory` enqueue idempotent / acked no-op (17 unit tests). Device kill matrix: **MANUAL_REQUIRED** (runbook) |
| 2 | Process death / inflight reclaim | **AUTOMATED** (Desktop + Mobile) | Desktop im-outbox reclaim; Mobile `stale inflight reclaim` + `startup reclaim covers sending rows` **PASS** |
| 3 | Hot path O(1) inbox | **PASS_BY_CODE_INSPECTION** | ConversationsController: no per-event invalidateSelf / full REST; `upsertConversation` + patch |
| 4 | unread: background +1 / focus clear / own no-bump | **PASS_BY_CODE_INSPECTION** + unit | Mobile `bumpUnread`; Desktop mark-read debounce 400ms unit. Dual-end observation: **MANUAL_REQUIRED** |
| 5 | loadOlder + Snapshot range | **AUTOMATED** (Desktop) + **CODE_INSPECTION** (Mobile) | `im-timeline-sync.test.ts`; Mobile Snapshot consumer + loadOlder |
| 6 | Sort by message_seq | **AUTOMATED** | Desktop `timeline-order.test.ts`; Mobile `social_message_order_test.dart` **17 suite includes 4 order tests PASS** |
| 7 | Agent bubble same id / no soft-dedupe | **AUTOMATED** | hub-timeline + backend two-bubble integration |
| 8 | Reaction offline deliverable | **AUTOMATED** (outbox) | Desktop reaction_toggle due queue; Mobile outbox classification unit. Device offline deliver: **MANUAL_REQUIRED** |
| 9 | Client Delete list clear | **AUTOMATED** (G2) | `scripts/im-reliability-gates.sh` |

**C6.4 overall:** structural bars automated; device kill/offline still runbook → **PARTIAL** (not full green).

---

## V3 — Multi-end matrix (C6.5)

| Scenario | Desktop | Mobile | Backend | Status |
|----------|---------|--------|---------|--------|
| Mutual text send | unit outbox + hub merge | outbox unit + code | client_message_id idempotency | **PARTIAL** — no live multi-client run |
| @agent two bubbles | code + backend integration | code path | `two_rapid_dispatches…` **PASS** | **PARTIAL** |
| Online no push | n/a | n/a | `online_presence_skips_push` **PASS** | **AUTOMATED** |
| SnapshotRequired | timeline-sync unit | Snapshot consumer code | n/a | **PARTIAL** |
| Offline reaction | outbox reaction_toggle unit | outbox unit | reaction idempotency **PASS** | **PARTIAL** |
| Sleep / resume live | C6.1 forceReconnect code | reconnect unit (mobile) | n/a | **PASS_BY_CODE_INSPECTION** |
| Approval intent | daemon outbox unit | Hub client_request_id | approval store tests **PASS** + Desktop P3 Hub HTTP | **PASS** (P3 Desktop Hub path shipped) |

**C6.5 overall:** live multi-end **NOT_RUN**; automated rows as above.

### V3 device runbook (do not fake PASS)

Use only when QA has two clients + hub:

1. **Mutual text** — Desktop + Mobile same conversation; both show one bubble; Hub `message_seq` monotonic.
2. **Kill sender mid-outbox** — send, airplane mode, force-quit app, relaunch; outbox drains; Hub still one row for `client_message_id`.
3. **@agent ×2** — two @mentions same session; two agent bubbles with distinct origin ids.
4. **Online no push** — Mobile foreground + WS live; trigger message; no APNs/FCM log success (policy unit already covers).
5. **Offline reaction** — Desktop offline toggle reaction; reconnect; single durable reaction event_id.
6. **Sleep/resume** — lock phone 60s; unlock; account resume_after catch-up without full wipe.
7. **Host link live (R1)** — link host on Desktop account path; Mobile hosts list upserts without pull-to-refresh.

---

## V4 — Program DoD (G4)

README §3 checkboxes updated **only** where evidence is solid. Full G4 **not** claimed (device matrix open).

See [README.md](README.md) §3 and [TASKS.md](TASKS.md) B7 / C6.4–C6.5 / G2–G4.

### Index: DoD → EVIDENCE

| DoD area | EVIDENCE section |
|----------|------------------|
| Write path outbox / idempotency | V1 #1, V2 Desktop + Mobile outbox, V3 #1–2 |
| Async dispatch | V1 #4, send_message_inner enqueue |
| O(1) inbox / unread / pagination | V3 #3–5 |
| Agent bubbles | V1 #5, V3 #7 |
| Push / lanes / lifecycle | V1 #2–3, #6, #9 |
| Delete list | V1 G2 + delete tables |
| Realtime Host roster | R0 audit + R1 host_link tests |

---

## Layer P — Product residuals

See [LAYER-P.md](LAYER-P.md). Status: P1–P4/P6 **DONE**; P5 **BLOCKED** (ops secrets, honest `NotWired`).

## Artifacts

| Path | Purpose |
|------|---------|
| `EVIDENCE.md` | This file |
| `LAYER-P.md` | Layer P evidence + status |
| `scripts/im-reliability-gates.sh` | G2/B7.2 gates |
| `realtime-surface-audit.md` | R0 DurableEvent × HTTP audit |
| `apps/desktop/src/shared/lib/conversation-sub-lru.ts` | R4 pure LRU |
