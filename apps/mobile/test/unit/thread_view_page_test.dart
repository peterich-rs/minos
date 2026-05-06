import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/presentation/widgets/chat/input_bar.dart';
import 'package:minos/presentation/widgets/chat/reasoning_section.dart';
import 'package:minos/presentation/widgets/chat/tool_call_card.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeThreadCore implements MinosCoreProtocol {
  _FakeThreadCore({required this.initialEvents, this.echoSentMessages = false});

  final List<UiEventMessage> initialEvents;
  final bool echoSentMessages;
  final _uiCtl = StreamController<UiEventFrame>.broadcast();
  var _nextSeq = BigInt.one;

  BigInt _takeSeq() {
    final seq = _nextSeq;
    _nextSeq += BigInt.one;
    return seq;
  }

  void emit(UiEventFrame frame) {
    if (frame.seq >= _nextSeq) {
      _nextSeq = frame.seq + BigInt.one;
    }
    _uiCtl.add(frame);
  }

  Future<void> dispose() async {
    await _uiCtl.close();
  }

  @override
  Stream<UiEventFrame> get uiEvents => _uiCtl.stream;

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) async {
    return ReadThreadResponse(uiEvents: initialEvents);
  }

  @override
  Future<StartAgentResponse> startAgent({
    required AgentName agent,
    required String prompt,
  }) async {
    return const StartAgentResponse(sessionId: 'thr-1', cwd: '/tmp');
  }

  @override
  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) async {
    if (!echoSentMessages) return;
    final messageId = 'echo-${_nextSeq.toInt()}';
    emit(
      UiEventFrame(
        threadId: sessionId,
        seq: _takeSeq(),
        ui: UiEventMessage.messageStarted(
          messageId: messageId,
          role: MessageRole.user,
          startedAtMs: 1,
        ),
        tsMs: 1,
      ),
    );
    emit(
      UiEventFrame(
        threadId: sessionId,
        seq: _takeSeq(),
        ui: UiEventMessage.textDelta(messageId: messageId, text: text),
        tsMs: 1,
      ),
    );
  }

  @override
  Future<void> closeThread({required String threadId}) async {}

  @override
  Future<FriendRequestSummary> acceptFriendRequest({
    required String requestId,
  }) async => throw UnimplementedError();

  @override
  Future<List<AgentDescriptor>> listClis() async => const <AgentDescriptor>[];

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
  void notifyForegrounded() {}

  @override
  void notifyBackgrounded() {}

  @override
  Future<void> resumePersistedSession() async {}

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
  }) async => throw UnimplementedError();

  @override
  Future<MyProfileResponse> setMinosId({required String minosId}) async =>
      MyProfileResponse(
        accountId: 'acc',
        email: 'test@example.com',
        minosId: minosId,
      );
}

Future<ProviderContainer> _pumpThreadPage(
  WidgetTester tester,
  _FakeThreadCore core,
) async {
  final container = ProviderContainer(
    overrides: [minosCoreProvider.overrideWithValue(core)],
  );
  addTearDown(() async {
    await core.dispose();
    container.dispose();
  });

  await container
      .read(activeSessionControllerProvider.notifier)
      .start(agent: AgentName.codex, prompt: 'hello');

  await tester.pumpWidget(
    UncontrolledProviderScope(
      container: container,
      child: const ShadApp(
        home: ThreadViewPage(threadId: 'thr-1', agent: AgentName.codex),
      ),
    ),
  );
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 120));
  return container;
}

void main() {
  testWidgets('only the newest assistant message owns the live cursor', (
    tester,
  ) async {
    final core = _FakeThreadCore(
      initialEvents: const <UiEventMessage>[
        UiEventMessage.messageStarted(
          messageId: 'assistant-1',
          role: MessageRole.assistant,
          startedAtMs: 0,
        ),
        UiEventMessage.textDelta(messageId: 'assistant-1', text: 'First reply'),
        UiEventMessage.messageStarted(
          messageId: 'assistant-2',
          role: MessageRole.assistant,
          startedAtMs: 1,
        ),
        UiEventMessage.textDelta(
          messageId: 'assistant-2',
          text: 'Second reply',
        ),
      ],
    );

    await _pumpThreadPage(tester, core);

    expect(find.text('First reply'), findsOneWidget);
    expect(find.text('Second reply'), findsOneWidget);
    expect(
      find.byKey(const ValueKey<String>('streaming-cursor')),
      findsOneWidget,
    );
  });

  testWidgets('live assistant message shows compact activity lines', (
    tester,
  ) async {
    final core = _FakeThreadCore(
      initialEvents: const <UiEventMessage>[
        UiEventMessage.messageStarted(
          messageId: 'assistant-1',
          role: MessageRole.assistant,
          startedAtMs: 0,
        ),
        UiEventMessage.textDelta(
          messageId: 'assistant-1',
          text: 'I am checking that now.',
        ),
        UiEventMessage.reasoningDelta(
          messageId: 'assistant-1',
          text: 'Inspecting the available workspace state',
        ),
        UiEventMessage.toolCallPlaced(
          messageId: 'assistant-1',
          toolCallId: 'tool-1',
          name: 'run_in_terminal',
          argsJson: '{"command":"git status"}',
        ),
      ],
    );

    await _pumpThreadPage(tester, core);

    expect(find.textContaining('思考中'), findsOneWidget);
    expect(find.textContaining('调用工具 · run_in_terminal'), findsOneWidget);
    expect(find.byType(ReasoningSection), findsNothing);
    expect(find.byType(ToolCallCard), findsNothing);
  });

  testWidgets('cursor disappears when the session closes', (tester) async {
    final core = _FakeThreadCore(
      initialEvents: const <UiEventMessage>[
        UiEventMessage.messageStarted(
          messageId: 'assistant-1',
          role: MessageRole.assistant,
          startedAtMs: 0,
        ),
        UiEventMessage.textDelta(
          messageId: 'assistant-1',
          text: 'Wrapping up now',
        ),
      ],
    );

    await _pumpThreadPage(tester, core);
    expect(
      find.byKey(const ValueKey<String>('streaming-cursor')),
      findsOneWidget,
    );

    core.emit(
      UiEventFrame(
        threadId: 'thr-1',
        seq: BigInt.one,
        ui: const UiEventMessage.threadClosed(
          threadId: 'thr-1',
          reason: ThreadEndReason.agentDone(),
          closedAtMs: 1,
        ),
        tsMs: 1,
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 120));

    expect(
      find.byKey(const ValueKey<String>('streaming-cursor')),
      findsNothing,
    );
  });

  testWidgets('successful send collapses optimistic and echoed user rows', (
    tester,
  ) async {
    final core = _FakeThreadCore(
      echoSentMessages: true,
      initialEvents: const <UiEventMessage>[
        UiEventMessage.messageStarted(
          messageId: 'assistant-1',
          role: MessageRole.assistant,
          startedAtMs: 0,
        ),
        UiEventMessage.textDelta(messageId: 'assistant-1', text: 'Ready'),
      ],
    );

    await _pumpThreadPage(tester, core);
    core.emit(
      UiEventFrame(
        threadId: 'thr-1',
        seq: BigInt.one,
        ui: const UiEventMessage.messageCompleted(
          messageId: 'assistant-1',
          finishedAtMs: 1,
        ),
        tsMs: 1,
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 120));

    await tester.enterText(
      find.descendant(
        of: find.byType(InputBar),
        matching: find.byType(EditableText),
      ),
      'hello duplicate',
    );
    await tester.tap(find.byType(ShadIconButton));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 120));

    expect(find.text('hello duplicate'), findsOneWidget);
  });
}
