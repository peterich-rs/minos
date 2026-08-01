import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/ui_state_providers.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart' show MinosError;

/// Fixed auth surface so [LoginPageStateController] can read without FRB.
class _FixedAuthController extends AuthController {
  _FixedAuthController(this.fixed);

  final AuthState fixed;

  @override
  AuthState build() => fixed;
}

void main() {
  group('LoginPageStateController', () {
    test('seeds banner error from AuthRefreshFailed inside build()', () {
      const error = MinosError.unauthorized(reason: 'refresh expired');
      final container = ProviderContainer(
        overrides: [
          authControllerProvider.overrideWith(
            () => _FixedAuthController(const AuthRefreshFailed(error)),
          ),
        ],
      );
      addTearDown(container.dispose);

      final state = container.read(loginPageStateControllerProvider);

      expect(state.error, same(error));
      expect(state.mode, LoginPageMode.login);
      expect(state.inFlight, isFalse);
    });

    test('leaves banner empty when auth is unauthenticated', () {
      final container = ProviderContainer(
        overrides: [
          authControllerProvider.overrideWith(
            () => _FixedAuthController(const AuthUnauthenticated()),
          ),
        ],
      );
      addTearDown(container.dispose);

      final state = container.read(loginPageStateControllerProvider);

      expect(state.error, isNull);
    });

    test('finishSubmitting replaces seed with submit error', () {
      const seed = MinosError.unauthorized(reason: 'refresh expired');
      const submit = MinosError.unauthorized(reason: 'bad password');
      final container = ProviderContainer(
        overrides: [
          authControllerProvider.overrideWith(
            () => _FixedAuthController(const AuthRefreshFailed(seed)),
          ),
        ],
      );
      addTearDown(container.dispose);

      container.read(loginPageStateControllerProvider);
      container
          .read(loginPageStateControllerProvider.notifier)
          .finishSubmitting(
            mode: LoginPageMode.login,
            clearError: false,
            error: submit,
          );

      expect(
        container.read(loginPageStateControllerProvider).error,
        same(submit),
      );
    });
  });
}
