@Tags(['ffi'])
library;

import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/presentation/pages/login_page.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/src/rust/frb_generated.dart';

/// Hand-written fake mirroring the protocol surface used by `LoginPage`.
/// Returns a successful [AuthSummary] from `login`/`register` by default;
/// individual tests override the behaviour by setting [throwOnLogin] /
/// [throwOnRegister].
class _FakeCore implements MinosCoreProtocol {
  String? lastLoginEmail;
  String? lastLoginPassword;
  String? lastRegisterEmail;
  String? lastRegisterPassword;
  Object? throwOnLogin;
  Object? throwOnRegister;

  @override
  Future<AuthSummary> login(
      {required String email, required String password}) async {
    lastLoginEmail = email;
    lastLoginPassword = password;
    if (throwOnLogin != null) throw throwOnLogin!;
    return AuthSummary(accountId: 'acc-${email.hashCode}', email: email);
  }

  @override
  Future<AuthSummary> register(
      {required String email, required String password}) async {
    lastRegisterEmail = email;
    lastRegisterPassword = password;
    if (throwOnRegister != null) throw throwOnRegister!;
    return AuthSummary(accountId: 'acc-${email.hashCode}', email: email);
  }

  // The auth-state stream is the path Phase 8 uses to drive the
  // controller; LoginPage only reads `authControllerProvider.notifier` so
  // an empty stream is fine — the controller stays in `AuthBootstrapping`
  // and we never need to assert on the post-success state.
  @override
  Stream<AuthStateFrame> get authStates => const Stream<AuthStateFrame>.empty();

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
  Future<StartAgentResponse> startAgent(
          {required AgentName agent, required String prompt}) async =>
      throw UnimplementedError();

  @override
  Future<void> sendUserMessage(
      {required String sessionId, required String text}) async {}

  @override
  Future<void> stopAgent() async {}

  @override
  void notifyForegrounded() {}

  @override
  void notifyBackgrounded() {}
}

String _hostDylibPath() {
  final workspaceRoot = Directory.current.parent.parent.path;
  final String suffix;
  if (Platform.isMacOS) {
    suffix = 'dylib';
  } else if (Platform.isWindows) {
    suffix = 'dll';
  } else {
    suffix = 'so';
  }
  final release =
      File('$workspaceRoot/target/release/libminos_ffi_frb.$suffix');
  if (release.existsSync()) return release.path;
  return '$workspaceRoot/target/debug/libminos_ffi_frb.$suffix';
}

/// Build the widget under test with a hand-rolled [ProviderContainer] so
/// the riverpod_lint `scoped_providers_should_specify_dependencies` rule
/// (which fires for `ProviderScope.overrides` of `keepAlive` providers)
/// stays quiet without forcing us to re-annotate `authControllerProvider`.
Widget _harness(MinosCoreProtocol core, {MinosError? errorBanner}) {
  final container = ProviderContainer(
    overrides: [minosCoreProvider.overrideWithValue(core)],
  );
  addTearDown(container.dispose);
  return UncontrolledProviderScope(
    container: container,
    child: ShadApp(home: LoginPage(errorBanner: errorBanner)),
  );
}

void main() {
  setUpAll(() async {
    final path = _hostDylibPath();
    if (!File(path).existsSync()) {
      fail(
        'Missing host dylib at $path. Build it first via: '
        'cargo build -p minos-ffi-frb',
      );
    }
    await RustLib.init(externalLibrary: ExternalLibrary.open(path));
  });

  testWidgets('Submit valid login calls MinosCoreProtocol.login with the input',
      (tester) async {
    final core = _FakeCore();
    await tester.pumpWidget(_harness(core));

    final inputs = find.byType(ShadInput);
    await tester.enterText(inputs.at(0), 'user@example.com');
    await tester.enterText(inputs.at(1), 'hunter2hunter2');

    await tester.tap(find.widgetWithText(ShadButton, 'Log in'));
    await tester.pumpAndSettle();

    expect(core.lastLoginEmail, 'user@example.com');
    expect(core.lastLoginPassword, 'hunter2hunter2');
  });

  testWidgets(
      'Submit register that returns EmailTaken switches to login mode '
      'and shows the banner', (tester) async {
    final core = _FakeCore()
      ..throwOnRegister = const MinosError.emailTaken();
    await tester.pumpWidget(_harness(core));

    // Toggle into register mode.
    await tester.tap(find.text('Create account'));
    await tester.pump();
    expect(find.widgetWithText(ShadButton, 'Register'), findsOneWidget);

    // Fill in matching credentials and submit.
    final inputs = find.byType(ShadInput);
    await tester.enterText(inputs.at(0), 'taken@example.com');
    await tester.enterText(inputs.at(1), 'hunter2hunter2');
    await tester.enterText(inputs.at(2), 'hunter2hunter2');

    await tester.tap(find.widgetWithText(ShadButton, 'Register'));
    await tester.pumpAndSettle();

    // The form is back in login mode (Submit reads "Log in") AND the
    // destructive banner is visible with the localized EmailTaken copy.
    expect(find.widgetWithText(ShadButton, 'Log in'), findsOneWidget);
    expect(find.byType(ShadAlert), findsOneWidget);
    expect(
      find.text(const MinosError.emailTaken().userMessage()),
      findsOneWidget,
    );
  });

  testWidgets('Login error other than EmailTaken keeps mode and shows banner',
      (tester) async {
    final core = _FakeCore()
      ..throwOnLogin = const MinosError.invalidCredentials();
    await tester.pumpWidget(_harness(core));

    final inputs = find.byType(ShadInput);
    await tester.enterText(inputs.at(0), 'user@example.com');
    await tester.enterText(inputs.at(1), 'hunter2hunter2');

    await tester.tap(find.widgetWithText(ShadButton, 'Log in'));
    await tester.pumpAndSettle();

    // Stayed in login mode.
    expect(find.widgetWithText(ShadButton, 'Log in'), findsOneWidget);
    expect(find.byType(ShadAlert), findsOneWidget);
    expect(
      find.text(const MinosError.invalidCredentials().userMessage()),
      findsOneWidget,
    );
  });

  testWidgets('errorBanner ctor argument is shown on first build',
      (tester) async {
    final core = _FakeCore();
    await tester.pumpWidget(
      _harness(core, errorBanner: const MinosError.invalidCredentials()),
    );
    await tester.pump();

    expect(find.byType(ShadAlert), findsOneWidget);
  });
}
