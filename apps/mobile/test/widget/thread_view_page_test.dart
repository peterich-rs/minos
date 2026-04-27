import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/presentation/widgets/chat/message_bubble.dart';
import 'package:minos/presentation/widgets/chat/streaming_text.dart';
import 'package:minos/presentation/widgets/chat/tool_call_card.dart';
import 'package:minos/src/rust/api/minos.dart';

Widget _wrap(Widget child, _FakeCore core) {
  return ProviderScope(
    overrides: [minosCoreProvider.overrideWithValue(core)],
    child: ShadApp(home: child),
  );
}

class _FakeCore implements MinosCoreProtocol {
  _FakeCore({this.uiEventsForThread = const []});

  List<UiEventMessage> uiEventsForThread;
  final StreamController<UiEventFrame> _uiCtl =
      StreamController<UiEventFrame>.broadcast();

  void emit(UiEventFrame f) => _uiCtl.add(f);

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) async {
    return ReadThreadResponse(uiEvents: uiEventsForThread);
  }

  @override
  Future<ListThreadsResponse> listThreads(ListThreadsParams params) async =>
      const ListThreadsResponse(threads: []);

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
  Stream<UiEventFrame> get uiEvents => _uiCtl.stream;

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
  Future<StartAgentResponse> startAgent({
    required AgentName agent,
    required String prompt,
  }) async => throw UnimplementedError();

  @override
  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) async {}

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
  // The streaming cursor inside `MessageBubble` runs an indefinite
  // `AnimationController.repeat`, so `pumpAndSettle` would deadlock.
  // We pump a short fixed duration to let async loaders resolve and
  // the first frame paint instead.
  Future<void> settle(WidgetTester tester) async {
    for (var i = 0; i < 5; i++) {
      await tester.pump(const Duration(milliseconds: 100));
    }
  }

  testWidgets(
    'New chat (threadId=null) shows empty state + InputBar with Send',
    (tester) async {
      final core = _FakeCore();
      await tester.pumpWidget(_wrap(const ThreadViewPage(), core));
      await settle(tester);

      expect(find.text('Start a new conversation'), findsOneWidget);
      expect(find.text('Send'), findsOneWidget);
      expect(find.byType(MessageBubble), findsNothing);
    },
  );

  testWidgets(
    'Renders user MessageBubble + assistant StreamingText for the event log',
    (tester) async {
      final core = _FakeCore(
        uiEventsForThread: const <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'u1',
            role: MessageRole.user,
            startedAtMs: 0,
          ),
          UiEventMessage.textDelta(messageId: 'u1', text: 'Hi'),
          UiEventMessage.messageCompleted(messageId: 'u1', finishedAtMs: 0),
          UiEventMessage.messageStarted(
            messageId: 'a1',
            role: MessageRole.assistant,
            startedAtMs: 0,
          ),
          UiEventMessage.textDelta(messageId: 'a1', text: 'Hel'),
          UiEventMessage.textDelta(messageId: 'a1', text: 'lo'),
        ],
      );

      await tester.pumpWidget(
        _wrap(const ThreadViewPage(threadId: 'thr1'), core),
      );
      await settle(tester);

      // Both bubbles materialised.
      expect(find.byType(MessageBubble), findsAtLeastNWidgets(2));
      expect(find.byType(StreamingText), findsOneWidget);

      // The assistant bubble is still streaming (no MessageCompleted yet).
      final streaming = tester.widget<StreamingText>(
        find.byType(StreamingText),
      );
      expect(streaming.accumulatedText, 'Hello');
      expect(streaming.isComplete, isFalse);
    },
  );

  testWidgets('MessageCompleted on assistant clears the streaming flag', (
    tester,
  ) async {
    final core = _FakeCore(
      uiEventsForThread: const <UiEventMessage>[
        UiEventMessage.messageStarted(
          messageId: 'a1',
          role: MessageRole.assistant,
          startedAtMs: 0,
        ),
        UiEventMessage.textDelta(messageId: 'a1', text: 'done'),
        UiEventMessage.messageCompleted(messageId: 'a1', finishedAtMs: 0),
      ],
    );

    await tester.pumpWidget(
      _wrap(const ThreadViewPage(threadId: 'thr1'), core),
    );
    await settle(tester);

    final streaming = tester.widget<StreamingText>(find.byType(StreamingText));
    expect(streaming.isComplete, isTrue);
  });

  testWidgets('ToolCallPlaced renders a ToolCallCard', (tester) async {
    final core = _FakeCore(
      uiEventsForThread: const <UiEventMessage>[
        UiEventMessage.messageStarted(
          messageId: 'a1',
          role: MessageRole.assistant,
          startedAtMs: 0,
        ),
        UiEventMessage.toolCallPlaced(
          messageId: 'a1',
          toolCallId: 'tc1',
          name: 'run_command',
          argsJson: '{"cmd":"ls"}',
        ),
      ],
    );

    await tester.pumpWidget(
      _wrap(const ThreadViewPage(threadId: 'thr1'), core),
    );
    await settle(tester);

    expect(find.byType(ToolCallCard), findsOneWidget);
    expect(find.text('run_command'), findsOneWidget);
    expect(find.text('running…'), findsOneWidget);
  });
}
