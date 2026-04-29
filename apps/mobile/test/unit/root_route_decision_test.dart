import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/root_route_decision.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  group('decideRootRoute auth gate', () {
    test('AuthBootstrapping always returns splash', () {
      expect(
        decideRootRoute(
          authState: const AuthBootstrapping(),
          connectionState: const ConnectionState.connected(),
          hasPersistedPairing: true,
        ),
        RootRoute.splash,
      );
    });

    test('AuthRefreshing returns splash regardless of pairing/connection', () {
      expect(
        decideRootRoute(
          authState: const AuthRefreshing(),
          connectionState: const ConnectionState.disconnected(),
          hasPersistedPairing: false,
        ),
        RootRoute.splash,
      );
    });

    test('AuthUnauthenticated returns login', () {
      expect(
        decideRootRoute(
          authState: const AuthUnauthenticated(),
          connectionState: const ConnectionState.connected(),
          hasPersistedPairing: true,
        ),
        RootRoute.login,
      );
    });

    test('AuthRefreshFailed returns login', () {
      expect(
        decideRootRoute(
          authState: const AuthRefreshFailed(MinosError.invalidCredentials()),
          connectionState: null,
          hasPersistedPairing: true,
        ),
        RootRoute.login,
      );
    });
  });

  group('decideRootRoute shell gate (when authenticated)', () {
    final account = const AuthSummary(accountId: 'a', email: 'a@b.test');

    test('Authenticated + no pairing still enters shell', () {
      expect(
        decideRootRoute(
          authState: AuthAuthenticated(account),
          connectionState: const ConnectionState.connected(),
          hasPersistedPairing: false,
        ),
        RootRoute.threadList,
      );
    });

    test('Authenticated + paired + connected -> threadList', () {
      expect(
        decideRootRoute(
          authState: AuthAuthenticated(account),
          connectionState: const ConnectionState.connected(),
          hasPersistedPairing: true,
        ),
        RootRoute.threadList,
      );
    });

    test(
      'Authenticated + paired + reconnecting -> threadList (banner only)',
      () {
        expect(
          decideRootRoute(
            authState: AuthAuthenticated(account),
            connectionState: const ConnectionState.reconnecting(attempt: 3),
            hasPersistedPairing: true,
          ),
          RootRoute.threadList,
        );
      },
    );

    test('Authenticated + paired + disconnected -> threadListOffline', () {
      expect(
        decideRootRoute(
          authState: AuthAuthenticated(account),
          connectionState: const ConnectionState.disconnected(),
          hasPersistedPairing: true,
        ),
        RootRoute.threadListOffline,
      );
    });

    test(
      'Authenticated + paired + null connection -> threadListOffline',
      () {
        expect(
          decideRootRoute(
            authState: AuthAuthenticated(account),
            connectionState: null,
            hasPersistedPairing: true,
          ),
          RootRoute.threadListOffline,
        );
      },
    );

    test(
      'Authenticated + paired + Pairing connection -> threadListOffline',
      () {
        // The Pairing connection state means the WS is in the QR-handshake
        // phase, which from the chat surface's perspective is "not fully
        // connected yet" — show the offline UI.
        expect(
          decideRootRoute(
            authState: AuthAuthenticated(account),
            connectionState: const ConnectionState.pairing(),
            hasPersistedPairing: true,
          ),
          RootRoute.threadListOffline,
        );
      },
    );
  });
}
