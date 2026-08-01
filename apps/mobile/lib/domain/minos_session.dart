import 'package:flutter/foundation.dart' show immutable;

/// Minos account session tokens after Supabase exchange.
///
/// Mirrors Desktop `MinosSession` / Web `AuthResponse` shape so pure-Dart
/// cloud clients can unit-test exchange without FRB.
@immutable
class MinosSession {
  const MinosSession({
    required this.accountId,
    required this.email,
    required this.accessToken,
    required this.refreshToken,
    required this.accessExpiresAtMs,
  });

  final String accountId;
  final String email;
  final String accessToken;
  final String refreshToken;
  final int accessExpiresAtMs;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is MinosSession &&
          accountId == other.accountId &&
          email == other.email &&
          accessToken == other.accessToken &&
          refreshToken == other.refreshToken &&
          accessExpiresAtMs == other.accessExpiresAtMs;

  @override
  int get hashCode => Object.hash(
    accountId,
    email,
    accessToken,
    refreshToken,
    accessExpiresAtMs,
  );
}
