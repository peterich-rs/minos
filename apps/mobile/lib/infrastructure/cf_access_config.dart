import 'dart:convert';

import 'package:minos/src/rust/api/minos.dart';

/// Build-time Cloudflare Access service-token configuration.
///
/// Flutter reads these values from `--dart-define`, so CI can inject GitHub
/// Secrets and local runs can forward shell env vars with:
/// `--dart-define=CF_ACCESS_CLIENT_ID=$CF_ACCESS_CLIENT_ID`.
class CfAccessConfig {
  CfAccessConfig({String? clientId, String? clientSecret})
    : clientId = _blankToNull(clientId),
      clientSecret = _blankToNull(clientSecret) {
    if ((this.clientId == null) != (this.clientSecret == null)) {
      throw ArgumentError(
        'CF_ACCESS_CLIENT_ID and CF_ACCESS_CLIENT_SECRET must be set together',
      );
    }
  }

  static const _envClientId = String.fromEnvironment('CF_ACCESS_CLIENT_ID');
  static const _envClientSecret = String.fromEnvironment(
    'CF_ACCESS_CLIENT_SECRET',
  );

  factory CfAccessConfig.fromEnvironment() =>
      CfAccessConfig(clientId: _envClientId, clientSecret: _envClientSecret);

  final String? clientId;
  final String? clientSecret;

  bool get isConfigured => clientId != null && clientSecret != null;

  PersistedPairingState applyToState(PersistedPairingState state) {
    return PersistedPairingState(
      backendUrl: state.backendUrl,
      deviceId: state.deviceId,
      deviceSecret: state.deviceSecret,
      cfAccessClientId: clientId,
      cfAccessClientSecret: clientSecret,
    );
  }

  /// Override any QR-carried CF Access fields with the build-time values.
  ///
  /// Invalid JSON is deliberately passed through unchanged so Rust preserves
  /// the existing `qr_payload` parse error behavior.
  String applyToQrJson(String qrJson) {
    if (!isConfigured && !qrJson.contains('cf_access_client_')) {
      return qrJson;
    }

    final Object? decoded;
    try {
      decoded = jsonDecode(qrJson);
    } catch (_) {
      return qrJson;
    }
    if (decoded is! Map) return qrJson;

    final next = Map<String, Object?>.from(decoded);
    if (isConfigured) {
      next['cf_access_client_id'] = clientId;
      next['cf_access_client_secret'] = clientSecret;
    } else {
      next.remove('cf_access_client_id');
      next.remove('cf_access_client_secret');
    }
    return jsonEncode(next);
  }

  static String? _blankToNull(String? value) {
    final trimmed = value?.trim();
    if (trimmed == null || trimmed.isEmpty) return null;
    return trimmed;
  }
}
