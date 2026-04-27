import 'dart:async';

import 'package:riverpod_annotation/riverpod_annotation.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart'
    show
        AuthStateFrame,
        AuthStateFrame_Authenticated,
        AuthStateFrame_RefreshFailed,
        AuthStateFrame_Refreshing,
        AuthStateFrame_Unauthenticated;

part 'auth_provider.g.dart';

/// Mirrors the Rust-side `AuthState` watch-channel into the Dart UI tier.
///
/// The Rust forwarder (`subscribe_auth_state`) emits the current cached
/// frame immediately on subscribe, then every transition. The provider
/// returns [AuthBootstrapping] from `build()`; the very first frame from
/// the stream replaces it on the next microtask. Components watching
/// this provider should treat [AuthBootstrapping] as "show splash".
///
/// Phase 8.9: on the first `Authenticated` transition, the controller
/// also kicks the Rust WS reconnect path via `resumePersistedSession()`
/// so the chat surface lights up without a separate trigger.
///
/// Phase 11.3 — cross-account migration sequence (manual smoke #7+8):
///   1. `register` / `login` go through [MinosCore.register] /
///      [MinosCore.login], which compare the freshly minted
///      `account_id` against the prior persisted snapshot.
///   2. If the prior `account_id` differs (a different account is
///      logging in on a previously paired device), `MinosCore` calls
///      `forgetPeer()` BEFORE persisting the new auth tuple. This wipes
///      the device-side pairing tuple so the route gate flips to
///      `pairing` for the new account.
///   3. The first `Authenticated` frame fires; this controller calls
///      `resumePersistedSession()` which is a no-op when pairing was
///      just dropped (and any error is swallowed).
///   4. The router observes `hasPersistedPairing == false` and shows
///      [PairingPage] for the new account to scan a fresh QR.
@Riverpod(keepAlive: true)
class AuthController extends _$AuthController {
  StreamSubscription<AuthStateFrame>? _sub;
  bool _wsResumed = false;

  @override
  AuthState build() {
    final core = ref.watch(minosCoreProvider);
    _sub = core.authStates.listen(_onFrame);
    ref.onDispose(() => _sub?.cancel());
    return const AuthBootstrapping();
  }

  void _onFrame(AuthStateFrame frame) {
    state = switch (frame) {
      AuthStateFrame_Unauthenticated() => const AuthUnauthenticated(),
      AuthStateFrame_Authenticated(:final account) =>
        AuthAuthenticated(account),
      AuthStateFrame_Refreshing() => const AuthRefreshing(),
      AuthStateFrame_RefreshFailed(:final error) => AuthRefreshFailed(error),
    };
    if (frame is AuthStateFrame_Authenticated && !_wsResumed) {
      _wsResumed = true;
      // Best-effort: a missing pairing snapshot or an unreachable Mac
      // surfaces on connectionStateProvider — don't block the auth flow.
      unawaited(ref.read(minosCoreProvider).resumePersistedSession().catchError((_) {}));
    } else if (frame is AuthStateFrame_Unauthenticated) {
      _wsResumed = false;
    }
  }

  /// Register a fresh account. Errors propagate; the state itself is
  /// driven exclusively from the Rust auth-state stream so the UI sees
  /// the same transitions whether the trigger was UI-initiated or
  /// background refresh.
  Future<void> register(String email, String password) async {
    await ref.read(minosCoreProvider).register(email: email, password: password);
  }

  /// Log into an existing account. See [register] for state-update
  /// semantics.
  Future<void> login(String email, String password) async {
    await ref.read(minosCoreProvider).login(email: email, password: password);
  }

  /// Best-effort logout: revoke server-side, wipe local secrets, and let
  /// the Rust core flip [authStates] to `Unauthenticated`.
  Future<void> logout() async {
    await ref.read(minosCoreProvider).logout();
  }
}
