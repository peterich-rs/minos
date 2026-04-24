import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Keychain-backed persistence for the mobile pairing state.
///
/// The Rust `minos_mobile::MobileClient` keeps this state in its own
/// in-memory `MobilePairingStore` for the lifetime of the process (plan 05
/// §D5 accepts the duplication as MVP tradeoff). This Dart-side store is
/// what survives an app cold-start; [saveFromQrJson] mirrors the fields
/// from the scanned QR payload, and [loadBackendUrl] is what a future
/// reconnect path will read before reopening the WS.
class SecurePairingStore {
  SecurePairingStore({FlutterSecureStorage? storage})
    : _storage = storage ?? const FlutterSecureStorage();

  final FlutterSecureStorage _storage;

  static const _keyBackendUrl = 'minos.backend_url';
  static const _keyCfId = 'minos.cf_access_client_id';
  static const _keyCfSecret = 'minos.cf_access_client_secret';

  Future<String?> loadBackendUrl() => _storage.read(key: _keyBackendUrl);
  Future<void> saveBackendUrl(String url) =>
      _storage.write(key: _keyBackendUrl, value: url);

  /// Load the CF Access service-token pair, or `null` if unset.
  Future<({String id, String secret})?> loadCfAccess() async {
    final id = await _storage.read(key: _keyCfId);
    final secret = await _storage.read(key: _keyCfSecret);
    if (id == null || secret == null) return null;
    return (id: id, secret: secret);
  }

  Future<void> saveCfAccess(String id, String secret) async {
    await _storage.write(key: _keyCfId, value: id);
    await _storage.write(key: _keyCfSecret, value: secret);
  }

  /// Extract persistable fields from a raw QR v2 JSON payload and write
  /// them to the Keychain. Silently no-ops on malformed JSON (the Rust
  /// side already surfaced a [MinosError] to the caller; writing garbage
  /// here would only add noise).
  Future<void> saveFromQrJson(String qrJson) async {
    final Map<String, dynamic> decoded;
    try {
      decoded = jsonDecode(qrJson) as Map<String, dynamic>;
    } catch (_) {
      return;
    }
    final backendUrl = decoded['backend_url'] as String?;
    if (backendUrl != null) {
      await saveBackendUrl(backendUrl);
    }
    final id = decoded['cf_access_client_id'] as String?;
    final secret = decoded['cf_access_client_secret'] as String?;
    if (id != null && secret != null) {
      await saveCfAccess(id, secret);
    }
  }

  /// Wipe every key this store owns. Called from `forgetPeer`.
  Future<void> clearAll() async {
    await _storage.delete(key: _keyBackendUrl);
    await _storage.delete(key: _keyCfId);
    await _storage.delete(key: _keyCfSecret);
  }
}
