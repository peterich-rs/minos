import 'package:flutter_test/flutter_test.dart';
import 'package:minos/application/root_route_decision.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  group('decideRootRoute', () {
    test('bootstrapping and refreshing stay on splash', () {
      expect(
        decideRootRoute(
          authState: const AuthBootstrapping(),
          connectionState: null,
        ),
        RootRoute.splash,
      );
      expect(
        decideRootRoute(
          authState: const AuthRefreshing(),
          connectionState: const ConnectionState.connected(),
        ),
        RootRoute.splash,
      );
    });

    test('unauthenticated and refresh-failed go to login', () {
      expect(
        decideRootRoute(
          authState: const AuthUnauthenticated(),
          connectionState: null,
        ),
        RootRoute.login,
      );
      expect(
        decideRootRoute(
          authState: const AuthRefreshFailed(
            MinosError.authRefreshFailed(message: 'expired'),
          ),
          connectionState: null,
        ),
        RootRoute.login,
      );
    });

    test('authenticated maps connection to shell or shellOffline', () {
      const auth = AuthAuthenticated(
        AuthSummary(accountId: 'acc', email: 'a@b.c'),
      );

      expect(
        decideRootRoute(
          authState: auth,
          connectionState: const ConnectionState.connected(),
        ),
        RootRoute.shell,
      );
      expect(
        decideRootRoute(
          authState: auth,
          connectionState: const ConnectionState.reconnecting(attempt: 1),
        ),
        RootRoute.shell,
      );
      expect(
        decideRootRoute(authState: auth, connectionState: null),
        RootRoute.shellOffline,
      );
      expect(
        decideRootRoute(
          authState: auth,
          connectionState: const ConnectionState.disconnected(),
        ),
        RootRoute.shellOffline,
      );
    });
  });
}
