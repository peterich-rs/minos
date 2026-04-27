import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/presentation/app.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Counts the lifecycle forwarder calls so the test can assert the
/// `WidgetsBindingObserver.didChangeAppLifecycleState` bridge wired in
/// Phase 11.2 fires on the right transitions.
class _FakeCore implements MinosCoreProtocol {
  int foregroundedCount = 0;
  int backgroundedCount = 0;

  @override
  void notifyForegrounded() => foregroundedCount += 1;

  @override
  void notifyBackgrounded() => backgroundedCount += 1;

  @override
  Stream<AuthStateFrame> get authStates => const Stream<AuthStateFrame>.empty();

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
  Future<void> resumePersistedSession() async {}

  @override
  Future<void> pairWithQrJson(String qrJson) async {}

  @override
  Future<void> forgetPeer() async {}

  @override
  Future<bool> hasPersistedPairing() async => false;

  @override
  Future<ListThreadsResponse> listThreads(ListThreadsParams params) async =>
      const ListThreadsResponse(threads: []);

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) async =>
      const ReadThreadResponse(uiEvents: []);

  @override
  Stream<ConnectionState> get connectionStates =>
      const Stream<ConnectionState>.empty();

  @override
  Stream<UiEventFrame> get uiEvents => const Stream<UiEventFrame>.empty();

  @override
  ConnectionState get currentConnectionState =>
      const ConnectionState.disconnected();

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
}

void main() {
  testWidgets('AppLifecycleState.resumed → notifyForegrounded',
      (tester) async {
    final core = _FakeCore();
    final container = ProviderContainer(
      overrides: [minosCoreProvider.overrideWithValue(core)],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MinosApp(),
      ),
    );
    // Prime the splash render so the observer is registered.
    await tester.pump();

    final binding = WidgetsBinding.instance as TestWidgetsFlutterBinding;
    binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);

    expect(core.foregroundedCount, 1);
    expect(core.backgroundedCount, 0);
  });

  testWidgets(
      'paused / inactive / detached / hidden → notifyBackgrounded',
      (tester) async {
    final core = _FakeCore();
    final container = ProviderContainer(
      overrides: [minosCoreProvider.overrideWithValue(core)],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MinosApp(),
      ),
    );
    await tester.pump();

    final binding = WidgetsBinding.instance as TestWidgetsFlutterBinding;
    binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    binding.handleAppLifecycleStateChanged(AppLifecycleState.detached);
    binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);

    expect(core.backgroundedCount, 4);
    expect(core.foregroundedCount, 0);
  });
}
