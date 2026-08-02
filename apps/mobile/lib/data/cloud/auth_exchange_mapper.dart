import 'package:minos/domain/minos_session.dart';

/// Pure JSON → [MinosSession] mapping for auth endpoints that return the
/// standard Minos token payload:
/// - `POST /v1/auth/supabase`
MinosSession mapAuthResponse(Object? json, {int? nowMs}) {
  if (json is! Map) {
    throw FormatException(
      'auth response expected object, got ${json.runtimeType}',
    );
  }
  final root = Map<String, dynamic>.from(json);
  final accountNode = root['account'];
  if (accountNode is! Map) {
    throw const FormatException('auth response missing account');
  }
  final account = Map<String, dynamic>.from(accountNode);
  final accountId = account['account_id']?.toString() ?? '';
  final email = account['email']?.toString() ?? '';
  final accessToken = root['access_token']?.toString() ?? '';
  final refreshToken = root['refresh_token']?.toString() ?? '';
  final expiresRaw = root['expires_in'];
  final expiresIn = _asInt(expiresRaw);
  if (accountId.isEmpty || accessToken.isEmpty || refreshToken.isEmpty) {
    throw const FormatException('auth response incomplete token tuple');
  }
  final baseMs = nowMs ?? DateTime.now().millisecondsSinceEpoch;
  return MinosSession(
    accountId: accountId,
    email: email,
    accessToken: accessToken,
    refreshToken: refreshToken,
    accessExpiresAtMs: baseMs + (expiresIn * 1000),
  );
}

/// Body for `POST /v1/auth/supabase`.
Map<String, dynamic> supabaseExchangeRequestBody({
  required String accessToken,
  String? deviceName,
}) {
  return <String, dynamic>{
    'access_token': accessToken,
    if (deviceName != null && deviceName.trim().isNotEmpty)
      'device_name': deviceName.trim(),
  };
}

int _asInt(Object? value) {
  if (value is int) return value;
  if (value is num) return value.toInt();
  if (value is String) return int.tryParse(value) ?? 0;
  return 0;
}
