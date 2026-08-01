import 'package:minos/data/cloud/cloud_config.dart';
import 'package:supabase_flutter/supabase_flutter.dart';

/// Thin wrapper around `supabase_flutter` for the mobile IdP path.
///
/// When [CloudConfig.isSupabaseConfigured] is false, methods throw
/// [StateError] so callers can fall back to Minos password auth.
class SupabaseAuthService {
  SupabaseAuthService({required CloudConfig config}) : _config = config;

  final CloudConfig _config;
  bool _initialized = false;

  bool get isConfigured => _config.isSupabaseConfigured;

  /// Idempotent init. Safe to call from `main` and again from login.
  Future<void> ensureInitialized() async {
    if (!isConfigured || _initialized) return;
    // publishableKey is the current Supabase Flutter param name; anon JWT
    // remains the value (public client key).
    await Supabase.initialize(
      url: _config.supabaseUrl.trim(),
      publishableKey: _config.supabaseAnonKey.trim(),
      authOptions: const FlutterAuthClientOptions(
        authFlowType: AuthFlowType.pkce,
      ),
    );
    _initialized = true;
  }

  SupabaseClient get _client {
    if (!isConfigured) {
      throw StateError('Supabase is not configured (SUPABASE_URL / ANON_KEY)');
    }
    return Supabase.instance.client;
  }

  /// Email + password via Supabase Auth (not Minos password).
  Future<String> signInWithPassword({
    required String email,
    required String password,
  }) async {
    await ensureInitialized();
    final result = await _client.auth.signInWithPassword(
      email: email,
      password: password,
    );
    final token = result.session?.accessToken;
    if (token == null || token.isEmpty) {
      throw StateError('Supabase sign-in returned no access_token');
    }
    return token;
  }

  Future<String> signUpWithPassword({
    required String email,
    required String password,
  }) async {
    await ensureInitialized();
    final result = await _client.auth.signUp(email: email, password: password);
    final token = result.session?.accessToken;
    if (token == null || token.isEmpty) {
      throw StateError(
        'Supabase sign-up succeeded but no session yet '
        '(email confirmation may be required)',
      );
    }
    return token;
  }

  Future<String?> currentAccessToken() async {
    if (!isConfigured) return null;
    await ensureInitialized();
    return _client.auth.currentSession?.accessToken;
  }

  /// Best-effort dual-session logout (T-auth-09 / T-mob-07).
  Future<void> signOut() async {
    if (!isConfigured) return;
    try {
      await ensureInitialized();
      await _client.auth.signOut();
    } catch (_) {
      // Best-effort: Minos session wipe is the source of truth for API access.
    }
  }
}
