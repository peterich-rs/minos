import 'package:flutter_test/flutter_test.dart';

import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  group('AuthState equality', () {
    test('AuthAuthenticated equals by account_id, ignoring email', () {
      final a1 = AuthAuthenticated(
        const AuthSummary(accountId: 'a', email: 'a@b.test'),
      );
      final a2 = AuthAuthenticated(
        const AuthSummary(accountId: 'a', email: 'b@c.test'),
      );
      expect(a1, equals(a2));
      expect(a1.hashCode, a2.hashCode);
    });

    test('AuthAuthenticated differs when account_id differs', () {
      final a1 = AuthAuthenticated(
        const AuthSummary(accountId: 'a', email: 'shared@x.test'),
      );
      final a2 = AuthAuthenticated(
        const AuthSummary(accountId: 'b', email: 'shared@x.test'),
      );
      expect(a1, isNot(equals(a2)));
    });

    test('AuthBootstrapping / AuthUnauthenticated / AuthRefreshing are singletons', () {
      expect(const AuthBootstrapping(), equals(const AuthBootstrapping()));
      expect(const AuthUnauthenticated(), equals(const AuthUnauthenticated()));
      expect(const AuthRefreshing(), equals(const AuthRefreshing()));
      expect(
        const AuthBootstrapping(),
        isNot(equals(const AuthUnauthenticated())),
      );
    });

    test('AuthRefreshFailed equals when underlying error is equal', () {
      const e = MinosError.invalidCredentials();
      expect(const AuthRefreshFailed(e), equals(const AuthRefreshFailed(e)));
      expect(
        const AuthRefreshFailed(e),
        isNot(equals(const AuthRefreshFailed(MinosError.rateLimited(retryAfterS: 5)))),
      );
    });
  });
}
