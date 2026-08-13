import 'dart:async';

import 'package:minos/data/repositories/auth_repository.dart';
import 'package:minos/data/services/social_cache_store_service.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart'
    show
        AuthStateFrame,
        AuthStateFrame_Authenticated,
        AuthStateFrame_RefreshFailed,
        AuthStateFrame_Refreshing,
        AuthStateFrame_Unauthenticated;
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'auth_provider.g.dart';

/// Mirrors the Rust-side `AuthState` watch-channel into the Dart UI tier.
///
/// The Rust forwarder (`subscribe_auth_state`) emits the current cached
/// frame immediately on subscribe, then every transition. The provider
/// returns [AuthBootstrapping] from `build()`; the very first frame from
/// the stream replaces it on the next microtask. Components watching
/// this provider should treat [AuthBootstrapping] as "show splash".
///
/// On the first `Authenticated` transition, the controller also kicks the
/// Rust WS reconnect path via `resumePersistedSession()` so the chat
/// surface lights up without a separate trigger.
///
/// Cross-account migration sequence:
///   1. `register` / `login` go through Supabase IdP then
///      [MinosCore.loginWithSupabase], which adopt the freshly minted
///      `account_id` against the prior persisted snapshot.
///   2. If the prior `account_id` differs, `MinosCore` clears the
///      cached peer display name so a stale label from the previous
///      account doesn't briefly flash in the partners list. The
///      server-side `account_mac_pairings` rows are already
///      account-scoped, so the next `listPairedHosts` sync naturally
///      yields the new account's Macs.
///   3. The first `Authenticated` frame fires; this controller calls
///      `resumePersistedSession()` which spins up the WS for the new
///      account.
///   4. The Partners tab calls `listPairedHosts` and shows whatever
///      Macs are paired to the new account (possibly empty → the user
///      taps "Add partner" to scan a QR).
@Riverpod(keepAlive: true)
class AuthController extends _$AuthController {
  StreamSubscription<AuthStateFrame>? _sub;
  bool _wsResumed = false;

  AuthRepository get _repository => ref.read(authRepositoryProvider);

  @override
  AuthState build() {
    _sub = ref.watch(authRepositoryProvider).authStates.listen(_onFrame);
    ref.onDispose(() => _sub?.cancel());
    return const AuthBootstrapping();
  }

  void _onFrame(AuthStateFrame frame) {
    state = switch (frame) {
      AuthStateFrame_Unauthenticated() => const AuthUnauthenticated(),
      AuthStateFrame_Authenticated(:final account) => AuthAuthenticated(
        account,
      ),
      AuthStateFrame_Refreshing() => const AuthRefreshing(),
      AuthStateFrame_RefreshFailed(:final error) => AuthRefreshFailed(error),
    };
    if (frame is AuthStateFrame_Authenticated && !_wsResumed) {
      _wsResumed = true;
      // Best-effort: missing host link / offline Mac surfaces via
      // connectionStateProvider — don't block the auth flow.
      unawaited(_repository.resumePersistedSession().catchError((_) {}));
    } else if (frame is AuthStateFrame_Unauthenticated) {
      _wsResumed = false;
    }
  }

  /// Register a fresh account. Errors propagate; the state itself is
  /// driven exclusively from the Rust auth-state stream so the UI sees
  /// the same transitions whether the trigger was UI-initiated or
  /// background refresh.
  Future<void> register(String email, String password) async {
    await _repository.register(email, password);
  }

  /// Log into an existing account. See [register] for state-update
  /// semantics. When Supabase is configured, uses IdP → Minos exchange.
  Future<void> login(String email, String password) async {
    await _repository.login(email, password);
  }

  /// Exchange a raw Supabase access token (OAuth / deep-link path).
  Future<void> loginWithSupabaseToken(String supabaseAccessToken) async {
    await _repository.loginWithSupabaseToken(supabaseAccessToken);
  }

  /// Dual-session logout: Minos revoke + local wipe + best-effort Supabase.
  Future<void> logout() async {
    await _repository.logout();
    // Account-scope: drop social cache + im_outbox so drafts cannot send as B.
    await ref.read(socialCacheStoreProvider).clearAllForLogout();
  }
}
