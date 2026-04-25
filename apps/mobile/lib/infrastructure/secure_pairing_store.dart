import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Keychain-backed persistence for the mobile pairing state.
///
/// The Rust `minos_mobile::MobileClient` keeps this state in its own
/// in-memory `MobilePairingStore` for the lifetime of the process. This
/// Dart-side store is what survives an app cold-start; it mirrors the full
/// persisted pairing snapshot so a fresh Rust client can rehydrate the same
/// `device_id` / `device_secret` pair on launch.
class SecurePairingStore {
  SecurePairingStore({FlutterSecureStorage? storage})
    : _storage = storage ?? const FlutterSecureStorage();

  final FlutterSecureStorage _storage;

  static const _keyBackendUrl = 'minos.backend_url';
  static const _keyDeviceId = 'minos.device_id';
  static const _keyDeviceSecret = 'minos.device_secret';
  static const _keyCfId = 'minos.cf_access_client_id';
  static const _keyCfSecret = 'minos.cf_access_client_secret';

  Future<PersistedPairingState?> loadState() async {
    final backendUrl = await _storage.read(key: _keyBackendUrl);
    final deviceId = await _storage.read(key: _keyDeviceId);
    final deviceSecret = await _storage.read(key: _keyDeviceSecret);
    final cfAccessClientId = await _storage.read(key: _keyCfId);
    final cfAccessClientSecret = await _storage.read(key: _keyCfSecret);

    final hasAnyValue =
        backendUrl != null ||
        deviceId != null ||
        deviceSecret != null ||
        cfAccessClientId != null ||
        cfAccessClientSecret != null;
    if (!hasAnyValue) return null;

    final state = PersistedPairingState(
      backendUrl: backendUrl,
      deviceId: deviceId,
      deviceSecret: deviceSecret,
      cfAccessClientId: cfAccessClientId,
      cfAccessClientSecret: cfAccessClientSecret,
    );

    if (!_isResumable(state)) {
      await clearAll();
      return null;
    }

    return state;
  }

  Future<void> saveState(PersistedPairingState state) async {
    await _writeOrDelete(_keyBackendUrl, state.backendUrl);
    await _writeOrDelete(_keyDeviceId, state.deviceId);
    await _writeOrDelete(_keyDeviceSecret, state.deviceSecret);
    await _writeOrDelete(_keyCfId, state.cfAccessClientId);
    await _writeOrDelete(_keyCfSecret, state.cfAccessClientSecret);
  }

  /// Wipe every key this store owns. Called from `forgetPeer`.
  Future<void> clearAll() async {
    await _storage.delete(key: _keyBackendUrl);
    await _storage.delete(key: _keyDeviceId);
    await _storage.delete(key: _keyDeviceSecret);
    await _storage.delete(key: _keyCfId);
    await _storage.delete(key: _keyCfSecret);
  }

  Future<void> _writeOrDelete(String key, String? value) {
    if (value == null) {
      return _storage.delete(key: key);
    }
    return _storage.write(key: key, value: value);
  }

  bool _isResumable(PersistedPairingState state) {
    final hasRequiredFields =
        state.backendUrl != null &&
        state.deviceId != null &&
        state.deviceSecret != null;
    final hasCfAccessId = state.cfAccessClientId != null;
    final hasCfAccessSecret = state.cfAccessClientSecret != null;
    return hasRequiredFields && hasCfAccessId == hasCfAccessSecret;
  }
}
