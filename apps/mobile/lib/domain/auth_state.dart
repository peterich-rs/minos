import 'package:flutter/foundation.dart' show immutable;

import 'package:minos/src/rust/api/minos.dart' show AuthSummary, MinosError;

/// Dart-owned mirror of the auth lifecycle. Distinct from the
/// frb-generated [AuthStateFrame] — this layer sits above the wire and is
/// what the UI watches.
///
/// `AuthBootstrapping` is the synthetic transient state held while the
/// frb stream subscription is being set up; once the first frame from the
/// Rust watch-channel arrives the state transitions to one of the
/// concrete variants.
@immutable
sealed class AuthState {
  const AuthState();
}

/// Pre-stream state. The provider returns this from `build()` and the
/// stream subscription replaces it on the very next microtask.
class AuthBootstrapping extends AuthState {
  const AuthBootstrapping();

  @override
  bool operator ==(Object other) => other is AuthBootstrapping;

  @override
  int get hashCode => (AuthBootstrapping).hashCode;
}

class AuthUnauthenticated extends AuthState {
  const AuthUnauthenticated();

  @override
  bool operator ==(Object other) => other is AuthUnauthenticated;

  @override
  int get hashCode => (AuthUnauthenticated).hashCode;
}

/// The user is logged in. Equality is keyed on `account_id` only — the
/// email field can update for the same account (e.g. address change) and
/// we don't want to thrash the UI on cosmetic re-emissions.
class AuthAuthenticated extends AuthState {
  final AuthSummary account;
  const AuthAuthenticated(this.account);

  @override
  bool operator ==(Object other) =>
      other is AuthAuthenticated && other.account.accountId == account.accountId;

  @override
  int get hashCode => account.accountId.hashCode;
}

class AuthRefreshing extends AuthState {
  const AuthRefreshing();

  @override
  bool operator ==(Object other) => other is AuthRefreshing;

  @override
  int get hashCode => (AuthRefreshing).hashCode;
}

/// Refresh attempt failed; the UI should route back to login. Carries the
/// typed [MinosError] so the LoginPage can render a destructive banner
/// with the same `userMessage()` it uses for direct failures.
class AuthRefreshFailed extends AuthState {
  final MinosError error;
  const AuthRefreshFailed(this.error);

  @override
  bool operator ==(Object other) =>
      other is AuthRefreshFailed && other.error == error;

  @override
  int get hashCode => error.hashCode;
}
