import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/presentation/pages/account_settings_page.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Hand-written fake exposing the surface AccountSettingsPage touches:
/// the auth-state stream (for the email subtitle) and the `logout`
/// forwarder (for the destructive entry).
class _FakeCore implements MinosCoreProtocol {
  _FakeCore({this.initialFrame});

  final AuthStateFrame? initialFrame;
  final _authCtl = StreamController<AuthStateFrame>.broadcast();
  int logoutCount = 0;

  void emit(AuthStateFrame frame) => _authCtl.add(frame);

  @override
  Stream<AuthStateFrame> get authStates async* {
    if (initialFrame != null) yield initialFrame!;
    yield* _authCtl.stream;
  }

  @override
  Future<void> logout() async {
    logoutCount += 1;
  }

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

  @override
  void notifyForegrounded() {}

  @override
  void notifyBackgrounded() {}
}

Widget _harness(MinosCoreProtocol core) {
  final container = ProviderContainer(
    overrides: [minosCoreProvider.overrideWithValue(core)],
  );
  addTearDown(container.dispose);
  return UncontrolledProviderScope(
    container: container,
    child: ShadApp(
      home: Builder(
        builder: (ctx) => Scaffold(
          body: Center(
            child: ElevatedButton(
              onPressed: () => Navigator.of(ctx).push(
                MaterialPageRoute<void>(
                  builder: (_) => const AccountSettingsPage(),
                ),
              ),
              child: const Text('open'),
            ),
          ),
        ),
      ),
    ),
  );
}

void main() {
  testWidgets('renders the authenticated email + version', (tester) async {
    final core = _FakeCore(
      initialFrame: const AuthStateFrame.authenticated(
        account: AuthSummary(accountId: 'acc-1', email: 'user@example.com'),
      ),
    );
    await tester.pumpWidget(_harness(core));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    expect(find.text('邮箱'), findsOneWidget);
    expect(find.text('user@example.com'), findsOneWidget);
    expect(find.text('版本'), findsOneWidget);
    expect(find.text('退出登录'), findsOneWidget);
  });

  testWidgets('logout tile invokes core.logout and pops the page',
      (tester) async {
    final core = _FakeCore(
      initialFrame: const AuthStateFrame.authenticated(
        account: AuthSummary(accountId: 'acc-1', email: 'user@example.com'),
      ),
    );
    await tester.pumpWidget(_harness(core));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    expect(find.byType(AccountSettingsPage), findsOneWidget);
    await tester.tap(find.text('退出登录'));
    await tester.pumpAndSettle();

    expect(core.logoutCount, 1);
    // Page popped → root harness is back.
    expect(find.byType(AccountSettingsPage), findsNothing);
    expect(find.text('open'), findsOneWidget);
  });

  testWidgets('email column is em-dash before any Authenticated frame',
      (tester) async {
    final core = _FakeCore();
    await tester.pumpWidget(_harness(core));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    // Bootstrapping state → no account.
    expect(find.text('—'), findsOneWidget);
  });
}
