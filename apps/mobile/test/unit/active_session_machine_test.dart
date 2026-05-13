import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeCore implements MinosCoreProtocol {
  final _uiCtl = StreamController<UiEventFrame>.broadcast();

  MinosError? interruptError;
  int interruptCount = 0;
  String? lastInterruptThreadId;

  MinosError? deleteError;
  int deleteCount = 0;
  String? lastDeleteThreadId;

  void emit(UiEventFrame frame) => _uiCtl.add(frame);

  Future<void> dispose() => _uiCtl.close();

  @override
  Stream<UiEventFrame> get uiEvents => _uiCtl.stream;

  @override
  Stream<SocialEventFrame> get socialEvents =>
      const Stream<SocialEventFrame>.empty();

  @override
  Future<void> interruptThread({required String threadId}) async {
    if (interruptError != null) throw interruptError!;
    interruptCount += 1;
    lastInterruptThreadId = threadId;
  }

  @override
  Future<void> closeThread({required String threadId}) async {}

  @override
  Future<void> deleteThread({required String threadId}) async {
    if (deleteError != null) throw deleteError!;
    deleteCount += 1;
    lastDeleteThreadId = threadId;
  }

  @override
  Future<void> sendApprovalDecision({
    required String requestId,
    required String threadId,
    required Map<String, dynamic> decision,
  }) async {}

  @override
  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) async {}

  @override
  Future<FriendRequestSummary> acceptFriendRequest({
    required String requestId,
  }) async => throw UnimplementedError();

  @override
  Future<ConversationsResponse> conversations() async =>
      const ConversationsResponse(conversations: <ConversationSummary>[]);

  @override
  Future<FriendRequestSummary> createFriendRequest({
    required String targetMinosId,
  }) async => throw UnimplementedError();

  @override
  Future<ConversationResponse> createGroupConversation({
    required String title,
    required List<String> memberAccountIds,
  }) async => throw UnimplementedError();

  @override
  Future<void> addGroupMember({
    required String conversationId,
    required String memberAccountId,
  }) async {}

  @override
  Future<ConversationMembersResponse> conversationMembers({
    required String conversationId,
  }) async => const ConversationMembersResponse(members: <UserSummary>[]);

  @override
  Future<ConversationAgentMembersResponse> listConversationAgents({
    required String conversationId,
  }) async => const ConversationAgentMembersResponse(agents: <AgentSummary>[]);

  @override
  Future<void> addAgentToConversation({
    required String conversationId,
    required String agentId,
  }) async {}

  @override
  Future<void> removeAgentFromConversation({
    required String conversationId,
    required String agentId,
  }) async {}

  @override
  Future<ConversationReadResponse> markConversationRead({
    required String conversationId,
  }) async => const ConversationReadResponse();

  @override
  Future<ConversationResponse> ensureDirectConversation({
    required String friendAccountId,
  }) async => throw UnimplementedError();

  @override
  Future<FriendRequestsResponse> friendRequests() async =>
      const FriendRequestsResponse(
        incoming: <FriendRequestSummary>[],
        outgoing: <FriendRequestSummary>[],
      );

  @override
  Future<FriendsResponse> friends() async =>
      const FriendsResponse(friends: <FriendSummary>[]);

  @override
  Future<ListChatMessagesResponse> listChatMessages({
    required String conversationId,
    int? beforeTsMs,
    int limit = 50,
  }) async => const ListChatMessagesResponse(messages: <ChatMessageSummary>[]);

  @override
  Future<MyProfileResponse> myProfile() async => const MyProfileResponse(
    accountId: 'acc',
    email: 'test@example.com',
    minosId: 'Test001',
  );

  @override
  Future<FriendRequestSummary> rejectFriendRequest({
    required String requestId,
  }) async => throw UnimplementedError();

  @override
  Future<List<UserSummary>> searchUsers({required String minosId}) async =>
      const <UserSummary>[];

  @override
  Future<ChatMessageSummary> sendChatMessage({
    required String conversationId,
    required String text,
    String? replyToMessageId,
  }) async => throw UnimplementedError();

  @override
  Future<ChatMessageSummary> recallChatMessage({
    required String conversationId,
    required String messageId,
  }) async => throw UnimplementedError();

  @override
  Future<MyProfileResponse> setMinosId({required String minosId}) async =>
      MyProfileResponse(
        accountId: 'acc',
        email: 'test@example.com',
        minosId: minosId,
      );

  @override
  Future<List<AgentDescriptor>> listClis() async => const [];

  @override
  Future<ListHostSkillsResponse> listHostSkills({
    String? hostDeviceId,
    bool forceReload = true,
  }) async => const ListHostSkillsResponse(data: <HostSkillsEntry>[]);

  @override
  Future<WriteHostSkillConfigResponse> writeHostSkillConfig({
    String? hostDeviceId,
    required String path,
    required bool enabled,
  }) async => WriteHostSkillConfigResponse(effectiveEnabled: enabled);

  @override
  Future<CreateProjectResponse> createProject({
    required String name,
    required String workspaceSlug,
  }) async => throw UnimplementedError();

  @override
  Future<ListProjectsResponse> listProjects() async =>
      const ListProjectsResponse(projects: <ProjectSummary>[]);

  @override
  Future<void> updateProject({
    required String projectId,
    required String name,
  }) async {}

  @override
  Future<void> deleteProject({required String projectId}) async {}

  @override
  Future<ListProjectThreadsResponse> listProjectThreads({
    required String projectId,
    int limit = 50,
    int? beforeTsMs,
  }) async => const ListProjectThreadsResponse(threads: <ThreadSummary>[]);

  @override
  Stream<AuthStateFrame> get authStates => const Stream<AuthStateFrame>.empty();

  @override
  Stream<ConnectionState> get connectionStates =>
      const Stream<ConnectionState>.empty();

  @override
  ConnectionState get currentConnectionState =>
      const ConnectionState.disconnected();

  @override
  Future<AuthSummary> register({
    required String email,
    required String password,
  }) async => throw UnimplementedError();

  @override
  Future<AuthSummary> login({
    required String email,
    required String password,
  }) async => throw UnimplementedError();

  @override
  Future<void> refreshSession() async {}

  @override
  Future<void> logout() async {}

  @override
  Future<void> pairWithQrJson(String qrJson) async {}

  @override
  Future<void> forgetHost(String hostDeviceId) async {}

  @override
  Future<List<HostSummaryDto>> listPairedHosts() async =>
      const <HostSummaryDto>[];

  @override
  Future<String?> activeHost() async => null;

  @override
  Future<void> setActiveHost(String hostDeviceId) async {}

  @override
  Future<bool> hasPersistedPairing() async => false;

  @override
  Future<String?> peerDisplayName() async => null;

  @override
  Future<void> setPeerDisplayName(String? name) async {}

  @override
  Future<ListThreadsResponse> listThreads(ListThreadsParams params) async =>
      const ListThreadsResponse(threads: <ThreadSummary>[]);

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) async =>
      const ReadThreadResponse(uiEvents: <UiEventMessage>[]);

  @override
  void notifyForegrounded() {}

  @override
  void notifyBackgrounded() {}

  @override
  Future<void> resumePersistedSession() async {}
}

ProviderContainer _container(_FakeCore core) {
  final container = ProviderContainer(
    overrides: [minosCoreProvider.overrideWithValue(core)],
  );
  addTearDown(() async {
    await core.dispose();
    container.dispose();
  });
  return container;
}

UiEventFrame _frame({
  required String threadId,
  required BigInt seq,
  required UiEventMessage ui,
}) {
  return UiEventFrame(threadId: threadId, seq: seq, ui: ui, tsMs: 1);
}

void main() {
  test('initial state is SessionIdle', () {
    final core = _FakeCore();
    final c = _container(core);
    expect(c.read(activeSessionControllerProvider), const SessionIdle());
  });

  test(
    'send() drives Idle -> Sending until the first frame binds a thread',
    () async {
      final core = _FakeCore();
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      final dispatched = Completer<void>();
      final sendFuture = notifier.send(
        agent: AgentName.codex,
        text: 'hello',
        dispatch: () => dispatched.future,
      );
      expect(
        c.read(activeSessionControllerProvider),
        const SessionSending(agent: AgentName.codex, text: 'hello'),
      );

      dispatched.complete();
      await sendFuture;
      expect(
        c.read(activeSessionControllerProvider),
        const SessionSending(agent: AgentName.codex, text: 'hello'),
      );

      core.emit(
        _frame(
          threadId: 'thr-A',
          seq: BigInt.zero,
          ui: const UiEventMessage.messageStarted(
            messageId: 'm-user',
            role: MessageRole.user,
            startedAtMs: 1,
          ),
        ),
      );
      await pumpEventQueue();

      expect(
        c.read(activeSessionControllerProvider),
        const SessionStreaming(threadId: 'thr-A', agent: AgentName.codex),
      );
    },
  );

  test(
    'send() error from Idle transitions to SessionError with no threadId',
    () async {
      final core = _FakeCore();
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      final error = await notifier.send(
        agent: AgentName.codex,
        text: 'hi',
        dispatch: () async =>
            throw const MinosError.agentStartFailed(reason: 'no daemon'),
      );

      final st = c.read(activeSessionControllerProvider);
      expect(st, isA<SessionError>());
      expect((st as SessionError).threadId, isNull);
      expect(st.error, isA<MinosError_AgentStartFailed>());
      expect(error, isA<MinosError_AgentStartFailed>());
    },
  );

  test('reset() clears a stale thread-bound error back to SessionIdle', () {
    final core = _FakeCore();
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    notifier.state = const SessionError(
      threadId: 'thr-reset',
      error: MinosError.timeout(),
    );
    notifier.reset();

    expect(c.read(activeSessionControllerProvider), const SessionIdle());
  });

  test('MessageCompleted on matching thread -> SessionAwaitingInput', () async {
    final core = _FakeCore();
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    notifier.state = const SessionStreaming(
      threadId: 'thr-B',
      agent: AgentName.codex,
    );
    core.emit(
      _frame(
        threadId: 'thr-B',
        seq: BigInt.zero,
        ui: const UiEventMessage.messageCompleted(
          messageId: 'm1',
          finishedAtMs: 1,
        ),
      ),
    );
    await pumpEventQueue();

    expect(
      c.read(activeSessionControllerProvider),
      const SessionAwaitingInput(threadId: 'thr-B', agent: AgentName.codex),
    );
  });

  test('UiEvent on a different thread is ignored', () async {
    final core = _FakeCore();
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    notifier.state = const SessionStreaming(
      threadId: 'thr-B',
      agent: AgentName.codex,
    );
    core.emit(
      _frame(
        threadId: 'thr-OTHER',
        seq: BigInt.zero,
        ui: const UiEventMessage.messageCompleted(
          messageId: 'mx',
          finishedAtMs: 1,
        ),
      ),
    );
    await pumpEventQueue();

    expect(
      c.read(activeSessionControllerProvider),
      const SessionStreaming(threadId: 'thr-B', agent: AgentName.codex),
    );
  });

  test('ThreadClosed on matching thread -> SessionSuspended', () async {
    final core = _FakeCore();
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    notifier.state = const SessionStreaming(
      threadId: 'thr-C',
      agent: AgentName.codex,
    );
    core.emit(
      _frame(
        threadId: 'thr-C',
        seq: BigInt.zero,
        ui: const UiEventMessage.threadClosed(
          threadId: 'thr-C',
          reason: ThreadEndReason.agentDone(),
          closedAtMs: 1,
        ),
      ),
    );
    await pumpEventQueue();

    expect(
      c.read(activeSessionControllerProvider),
      const SessionSuspended(threadId: 'thr-C', agent: AgentName.codex),
    );
  });

  test(
    'Error frame on matching thread -> SessionError with threadId',
    () async {
      final core = _FakeCore();
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      notifier.state = const SessionStreaming(
        threadId: 'thr-D',
        agent: AgentName.codex,
      );
      core.emit(
        _frame(
          threadId: 'thr-D',
          seq: BigInt.zero,
          ui: const UiEventMessage.error(code: 'agent_crash', message: 'boom'),
        ),
      );
      await pumpEventQueue();

      final st = c.read(activeSessionControllerProvider);
      expect(st, isA<SessionError>());
      expect((st as SessionError).threadId, 'thr-D');
    },
  );

  test('send() in AwaitingInput re-enters Streaming on success', () async {
    final core = _FakeCore();
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    notifier.state = const SessionAwaitingInput(
      threadId: 'thr-E',
      agent: AgentName.codex,
    );
    var dispatchCount = 0;

    final error = await notifier.send(
      agent: AgentName.codex,
      text: 'follow-up',
      dispatch: () async {
        dispatchCount += 1;
      },
    );

    expect(dispatchCount, 1);
    expect(error, isNull);
    expect(
      c.read(activeSessionControllerProvider),
      const SessionStreaming(threadId: 'thr-E', agent: AgentName.codex),
    );
  });

  test(
    'send() failure restores AwaitingInput instead of poisoning session',
    () async {
      final core = _FakeCore();
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      notifier.state = const SessionAwaitingInput(
        threadId: 'thr-send-fail',
        agent: AgentName.codex,
      );

      final error = await notifier.send(
        agent: AgentName.codex,
        text: 'follow-up',
        dispatch: () async => throw const MinosError.timeout(),
      );

      expect(error, isA<MinosError_Timeout>());
      expect(
        c.read(activeSessionControllerProvider),
        const SessionAwaitingInput(
          threadId: 'thr-send-fail',
          agent: AgentName.codex,
        ),
      );
    },
  );

  test('send() in Suspended resumes the known thread', () async {
    final core = _FakeCore();
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    notifier.state = const SessionSuspended(
      threadId: 'thr-existing',
      agent: AgentName.codex,
    );
    var dispatchCount = 0;

    final error = await notifier.send(
      agent: AgentName.codex,
      text: 'resume',
      dispatch: () async {
        dispatchCount += 1;
      },
    );

    expect(error, isNull);
    expect(dispatchCount, 1);
    expect(
      c.read(activeSessionControllerProvider),
      const SessionStreaming(threadId: 'thr-existing', agent: AgentName.codex),
    );
  });

  test(
    'stop() in Streaming calls interruptThread and transitions to Suspended',
    () async {
      final core = _FakeCore();
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      notifier.state = const SessionStreaming(
        threadId: 'thr-F',
        agent: AgentName.codex,
      );
      await notifier.stop();

      expect(core.interruptCount, 1);
      expect(core.lastInterruptThreadId, 'thr-F');
      expect(
        c.read(activeSessionControllerProvider),
        const SessionSuspended(threadId: 'thr-F', agent: AgentName.codex),
      );
    },
  );

  test('stop() failure preserves threadId in SessionError', () async {
    final core = _FakeCore()..interruptError = const MinosError.timeout();
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    notifier.state = const SessionStreaming(
      threadId: 'thr-stop-fail',
      agent: AgentName.codex,
    );
    await notifier.stop();

    expect(core.interruptCount, 0);
    expect(
      c.read(activeSessionControllerProvider),
      const SessionError(
        threadId: 'thr-stop-fail',
        error: MinosError.timeout(),
      ),
    );
  });

  test('stop() in Idle is a no-op', () async {
    final core = _FakeCore();
    final c = _container(core);
    await c.read(activeSessionControllerProvider.notifier).stop();
    expect(core.interruptCount, 0);
    expect(c.read(activeSessionControllerProvider), const SessionIdle());
  });

  // ---- Interrupt vs Close/Delete semantics (Task 11.5) ----

  group('interrupt vs close separation', () {
    test(
      'stop() calls interruptThread (not closeThread/deleteThread)',
      () async {
        final core = _FakeCore();
        final c = _container(core);
        final notifier = c.read(activeSessionControllerProvider.notifier);

        notifier.state = const SessionStreaming(
          threadId: 'thr-int',
          agent: AgentName.codex,
        );
        await notifier.stop();

        expect(core.interruptCount, 1);
        expect(core.lastInterruptThreadId, 'thr-int');
        expect(core.deleteCount, 0);
      },
    );

    test(
      'stop() in AwaitingInput calls interruptThread and transitions to Suspended',
      () async {
        final core = _FakeCore();
        final c = _container(core);
        final notifier = c.read(activeSessionControllerProvider.notifier);

        notifier.state = const SessionAwaitingInput(
          threadId: 'thr-await',
          agent: AgentName.claude,
        );
        await notifier.stop();

        expect(core.interruptCount, 1);
        expect(core.lastInterruptThreadId, 'thr-await');
        expect(
          c.read(activeSessionControllerProvider),
          const SessionSuspended(
            threadId: 'thr-await',
            agent: AgentName.claude,
          ),
        );
      },
    );

    test('interrupt preserves threadId and agent for later resume', () async {
      final core = _FakeCore();
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      notifier.state = const SessionStreaming(
        threadId: 'thr-preserve',
        agent: AgentName.gemini,
      );
      await notifier.stop();

      final suspended = c.read(activeSessionControllerProvider);
      expect(suspended, isA<SessionSuspended>());
      expect((suspended as SessionSuspended).threadId, 'thr-preserve');
      expect(suspended.agent, AgentName.gemini);
    });

    test(
      'after interrupt, send() resumes the same thread (not a new session)',
      () async {
        final core = _FakeCore();
        final c = _container(core);
        final notifier = c.read(activeSessionControllerProvider.notifier);

        // Simulate: streaming → interrupt → suspended → send follow-up
        notifier.state = const SessionStreaming(
          threadId: 'thr-resume',
          agent: AgentName.codex,
        );
        await notifier.stop();
        expect(
          c.read(activeSessionControllerProvider),
          const SessionSuspended(
            threadId: 'thr-resume',
            agent: AgentName.codex,
          ),
        );

        // Send from Suspended should reuse the same threadId
        final error = await notifier.send(
          agent: AgentName.codex,
          text: 'continue',
          dispatch: () async {},
        );

        expect(error, isNull);
        expect(
          c.read(activeSessionControllerProvider),
          const SessionStreaming(
            threadId: 'thr-resume',
            agent: AgentName.codex,
          ),
        );
      },
    );

    test(
      'deleteThread is a separate permanent close path (not called by stop)',
      () async {
        final core = _FakeCore();
        final c = _container(core);
        final notifier = c.read(activeSessionControllerProvider.notifier);

        notifier.state = const SessionStreaming(
          threadId: 'thr-del',
          agent: AgentName.codex,
        );

        // stop() should only call interruptThread
        await notifier.stop();
        expect(core.interruptCount, 1);
        expect(core.deleteCount, 0);

        // deleteThread is called directly by the UI (swipe-to-delete),
        // not through the controller's stop() method
        await core.deleteThread(threadId: 'thr-del');
        expect(core.deleteCount, 1);
        expect(core.lastDeleteThreadId, 'thr-del');
      },
    );

    test(
      'interrupt failure transitions to Error with threadId preserved',
      () async {
        final core = _FakeCore()..interruptError = const MinosError.timeout();
        final c = _container(core);
        final notifier = c.read(activeSessionControllerProvider.notifier);

        notifier.state = const SessionStreaming(
          threadId: 'thr-err',
          agent: AgentName.codex,
        );
        await notifier.stop();

        final st = c.read(activeSessionControllerProvider);
        expect(st, isA<SessionError>());
        expect((st as SessionError).threadId, 'thr-err');
        // deleteThread was never called
        expect(core.deleteCount, 0);
      },
    );

    test('stop() in Suspended or Error is a no-op (already paused)', () async {
      final core = _FakeCore();
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      notifier.state = const SessionSuspended(
        threadId: 'thr-already',
        agent: AgentName.codex,
      );
      await notifier.stop();

      expect(core.interruptCount, 0);
      expect(core.deleteCount, 0);
      expect(
        c.read(activeSessionControllerProvider),
        const SessionSuspended(threadId: 'thr-already', agent: AgentName.codex),
      );
    });
  });
}
