import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/flutter_log.dart';
import 'package:minos/application/ui_state_providers.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/src/rust/api/minos.dart' show MinosError;
import 'package:minos/ui/features/auth/widgets/auth_error_banner.dart';
import 'package:minos/ui/features/auth/widgets/auth_form.dart';

/// Email + password login / register surface. Owns the local mode toggle,
/// in-flight flag and most-recent error so the form widget can stay
/// stateless re: side effects.
///
/// On `EmailTaken` while in register mode we automatically flip to login —
/// the user almost certainly just forgot they already have an account, and
/// re-entering the same email + password is the right next step. Other
/// errors stay in the current mode and surface only the destructive
/// banner.
///
/// Wired from the GoRouter for `RootRoute.login`.
/// Reads the auth state directly to surface any `AuthRefreshFailed` error
/// as the initial banner.
class LoginPage extends ConsumerStatefulWidget {
  const LoginPage({super.key});

  @override
  ConsumerState<LoginPage> createState() => _LoginPageState();
}

class _LoginPageState extends ConsumerState<LoginPage> {
  @override
  void initState() {
    super.initState();
    // Surface any carry-over error from AuthRefreshFailed.
    final authState = ref.read(authControllerProvider);
    ref
        .read(loginPageStateControllerProvider.notifier)
        .seedInitialError(
          authState is AuthRefreshFailed ? authState.error : null,
        );
  }

  Future<void> _submit(String email, String password) async {
    final controller = ref.read(loginPageStateControllerProvider.notifier);
    final currentState = ref.read(loginPageStateControllerProvider);
    controller.startSubmitting();

    var nextMode = currentState.mode;
    Object? nextError;
    var clearError = false;

    try {
      final notifier = ref.read(authControllerProvider.notifier);
      if (currentState.mode == .login) {
        await notifier.login(email, password);
      } else {
        await notifier.register(email, password);
      }
      clearError = true;
    } on MinosError catch (e) {
      // EmailTaken in register mode is the one auto-mode-switch we do —
      // see class doc.
      if (e.kind == .emailTaken && currentState.mode == .register) {
        nextMode = .login;
        nextError = e;
      } else {
        nextError = e;
      }
    } catch (e, st) {
      logFlutterError(
        'login_page',
        'unexpected auth submit failure mode=${currentState.mode}',
        error: e,
        stackTrace: st,
      );
      nextError = e;
    } finally {}

    if (!mounted) return;
    controller.finishSubmitting(
      mode: nextMode,
      clearError: clearError,
      error: nextError,
    );
  }

  @override
  Widget build(BuildContext context) {
    final pageState = ref.watch(loginPageStateControllerProvider);
    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const .all(24),
          child: Column(
            crossAxisAlignment: .stretch,
            children: <Widget>[
              const SizedBox(height: 48),
              const Text(
                'Minos',
                textAlign: .center,
                style: TextStyle(fontSize: 32, fontWeight: .bold),
              ),
              const SizedBox(height: 32),
              AuthErrorBanner(error: pageState.error),
              const SizedBox(height: 16),
              AuthForm(
                mode: _toAuthMode(pageState.mode),
                onModeChanged: pageState.inFlight
                    ? (_) {}
                    : (m) => ref
                          .read(loginPageStateControllerProvider.notifier)
                          .setMode(_fromAuthMode(m)),
                onSubmit: _submit,
                inFlight: pageState.inFlight,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

AuthMode _toAuthMode(LoginPageMode mode) {
  return switch (mode) {
    .login => AuthMode.login,
    .register => AuthMode.register,
  };
}

LoginPageMode _fromAuthMode(AuthMode mode) {
  return switch (mode) {
    .login => LoginPageMode.login,
    .register => LoginPageMode.register,
  };
}
