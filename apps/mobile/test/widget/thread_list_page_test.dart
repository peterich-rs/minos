import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/presentation/pages/account_settings_page.dart';
import 'package:minos/presentation/pages/thread_list_page.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeCore implements MinosCoreProtocol {
  _FakeCore({
    required this.threads,
    this.connectionState = const ConnectionState.disconnected(),
  });

  final List<ThreadSummary> threads;
  final ConnectionState connectionState;

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
  Future<bool> hasPersistedPairing() async => false;

  @override
  Stream<ConnectionState> get connectionStates =>
      Stream<ConnectionState>.value(connectionState);

  @override
  Stream<UiEventFrame> get uiEvents => const Stream<UiEventFrame>.empty();

  @override
  ConnectionState get currentConnectionState => connectionState;

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
  Stream<AuthStateFrame> get authStates =>
      const Stream<AuthStateFrame>.empty();

  @override
  Future<void> resumePersistedSession() async {}
}

Widget _wrap(Widget child, _FakeCore core) {
  return ProviderScope(
    overrides: [minosCoreProvider.overrideWithValue(core)],
    child: ShadApp(home: child),
  );
}

void main() {
  testWidgets(
    'renders one ListTile per thread when connected (no offline banner)',
    (tester) async {
      final core = _FakeCore(
        connectionState: const ConnectionState.connected(),
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

      await tester.pumpWidget(_wrap(const ThreadListPage(), core));
      await tester.pumpAndSettle();

      expect(find.byType(ListTile), findsNWidgets(3));
      expect(find.text('Hello'), findsOneWidget);
      expect(find.text('<untitled>'), findsOneWidget);
      expect(find.text('Big'), findsOneWidget);
      expect(find.byIcon(Icons.lock), findsOneWidget);
      // No offline banner when connected.
      expect(find.text('Mac 已离线'), findsNothing);
      expect(find.byIcon(Icons.cloud_off_outlined), findsNothing);
    },
  );

  testWidgets('shows Mac-offline banner when disconnected', (tester) async {
    final core = _FakeCore(
      threads: const [],
      connectionState: const ConnectionState.disconnected(),
    );

    await tester.pumpWidget(_wrap(const ThreadListPage(), core));
    await tester.pumpAndSettle();

    expect(find.byIcon(Icons.cloud_off_outlined), findsOneWidget);
    expect(find.text('Mac 已离线'), findsOneWidget);
  });

  testWidgets('reconnecting state shows reconnect message in banner', (
    tester,
  ) async {
    final core = _FakeCore(
      threads: const [],
      connectionState: const ConnectionState.reconnecting(attempt: 3),
    );

    await tester.pumpWidget(_wrap(const ThreadListPage(), core));
    await tester.pumpAndSettle();

    expect(find.byIcon(Icons.cloud_off_outlined), findsOneWidget);
    expect(find.textContaining('重连'), findsOneWidget);
  });

  testWidgets('new-chat FAB pushes ThreadViewPage with no thread id', (
    tester,
  ) async {
    final core = _FakeCore(
      connectionState: const ConnectionState.connected(),
      threads: const [],
    );

    await tester.pumpWidget(_wrap(const ThreadListPage(), core));
    await tester.pumpAndSettle();

    expect(find.byType(FloatingActionButton), findsOneWidget);
    await tester.tap(find.byType(FloatingActionButton));
    await tester.pumpAndSettle();

    expect(find.byType(ThreadViewPage), findsOneWidget);
    final view = tester.widget<ThreadViewPage>(find.byType(ThreadViewPage));
    expect(view.threadId, isNull);
  });

  testWidgets('account-settings IconButton pushes AccountSettingsPage', (
    tester,
  ) async {
    final core = _FakeCore(
      connectionState: const ConnectionState.connected(),
      threads: const [],
    );

    await tester.pumpWidget(_wrap(const ThreadListPage(), core));
    await tester.pumpAndSettle();

    await tester.tap(find.byIcon(Icons.account_circle_outlined));
    await tester.pumpAndSettle();

    expect(find.byType(AccountSettingsPage), findsOneWidget);
  });

  testWidgets('empty list still shows placeholder', (tester) async {
    final core = _FakeCore(
      connectionState: const ConnectionState.connected(),
      threads: const [],
    );

    await tester.pumpWidget(_wrap(const ThreadListPage(), core));
    await tester.pumpAndSettle();

    expect(find.text('暂无会话'), findsOneWidget);
  });
}
