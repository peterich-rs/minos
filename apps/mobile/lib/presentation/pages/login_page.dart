import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/auth_provider.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/presentation/widgets/auth/auth_error_banner.dart';
import 'package:minos/presentation/widgets/auth/auth_form.dart';
import 'package:minos/src/rust/api/minos.dart' show ErrorKind, MinosError;

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
  AuthMode _mode = AuthMode.login;
  bool _inFlight = false;
  Object? _error;

  @override
  void initState() {
    super.initState();
    // Surface any carry-over error from AuthRefreshFailed.
    final authState = ref.read(authControllerProvider);
    if (authState is AuthRefreshFailed) {
      _error = authState.error;
    }
  }

  Future<void> _submit(String email, String password) async {
    setState(() {
      _inFlight = true;
    });

    var nextMode = _mode;
    Object? nextError;
    var clearError = false;

    try {
      final notifier = ref.read(authControllerProvider.notifier);
      if (_mode == AuthMode.login) {
        await notifier.login(email, password);
      } else {
        await notifier.register(email, password);
      }
      clearError = true;
    } on MinosError catch (e) {
      // EmailTaken in register mode is the one auto-mode-switch we do —
      // see class doc.
      if (e.kind == ErrorKind.emailTaken && _mode == AuthMode.register) {
        nextMode = AuthMode.login;
        nextError = e;
      } else {
        nextError = e;
      }
    } catch (e, st) {
      debugPrint('auth submit failed unexpectedly: $e\n$st');
      nextError = e;
    } finally {
      if (mounted) {
        setState(() {
          _inFlight = false;
          _mode = nextMode;
          if (clearError) {
            _error = null;
          } else if (nextError != null) {
            _error = nextError;
          }
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: <Widget>[
              const SizedBox(height: 48),
              const Text(
                'Minos',
                textAlign: TextAlign.center,
                style: TextStyle(fontSize: 32, fontWeight: FontWeight.bold),
              ),
              const SizedBox(height: 32),
              AuthErrorBanner(error: _error),
              const SizedBox(height: 16),
              AuthForm(
                mode: _mode,
                onModeChanged: _inFlight
                    ? (_) {}
                    : (m) => setState(() => _mode = m),
                onSubmit: _submit,
                inFlight: _inFlight,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
