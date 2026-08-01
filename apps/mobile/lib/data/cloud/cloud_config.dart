/// Compile-time cloud identity / backend config for the mobile data plane.
///
/// Values come from `--dart-define=KEY=value` (CI, local `just dev-mobile-*`,
/// or Xcode/Gradle scheme env). Empty strings mean "not configured".
///
/// Mirrors Desktop/Web:
/// - `MINOS_BACKEND_URL` — ws(s) or http(s) base for the Minos hub
/// - `SUPABASE_URL` + `SUPABASE_ANON_KEY` — Supabase Auth IdP
class CloudConfig {
  const CloudConfig({
    required this.backendUrl,
    required this.supabaseUrl,
    required this.supabaseAnonKey,
  });

  /// From `--dart-define=MINOS_BACKEND_URL=...` (ws or http).
  final String backendUrl;

  /// From `--dart-define=SUPABASE_URL=...`.
  final String supabaseUrl;

  /// From `--dart-define=SUPABASE_ANON_KEY=...`.
  final String supabaseAnonKey;

  /// Production / staging config resolved from dart-defines.
  factory CloudConfig.fromEnvironment() {
    return const CloudConfig(
      backendUrl: String.fromEnvironment(
        'MINOS_BACKEND_URL',
        defaultValue: 'ws://127.0.0.1:8787/devices',
      ),
      supabaseUrl: String.fromEnvironment('SUPABASE_URL'),
      supabaseAnonKey: String.fromEnvironment('SUPABASE_ANON_KEY'),
    );
  }

  /// True when both Supabase env slots are non-empty (OAuth / IdP path).
  bool get isSupabaseConfigured =>
      supabaseUrl.trim().isNotEmpty && supabaseAnonKey.trim().isNotEmpty;

  /// HTTP origin used by pure-Dart repositories (`GET /v1/hosts`, exchange).
  ///
  /// Accepts the same `ws://` / `wss://` URLs baked into the Rust mobile
  /// client (`…/devices`) and normalizes them to an http(s) origin without
  /// a trailing slash.
  String get httpBase {
    final raw = backendUrl.trim();
    if (raw.isEmpty) {
      return 'http://127.0.0.1:8787';
    }
    var s = raw;
    if (s.startsWith('wss://')) {
      s = 'https://${s.substring(6)}';
    } else if (s.startsWith('ws://')) {
      s = 'http://${s.substring(5)}';
    }
    // Strip `/devices` suffix used by the WS gateway path.
    if (s.endsWith('/devices')) {
      s = s.substring(0, s.length - '/devices'.length);
    }
    while (s.endsWith('/')) {
      s = s.substring(0, s.length - 1);
    }
    return s;
  }
}
