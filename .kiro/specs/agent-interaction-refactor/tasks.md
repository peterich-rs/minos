# Implementation Plan: Agent Interaction Refactor

## Overview

This plan implements the architectural refactoring of the mobile ↔ agent interaction model across 15 tasks organized in dependency order. Tasks 1–5 build the host-side foundation (wire types, steer, dispatch, policies, approvals). Tasks 6–8 implement server-side routing and orchestration. Tasks 9–12 refactor the mobile client. Tasks 13–15 handle history reads, UI updates, and end-to-end validation.

## Tasks

- [x] 1. Add wire types and envelope variants to `minos-protocol`. Add `AgentDispatchRequest/Response`, `ApprovalDecisionRequest` to `messages.rs`. Add `ApprovalRequest`, `ApprovalTimeout`, `AgentError` variants to `EventKind` in `envelope.rs`. Add golden JSON fixture tests for each new variant. Run `cargo test -p minos-protocol`.
- [x] 2. Implement `steer_turn` in `AgentManager`. Add `steer_turn(&self, thread_id, text)` to `manager.rs` that looks up the thread handle, gets the instance, synthesizes user-message ingest, and calls `inst.steer_turn()`. Add `steer_turn` to `AppServerInstance` using `TurnSteerParams`. Track `active_turn_id` on `ThreadHandle` (set on `turn/started`, clear on `turn/completed`). Modify `send_user_message` to call `steer_turn` when state is Running. Add unit tests with `FakeCodexBackend`.
- [x] 3. Implement `dispatch_message` in `AgentManager` and `AgentGlue`. Add `SessionPolicies` struct. Add `dispatch_message` to `AgentManager` that routes: None session_id → auto-create + send, Some → state-based dispatch (Idle→start, Running→steer, Suspended→resume). Add `dispatch_message` to `AgentGlue` with local-store persistence. Add `minos_agent_dispatch` arm to `invoke_forwarded` in `rpc_server.rs`. Add unit tests for each state branch.
- [x] 4. Remove hardcoded `approval_policy=never` from codex spawn. In `spawn_instance`, remove the hardcoded `-c approval_policy=never` arg. Read policies from config.toml when `SessionPolicies` is None. Pass profile-specified policies as `-c` flags when present. Validate policy values against allowed sets. Add unit tests.
- [x] 5. Replace approval auto-reject with forwarding in `event_pump_loop`. Add `PendingApproval` struct and `PendingApprovals` map. In `Inbound::ServerRequest` arm, store pending entry and emit `RawIngest` with `method: "approval/request"`. Add `resolve_approval` method. Spawn per-request timeout task (120s default). Add `minos_approval_decision` arm to `rpc_server.rs`. Add unit tests for forwarding, timeout, and decision reply.
- [ ] 6. Server-side agent routing in chat message handler. Create migration adding `agent_session_id` column to `chat_messages` and `pending_approvals` table. Add `bind_session_to_message` and `lookup_session_id_for_message` store helpers. In `send_chat_message` handler: detect 1:1 agent conversations, detect @mentions in groups, forward `AgentDispatchRequest` to host, bind session_id to messages. Handle reply-to-agent-message session reuse. Add integration tests.
- [ ] 7. Server-side group chat completion watcher. Implement `spawn_group_completion_watcher` that subscribes to ingest events for a thread_id. On `item/completed`: extract final text, post agent-attributed reply message with `reply_to_message_id` and `@userId`, bind session_id. Implement 300s timeout with error message posting. Ensure intermediate events are NOT fanned out to group participants. Add integration tests.
- [ ] 8. Server-side approval relay and timeout. In `ingest/mod.rs`, detect `method: "approval/request"` and emit `EventKind::ApprovalRequest` instead of `UiEventMessage`. Insert `pending_approvals` row. Spawn background poller for expired entries. Add endpoint for mobile to submit `ApprovalDecisionRequest`. On decision: update row, forward to host. On mobile disconnect: resolve pending approvals as 'disconnected'. Add integration tests.
- [ ] 9. Mobile state machine simplification (Dart). Replace `SessionStarting` with `SessionSending`, rename `SessionStopped` to `SessionSuspended` in `active_session.dart`. Remove `start()`/`startAndSend()` from `ActiveSessionController`. Modify `send()` for Idle→Sending→Streaming flow. Change `stop()` to call `interruptThread`. Delete `group_agent_dispatcher.dart`. Update `minos_core_protocol.dart` and `minos_core.dart`. Update all unit tests.
- [ ] 10. Mobile approval UI. Create `approval_sheet.dart` modal bottom sheet showing request details per type (command, file change, permissions). Show decision buttons and countdown timer. On tap: call `core.sendApprovalDecision()`. Add `ApprovalRequest` event handling in `ThreadViewPage`. Handle `ApprovalTimeout` (dismiss sheet + toast). Add `sendApprovalDecision` to protocol/core/FRB/mobile-client.
- [ ] 11. Mobile interrupt vs close separation. Add `interruptThread` to `MinosCoreProtocol` and `MinosCore`. Change `ActiveSessionController.stop()` to call `interruptThread` → `SessionSuspended`. Add `deleteThread` for permanent close (swipe-to-delete only). Update `InputBar` stop button and thread list delete action. Add unit tests verifying interrupt vs close semantics.
- [ ] 12. Remove `startAgent` from mobile Rust layer. Remove `start_agent` and `start_agent_in_project` from `minos-mobile/src/client.rs` and `minos-ffi-frb/src/api/minos.rs`. Run `flutter_rust_bridge_codegen`. Fix Dart compilation errors. Update `agent_start_page.dart` to create conversation instead of starting agent. Update `thread_view_page.dart` to remove `startAndSend` path. Fix all test mocks.
- [ ] 13. Server-side history read through translation. Add `read_thread` HTTP endpoint to `minos-backend` accepting `thread_id`, `from_seq`, `limit`. Query `raw_events`, translate with fresh `CodexTranslatorState`, return `UiEventMessage` list + cursor. Handle translation failures inline. Add authorization check. Update mobile's `readThread` to route through server. Add integration tests.
- [ ] 14. Update `thread_view_page.dart` for new architecture. Remove `targetThreadId == null` branch calling `startAndSend`. All sends go through `sendChatMessage`. Add `ApprovalRequest` event handling to trigger approval sheet. Simplify `_dispatchMessage` to use new controller. Keep `AgentProfile` selection for pre-chat agent choice. Remove workspace selection from dispatch path.
- [ ] 15. End-to-end integration tests. Add daemon e2e tests: new session creation, running session steer, interrupt + resume, approval forwarding + decision. Add backend e2e tests: group chat mention-to-completion, group reply session reuse, 1:1 agent conversation lifecycle. Verify all existing tests pass with `cargo test --workspace`.

## Task Dependency Graph

```json
{
  "waves": [
    [1],
    [2, 6],
    [3, 4, 7],
    [5, 8],
    [9, 11],
    [10, 12],
    [13, 14],
    [15]
  ]
}
```

- Wave 1: Task 1 (wire types) is the foundation
- Wave 2: Tasks 2, 6 can start in parallel (host steer + server routing)
- Wave 3: Tasks 3, 4, 7 depend on wave 2 outputs
- Wave 4: Tasks 5, 8 (approval forwarding on both sides)
- Wave 5: Tasks 9, 11 (mobile state machine + interrupt)
- Wave 6: Tasks 10, 12 (approval UI + remove startAgent)
- Wave 7: Tasks 13, 14 (history reads + UI update)
- Wave 8: Task 15 (end-to-end validation)

## Notes

- Tasks 2–5 form the "host foundation" phase and should be merged together or in rapid sequence to avoid a half-migrated state on the daemon.
- Task 12 (removing startAgent) is intentionally late because it's a breaking change — all other layers must be ready before mobile drops the old path.
- The existing `minos_start_agent` and `minos_send_user_message` RPC arms in `rpc_server.rs` should be kept temporarily for backward compatibility until Task 12 lands. The new `minos_agent_dispatch` arm coexists with them.
- Golden test fixtures should be added incrementally as each envelope variant lands (Task 1) rather than batched at the end.
