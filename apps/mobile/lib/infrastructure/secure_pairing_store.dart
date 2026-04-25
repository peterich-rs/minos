import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:minos/infrastructure/cf_access_config.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Keychain-backed persistence for the mobile pairing state.
///
/// The Rust `minos_mobile::MobileClient` keeps this state in its own
/// in-memory `MobilePairingStore` for the lifetime of the process. This
/// Dart-side store is what survives an app cold-start; it persists only the
/// backend URL and Minos device credential. CF Access credentials come from
/// build-time env (`--dart-define`) and are injected into the Rust snapshot
/// on load instead of being written to Keychain.
class SecurePairingStore {
  SecurePairingStore({
    FlutterSecureStorage? storage,
    CfAccessConfig? cfAccessConfig,
  }) : _storage = storage ?? const FlutterSecureStorage(),
       _cfAccessConfig = cfAccessConfig ?? CfAccessConfig.fromEnvironment();

  final FlutterSecureStorage _storage;
  final CfAccessConfig _cfAccessConfig;

  static const _keyBackendUrl = 'minos.backend_url';
  static const _keyDeviceId = 'minos.device_id';
  static const _keyDeviceSecret = 'minos.device_secret';
  static const _keyCfId = 'minos.cf_access_client_id';
  static const _keyCfSecret = 'minos.cf_access_client_secret';

  Future<PersistedPairingState?> loadState() async {
    final backendUrl = await _storage.read(key: _keyBackendUrl);
    final deviceId = await _storage.read(key: _keyDeviceId);
    final deviceSecret = await _storage.read(key: _keyDeviceSecret);
    await _deleteLegacyCfAccessKeys();

    final hasAnyValue =
        backendUrl != null || deviceId != null || deviceSecret != null;
    if (!hasAnyValue) return null;

    final state = _cfAccessConfig.applyToState(
      PersistedPairingState(
        backendUrl: backendUrl,
        deviceId: deviceId,
        deviceSecret: deviceSecret,
      ),
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
    await _deleteLegacyCfAccessKeys();
  }

  /// Wipe every key this store owns. Called from `forgetPeer`.
  Future<void> clearAll() async {
    await _storage.delete(key: _keyBackendUrl);
    await _storage.delete(key: _keyDeviceId);
    await _storage.delete(key: _keyDeviceSecret);
    await _deleteLegacyCfAccessKeys();
  }

  Future<void> _writeOrDelete(String key, String? value) {
    if (value == null) {
      return _storage.delete(key: key);
    }
    return _storage.write(key: key, value: value);
  }

  bool _isResumable(PersistedPairingState state) {
    return state.backendUrl != null &&
        state.deviceId != null &&
        state.deviceSecret != null;
  }

  Future<void> _deleteLegacyCfAccessKeys() async {
    await _storage.delete(key: _keyCfId);
    await _storage.delete(key: _keyCfSecret);
  }
}
