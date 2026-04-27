import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeCore implements MinosCoreProtocol {
  _FakeCore({required this.uiEventsForThread});

  final List<UiEventMessage> uiEventsForThread;

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) async {
    return ReadThreadResponse(uiEvents: uiEventsForThread);
  }

  @override
  Future<ListThreadsResponse> listThreads(ListThreadsParams params) async {
    return const ListThreadsResponse(threads: []);
  }

  @override
  Future<void> pairWithQrJson(String qrJson) async {}

  @override
  Future<void> forgetPeer() async {}

  @override
  Future<bool> hasPersistedPairing() async => false;

  @override
  Stream<ConnectionState> get connectionStates =>
      const Stream<ConnectionState>.empty();

  @override
  Stream<UiEventFrame> get uiEvents => const Stream<UiEventFrame>.empty();

  @override
  ConnectionState get currentConnectionState =>
      const ConnectionState.disconnected();

  @override
  Future<AuthSummary> register({required String email, required String password}) async =>
      throw UnimplementedError();

  @override
  Future<AuthSummary> login({required String email, required String password}) async =>
      throw UnimplementedError();

  @override
  Future<void> refreshSession() async {}

  @override
  Future<void> logout() async {}

  @override
  Future<StartAgentResponse> startAgent({required AgentName agent, required String prompt}) async =>
      throw UnimplementedError();

  @override
  Future<void> sendUserMessage({required String sessionId, required String text}) async {}

  @override
  Future<void> stopAgent() async {}

  @override
  void notifyForegrounded() {}

  @override
  void notifyBackgrounded() {}

  @override
  Stream<AuthStateFrame> get authStates => const Stream<AuthStateFrame>.empty();

  @override
  Future<void> resumePersistedSession() async {}
}

void main() {
  testWidgets('ThreadViewPage renders one tile per UiEventMessage', (
    tester,
  ) async {
    final events = <UiEventMessage>[
      const UiEventMessage.threadOpened(
        threadId: 'thr1',
        agent: AgentName.codex,
        openedAtMs: 0,
      ),
      const UiEventMessage.messageStarted(
        messageId: 'm1',
        role: MessageRole.assistant,
        startedAtMs: 0,
      ),
      const UiEventMessage.textDelta(messageId: 'm1', text: 'Hel'),
      const UiEventMessage.textDelta(messageId: 'm1', text: 'lo'),
      const UiEventMessage.toolCallPlaced(
        messageId: 'm1',
        toolCallId: 'tc1',
        name: 'run_command',
        argsJson: '{}',
      ),
      const UiEventMessage.toolCallCompleted(
        toolCallId: 'tc1',
        output: 'ok',
        isError: false,
      ),
      const UiEventMessage.messageCompleted(messageId: 'm1', finishedAtMs: 0),
      const UiEventMessage.reasoningDelta(messageId: 'm1', text: 'why'),
      const UiEventMessage.threadTitleUpdated(threadId: 'thr1', title: 'chat'),
      const UiEventMessage.threadClosed(
        threadId: 'thr1',
        reason: ThreadEndReason.agentDone(),
        closedAtMs: 0,
      ),
    ];

    final core = _FakeCore(uiEventsForThread: events);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [minosCoreProvider.overrideWithValue(core)],
        child: const MaterialApp(home: ThreadViewPage(threadId: 'thr1')),
      ),
    );
    await tester.pumpAndSettle();

    // ListView.builder only materializes visible tiles, but the first
    // several are always on screen. We assert a floor on tile count and
    // presence of several kinds of events.
    expect(find.byType(ListTile), findsAtLeastNWidgets(4));
    expect(find.text('ThreadOpened'), findsOneWidget);
    expect(find.text('MessageStarted'), findsOneWidget);
    expect(find.text('TextDelta'), findsAtLeastNWidgets(1));
  });

  testWidgets('ThreadViewPage shows empty placeholder when no events', (
    tester,
  ) async {
    final core = _FakeCore(uiEventsForThread: const []);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [minosCoreProvider.overrideWithValue(core)],
        child: const MaterialApp(home: ThreadViewPage(threadId: 'thr_empty')),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('暂无事件'), findsOneWidget);
  });
}
