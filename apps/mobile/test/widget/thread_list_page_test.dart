import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/presentation/pages/thread_list_page.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeCore implements MinosCoreProtocol {
  _FakeCore({required this.threads});

  final List<ThreadSummary> threads;

  @override
  Future<ListThreadsResponse> listThreads(ListThreadsParams params) async {
    return ListThreadsResponse(threads: threads);
  }

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) async {
    return const ReadThreadResponse(uiEvents: []);
  }

  @override
  Future<void> pairWithQrJson(String qrJson) async {}

  @override
  Future<void> forgetPeer() async {}

  @override
  Stream<ConnectionState> get connectionStates =>
      const Stream<ConnectionState>.empty();

  @override
  Stream<UiEventFrame> get uiEvents => const Stream<UiEventFrame>.empty();

  @override
  ConnectionState get currentConnectionState =>
      const ConnectionState.disconnected();
}

void main() {
  testWidgets('ThreadListPage renders one ListTile per thread', (tester) async {
    final core = _FakeCore(
      threads: [
        const ThreadSummary(
          threadId: 'a',
          agent: AgentName.codex,
          title: 'Hello',
          firstTsMs: 0,
          lastTsMs: 0,
          messageCount: 3,
        ),
        const ThreadSummary(
          threadId: 'b',
          agent: AgentName.codex,
          firstTsMs: 0,
          lastTsMs: 0,
          messageCount: 1,
        ),
        const ThreadSummary(
          threadId: 'c',
          agent: AgentName.claude,
          title: 'Big',
          firstTsMs: 0,
          lastTsMs: 0,
          messageCount: 8,
          endedAtMs: 99,
          endReason: ThreadEndReason.agentDone(),
        ),
      ],
    );

    await tester.pumpWidget(
      ProviderScope(
        overrides: [minosCoreProvider.overrideWithValue(core)],
        child: const MaterialApp(home: ThreadListPage()),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byType(ListTile), findsNWidgets(3));
    expect(find.text('Hello'), findsOneWidget);
    expect(find.text('<untitled>'), findsOneWidget);
    expect(find.text('Big'), findsOneWidget);
    expect(find.byIcon(Icons.lock), findsOneWidget);
  });

  testWidgets('ThreadListPage empty list shows placeholder', (tester) async {
    final core = _FakeCore(threads: const []);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [minosCoreProvider.overrideWithValue(core)],
        child: const MaterialApp(home: ThreadListPage()),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('暂无会话'), findsOneWidget);
  });
}
