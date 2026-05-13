# Tasks

- [x] 1. Add wire types and envelope variants to `minos-protocol`
  - [x] 1.1 Add AgentDispatchRequest/Response and ApprovalDecisionRequest to messages.rs
  - [x] 1.2 Add ApprovalRequest, ApprovalTimeout, AgentError variants to EventKind in envelope.rs
  - [x] 1.3 Add golden JSON fixture tests for each new variant and run cargo test -p minos-protocol
- [x] 2. Implement steer_turn in AgentManager
  - [x] 2.1 Add steer_turn to manager.rs that looks up the thread handle, gets the instance, synthesizes user-message ingest
  - [x] 2.2 Add steer_turn to AppServerInstance using TurnSteerParams
  - [x] 2.3 Track active_turn_id on ThreadHandle
  - [x] 2.4 Modify send_user_message to call steer_turn when state is Running
  - [x] 2.5 Add unit tests with FakeCodexBackend
- [x] 3. Implement dispatch_message in AgentManager and AgentGlue
  - [x] 3.1 Add SessionPolicies struct
  - [x] 3.2 Add dispatch_message to AgentManager that routes based on session_id presence and state
  - [x] 3.3 Add dispatch_message to AgentGlue with local-store persistence
  - [x] 3.4 Add minos_agent_dispatch arm to invoke_forwarded in rpc_server.rs
  - [x] 3.5 Add unit tests for each state branch
- [x] 4. Remove hardcoded approval_policy=never from codex spawn
  - [x] 4.1 In spawn_instance remove the hardcoded approval_policy=never arg
  - [x] 4.2 Read policies from config.toml when SessionPolicies is None
  - [x] 4.3 Pass profile-specified policies as -c flags when present
  - [x] 4.4 Validate policy values against allowed sets
  - [x] 4.5 Add unit tests
- [x] 5. Replace approval auto-reject with forwarding in event_pump_loop
  - [x] 5.1 Add PendingApproval struct and PendingApprovals map
  - [x] 5.2 In Inbound::ServerRequest arm store pending entry and emit RawIngest with method approval/request
  - [x] 5.3 Add resolve_approval method
  - [x] 5.4 Spawn per-request timeout task (120s default)
  - [x] 5.5 Add minos_approval_decision arm to rpc_server.rs
  - [x] 5.6 Add unit tests for forwarding, timeout, and decision reply
- [x] 6. Server-side agent routing in chat message handler
  - [x] 6.1 Create migration adding agent_session_id column to chat_messages and pending_approvals table
  - [x] 6.2 Add bind_session_to_message and lookup_session_id_for_message store helpers
  - [x] 6.3 In send_chat_message handler detect 1:1 agent conversations and mentions in groups and forward to host
  - [x] 6.4 Handle reply-to-agent-message session reuse
  - [x] 6.5 Add integration tests
- [x] 7. Server-side group chat completion watcher
  - [x] 7.1 Implement spawn_group_completion_watcher that subscribes to ingest events for a thread_id
  - [x] 7.2 On item/completed extract final text and post agent-attributed reply message
  - [x] 7.3 Implement 300s timeout with error message posting
  - [x] 7.4 Ensure intermediate events are NOT fanned out to group participants
  - [x] 7.5 Add integration tests
- [x] 8. Server-side approval relay and timeout
  - [x] 8.1 Detect method approval/request in ingest and emit EventKind::ApprovalRequest
  - [x] 8.2 Insert pending_approvals row
  - [x] 8.3 Spawn background poller for expired entries
  - [x] 8.4 Add endpoint for mobile to submit ApprovalDecisionRequest
  - [x] 8.5 On decision update row and forward to host
  - [x] 8.6 Add integration tests
- [x] 9. Mobile state machine simplification (Dart)
  - [x] 9.1 Replace SessionStarting with SessionSending and rename SessionStopped to SessionSuspended in active_session.dart
  - [x] 9.2 Remove start() and startAndSend() from ActiveSessionController and modify send() for Idle to Sending to Streaming flow
  - [x] 9.3 Change stop() to call interruptThread and delete group_agent_dispatcher.dart
  - [x] 9.4 Update minos_core_protocol.dart and minos_core.dart
  - [x] 9.5 Update all unit tests
- [x] 10. Mobile approval UI
  - [x] 10.1 Create approval_sheet.dart modal bottom sheet showing request details per type with decision buttons and countdown timer
  - [x] 10.2 Add ApprovalRequest event handling in ThreadViewPage to call core.sendApprovalDecision()
  - [x] 10.3 Handle ApprovalTimeout by dismissing sheet and showing toast
  - [x] 10.4 Add sendApprovalDecision to protocol/core/FRB/mobile-client
- [x] 11. Mobile interrupt vs close separation
  - [x] 11.1 Add interruptThread to MinosCoreProtocol and MinosCore
  - [x] 11.2 Change ActiveSessionController.stop() to call interruptThread transitioning to SessionSuspended
  - [x] 11.3 Add deleteThread for permanent close (swipe-to-delete only)
  - [x] 11.4 Update InputBar stop button and thread list delete action
  - [x] 11.5 Add unit tests verifying interrupt vs close semantics
- [x] 12. Remove startAgent from mobile Rust layer
  - [x] 12.1 Remove start_agent and start_agent_in_project from minos-mobile/src/client.rs and minos-ffi-frb/src/api/minos.rs
  - [x] 12.2 Run flutter_rust_bridge_codegen and fix Dart compilation errors
  - [x] 12.3 Update agent_start_page.dart to create conversation instead of starting agent
  - [x] 12.4 Update thread_view_page.dart to remove startAndSend path and fix all test mocks
- [x] 13. Server-side history read through translation
  - [x] 13.1 Add read_thread HTTP endpoint to minos-backend accepting thread_id, from_seq, limit
  - [x] 13.2 Query raw_events, translate with fresh CodexTranslatorState, return UiEventMessage list plus cursor
  - [x] 13.3 Handle translation failures inline and add authorization check
  - [x] 13.4 Update mobile readThread to route through server
  - [x] 13.5 Add integration tests
- [ ] 14. Update thread_view_page.dart for new architecture
  - [ ] 14.1 Remove targetThreadId == null branch calling startAndSend and route all sends through sendChatMessage
  - [ ] 14.2 Add ApprovalRequest event handling to trigger approval sheet
  - [ ] 14.3 Simplify _dispatchMessage to use new controller and keep AgentProfile selection for pre-chat agent choice
  - [ ] 14.4 Remove workspace selection from dispatch path
- [ ] 15. End-to-end integration tests
  - [ ] 15.1 Add daemon e2e tests for new session creation, running session steer, interrupt plus resume, approval forwarding plus decision
  - [ ] 15.2 Add backend e2e tests for group chat mention-to-completion, group reply session reuse, 1:1 agent conversation lifecycle
  - [ ] 15.3 Verify all existing tests pass with cargo test --workspace

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

> Note: UI widget test cases were removed for this refactor. Verification is now limited to logic-focused unit tests plus existing non-UI integration coverage.
