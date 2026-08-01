import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/flutter_log.dart';
import 'package:minos/application/ui_state_providers.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/src/rust/api/minos.dart' show ErrorKind, MinosError;
import 'package:minos/ui/features/auth/widgets/auth_error_banner.dart';
import 'package:minos/ui/features/auth/widgets/auth_form.dart';
import 'package:minos/ui/theme/theme.dart';

/// Clean mobile auth surface (login / register).
class LoginPage extends ConsumerStatefulWidget {
  const LoginPage({super.key});

  @override
  ConsumerState<LoginPage> createState() => _LoginPageState();
}

class _LoginPageState extends ConsumerState<LoginPage> {
  @override
  void initState() {
    super.initState();
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
      if (currentState.mode == LoginPageMode.login) {
        await notifier.login(email, password);
      } else {
        await notifier.register(email, password);
      }
      clearError = true;
    } on MinosError catch (e) {
      if (e.kind == ErrorKind.emailTaken &&
          currentState.mode == LoginPageMode.register) {
        nextMode = LoginPageMode.login;
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
    }

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
    final colors = context.minosColors;
    final theme = Theme.of(context);

    return Scaffold(
      backgroundColor: colors.canvas,
      body: SafeArea(
        child: LayoutBuilder(
          builder: (context, constraints) {
            return SingleChildScrollView(
              padding: const EdgeInsets.symmetric(
                horizontal: MinosSpacing.xxl,
                vertical: MinosSpacing.xxl,
              ),
              child: ConstrainedBox(
                constraints: BoxConstraints(
                  minHeight: constraints.maxHeight - 48,
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: <Widget>[
                    const SizedBox(height: MinosSpacing.huge),
                    Text(
                      'Minos',
                      textAlign: TextAlign.center,
                      style: theme.textTheme.displayLarge,
                    ),
                    const SizedBox(height: MinosSpacing.sm),
                    Text(
                      pageState.mode == LoginPageMode.register
                          ? '创建账号以查看 Linked Host 与会话'
                          : '登录后远程驱动你的 Mac',
                      textAlign: TextAlign.center,
                      style: theme.textTheme.bodyMedium?.copyWith(
                        color: colors.textSecondary,
                      ),
                    ),
                    const SizedBox(height: MinosSpacing.xxxl),
                    AuthErrorBanner(error: pageState.error),
                    const SizedBox(height: MinosSpacing.md),
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
            );
          },
        ),
      ),
    );
  }
}

AuthMode _toAuthMode(LoginPageMode mode) {
  return switch (mode) {
    LoginPageMode.login => AuthMode.login,
    LoginPageMode.register => AuthMode.register,
  };
}

LoginPageMode _fromAuthMode(AuthMode mode) {
  return switch (mode) {
    AuthMode.login => LoginPageMode.login,
    AuthMode.register => LoginPageMode.register,
  };
}
