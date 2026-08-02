import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/cloud/cloud_config.dart';
import 'package:minos/data/cloud/supabase_auth_service.dart';
import 'package:minos/data/repositories/hosts_repository.dart'
    show cloudConfigProvider;
import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart' show AuthStateFrame;

final supabaseAuthServiceProvider = Provider<SupabaseAuthService>((ref) {
  return SupabaseAuthService(config: ref.watch(cloudConfigProvider));
});

final authRepositoryProvider = Provider<AuthRepository>((ref) {
  return AuthRepository(
    ref.watch(minosCoreServiceProvider),
    supabase: ref.watch(supabaseAuthServiceProvider),
    cloudConfig: ref.watch(cloudConfigProvider),
  );
});

class AuthRepository {
  const AuthRepository(
    this._core, {
    required SupabaseAuthService supabase,
    required CloudConfig cloudConfig,
  }) : _supabase = supabase,
       _cloudConfig = cloudConfig;

  final MinosCoreProtocol _core;
  final SupabaseAuthService _supabase;
  final CloudConfig _cloudConfig;

  bool get isSupabaseConfigured => _cloudConfig.isSupabaseConfigured;

  Stream<AuthStateFrame> get authStates => _core.authStates;

  Future<void> resumePersistedSession() {
    return _core.resumePersistedSession();
  }

  void _requireSupabase() {
    if (!isSupabaseConfigured) {
      throw StateError(
        'Supabase is required for Minos account auth. '
        'Set SUPABASE_URL and SUPABASE_ANON_KEY (dart-defines) before login/register.',
      );
    }
  }

  /// Supabase IdP sign-up → Minos `POST /v1/auth/supabase` exchange.
  Future<void> register(String email, String password) async {
    _requireSupabase();
    final token = await _supabase.signUpWithPassword(
      email: email,
      password: password,
    );
    await _core.loginWithSupabase(supabaseAccessToken: token);
  }

  /// Supabase IdP sign-in → Minos exchange.
  Future<void> login(String email, String password) async {
    _requireSupabase();
    final token = await _supabase.signInWithPassword(
      email: email,
      password: password,
    );
    await _core.loginWithSupabase(supabaseAccessToken: token);
  }

  /// Explicit Supabase-token exchange (OAuth deep-link path).
  Future<void> loginWithSupabaseToken(String supabaseAccessToken) {
    _requireSupabase();
    return _core.loginWithSupabase(supabaseAccessToken: supabaseAccessToken);
  }

  /// Dual-session logout: revoke Minos refresh, wipe local, best-effort
  /// Supabase signOut.
  Future<void> logout() async {
    await _core.logout();
    await _supabase.signOut();
  }
}
