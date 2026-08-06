# Layer P — Product residuals (evidence)

| Field | Value |
|-------|--------|
| Status | **Executed** 2026-08-04 |
| Branch | `feat/im-reliability-program` |
| Spec | [closeout-and-backlog.md](closeout-and-backlog.md) §3 |

## Summary

| ID | Status | Evidence |
|----|--------|----------|
| **P1** | **DONE** | Hub IM mode rail unread SSOT = digest only (`unreadSource: "hub"`). Local `readMessageCountById` baseline only for daemon-only / unauthenticated. Tests: `hub-digest-cache.test.ts` (`resolveRailUnread`, merge hub/local). |
| **P2** | **DONE** | Already shipped in R4: `conversation-sub-lru.ts` + `HubRealtimeSession.subscribeConversation` calls `unsubscribeConversation` on LRU eviction. Tests in `hub-realtime.test.ts`. |
| **P3** | **DONE** | Desktop `syncApprovalResolve` branches by Hub auth: Hub → `POST /v1/approvals/respond` + top-level `client_request_id` via Intent Outbox; local → daemon resolve without stuffing id into decision JSON. |
| **P4** | **DONE** | TUI conversation submit generates `tui-{conv}-{ms}` origin, appends with that id, passes `origin_message_id` on `minos_local_send_user_message`. Pure local workbench (no conversation) still `None` (documented). MCP/delegation paths without conversation user rows stay `None`. |
| **P5** | **BLOCKED (ops secrets)** | Clean channel interface + env config hooks remain. `send` returns `PushSendOutcome::NotWired` (never fake `Sent`). Runtime registers APNs/FCM by kind. Production wire needs Apple/Google credentials + provider HTTP. |
| **P6** | **DONE (process-local contract)** | CompletionWatch stays in-process; module docs + host-online force-due re-arm path documented. Full Redis-backed watch registry deferred until multi-replica host affinity is required. |

## Key paths

| Area | Paths |
|------|--------|
| P1 | `apps/desktop/src/shared/lib/conversation-list-merge.ts`, `conversation-list.ts` |
| P2 | `apps/desktop/src/shared/lib/conversation-sub-lru.ts`, `hub-realtime.ts` |
| P3 | `apps/desktop/src/shared/lib/im-cloud-sync.ts`, `minos-cloud.ts` (`respondHubApproval`), `use-cases.ts` |
| P4 | `crates/minos-tui/src/app/submission.rs`, `backend/{mod,daemon}.rs` |
| P5 | `crates/minos-backend/src/notifications/channels/{apns,fcm,mod,composite}.rs`, `runtime.rs` |
| P6 | `crates/minos-backend/src/completion_watch.rs`, `http/v1/social.rs` (`on_host_online_force_agent_dispatch`) |

## Honest residuals after Layer P

- **P5 production push**: blocked on ops secrets + `a2`/FCM HTTP v1 implementation body.
- **P6 Redis watch store**: not required under co-located host WS + dispatch worker (current deploy model).
- **TUI pure local / MCP delegation**: `origin_message_id: None` when no conversation user message id exists (non-collab workbench).
