import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Hand-written fake implementing [MinosCoreProtocol]. Emits auth frames
/// via the [emit] helper so the test can drive the controller through
/// each state.
class _FakeCore implements MinosCoreProtocol {
  final _authCtl = StreamController<AuthStateFrame>.broadcast();

  String? lastRegisterEmail;
  String? lastRegisterPassword;
  String? lastLoginEmail;
  String? lastLoginPassword;
  int logoutCount = 0;
  int resumeCount = 0;

  void emit(AuthStateFrame frame) => _authCtl.add(frame);

  @override
  Stream<AuthStateFrame> get authStates => _authCtl.stream;

  @override
  Future<AuthSummary> register({
    required String email,
    required String password,
  }) async {
    lastRegisterEmail = email;
    lastRegisterPassword = password;
    return AuthSummary(accountId: 'acc-${email.hashCode}', email: email);
  }

  @override
  Future<AuthSummary> login({
    required String email,
    required String password,
  }) async {
    lastLoginEmail = email;
    lastLoginPassword = password;
    return AuthSummary(accountId: 'acc-${email.hashCode}', email: email);
  }

  @override
  Future<void> refreshSession() async {}

  @override
  Future<void> logout() async {
    logoutCount += 1;
  }

  @override
  Future<void> pairWithQrJson(String qrJson) async {}

  @override
  Future<void> forgetMac(String macDeviceId) async {}

  @override
  Future<List<MacSummaryDto>> listPairedMacs() async => const <MacSummaryDto>[];

  @override
  Future<String?> activeMac() async => null;

  @override
  Future<void> setActiveMac(String macDeviceId) async {}

  @override
  Future<bool> hasPersistedPairing() async => false;

  @override
  Future<String?> peerDisplayName() async => null;

  @override
  Future<void> setPeerDisplayName(String? name) async {}

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
  Future<List<AgentDescriptor>> listClis() async => const [];

  @override
  void notifyForegrounded() {}

  @override
  void notifyBackgrounded() {}

  @override
  Future<void> resumePersistedSession() async {
    resumeCount += 1;
  }
}

void main() {
  late _FakeCore core;
  late ProviderContainer container;

  setUp(() {
    core = _FakeCore();
    container = ProviderContainer(
      overrides: [minosCoreProvider.overrideWithValue(core)],
    );
    addTearDown(container.dispose);
  });

  test('initial build state is AuthBootstrapping', () {
    expect(container.read(authControllerProvider), const AuthBootstrapping());
  });

  test(
    'Authenticated frame transitions controller to AuthAuthenticated',
    () async {
      container.read(authControllerProvider);
      core.emit(
        const AuthStateFrame.authenticated(
          account: AuthSummary(accountId: 'acc-1', email: 'a@b.test'),
        ),
      );
      await pumpEventQueue();
      final state = container.read(authControllerProvider);
      expect(state, isA<AuthAuthenticated>());
      expect((state as AuthAuthenticated).account.accountId, 'acc-1');
    },
  );

  test(
    'Unauthenticated frame transitions controller to AuthUnauthenticated',
    () async {
      container.read(authControllerProvider);
      core.emit(const AuthStateFrame.unauthenticated());
      await pumpEventQueue();
      expect(
        container.read(authControllerProvider),
        const AuthUnauthenticated(),
      );
    },
  );

  test('Refreshing then RefreshFailed surfaces typed MinosError', () async {
    container.read(authControllerProvider);
    core.emit(const AuthStateFrame.refreshing());
    await pumpEventQueue();
    expect(container.read(authControllerProvider), const AuthRefreshing());
    core.emit(
      const AuthStateFrame.refreshFailed(
        error: MinosError.invalidCredentials(),
      ),
    );
    await pumpEventQueue();
    final state = container.read(authControllerProvider);
    expect(state, isA<AuthRefreshFailed>());
    expect(
      (state as AuthRefreshFailed).error,
      const MinosError.invalidCredentials(),
    );
  });

  test(
    'first Authenticated frame triggers resumePersistedSession exactly once',
    () async {
      container.read(authControllerProvider);
      expect(core.resumeCount, 0);

      core.emit(
        const AuthStateFrame.authenticated(
          account: AuthSummary(accountId: 'a1', email: 'x@y.test'),
        ),
      );
      await pumpEventQueue();
      expect(core.resumeCount, 1);

      // A second Authenticated for the same session must not double-resume.
      core.emit(
        const AuthStateFrame.authenticated(
          account: AuthSummary(accountId: 'a1', email: 'x2@y.test'),
        ),
      );
      await pumpEventQueue();
      expect(core.resumeCount, 1);

      // After logout (Unauthenticated), the next Authenticated re-arms.
      core.emit(const AuthStateFrame.unauthenticated());
      await pumpEventQueue();
      core.emit(
        const AuthStateFrame.authenticated(
          account: AuthSummary(accountId: 'a2', email: 'z@y.test'),
        ),
      );
      await pumpEventQueue();
      expect(core.resumeCount, 2);
    },
  );

  test('register/login/logout call through to MinosCoreProtocol', () async {
    final notifier = container.read(authControllerProvider.notifier);

    await notifier.register('new@x.test', 'hunter2hunter2');
    expect(core.lastRegisterEmail, 'new@x.test');
    expect(core.lastRegisterPassword, 'hunter2hunter2');

    await notifier.login('back@x.test', 'pwpwpwpw');
    expect(core.lastLoginEmail, 'back@x.test');
    expect(core.lastLoginPassword, 'pwpwpwpw');

    await notifier.logout();
    expect(core.logoutCount, 1);
  });
}
