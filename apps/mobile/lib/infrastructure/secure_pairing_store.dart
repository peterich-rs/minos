import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:minos/infrastructure/cf_access_config.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Keychain-backed persistence for the mobile pairing state.
///
/// The Rust `minos_mobile::MobileClient` keeps this state in its own
/// in-memory `MobilePairingStore` for the lifetime of the process. This
/// Dart-side store is what survives an app cold-start; it persists the
/// backend URL and Minos device credential. CF Access credentials normally
/// come from build-time env (`--dart-define`), but QR-carried credentials are
/// kept in Keychain when no build-time pair exists so real-device pairing can
/// reconnect after an app restart.
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
    final storedCfId = await _storage.read(key: _keyCfId);
    final storedCfSecret = await _storage.read(key: _keyCfSecret);

    final hasAnyValue =
        backendUrl != null ||
        deviceId != null ||
        deviceSecret != null ||
        storedCfId != null ||
        storedCfSecret != null;
    if (!hasAnyValue) return null;

    final state = PersistedPairingState(
      backendUrl: backendUrl,
      deviceId: deviceId,
      deviceSecret: deviceSecret,
      cfAccessClientId: _cfAccessConfig.clientId ?? storedCfId,
      cfAccessClientSecret: _cfAccessConfig.clientSecret ?? storedCfSecret,
    );

    if (!_isResumable(state) || !_hasCompleteCfAccess(state)) {
      await clearAll();
      return null;
    }

    return state;
  }

  Future<void> saveState(PersistedPairingState state) async {
    await _writeOrDelete(_keyBackendUrl, state.backendUrl);
    await _writeOrDelete(_keyDeviceId, state.deviceId);
    await _writeOrDelete(_keyDeviceSecret, state.deviceSecret);
    if (_cfAccessConfig.isConfigured || !_hasCompleteCfAccess(state)) {
      await _deleteCfAccessKeys();
    } else {
      await _writeOrDelete(_keyCfId, state.cfAccessClientId);
      await _writeOrDelete(_keyCfSecret, state.cfAccessClientSecret);
    }
  }

  /// Wipe every key this store owns. Called from `forgetPeer`.
  Future<void> clearAll() async {
    await _storage.delete(key: _keyBackendUrl);
    await _storage.delete(key: _keyDeviceId);
    await _storage.delete(key: _keyDeviceSecret);
    await _deleteCfAccessKeys();
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

  bool _hasCompleteCfAccess(PersistedPairingState state) {
    return (state.cfAccessClientId == null) ==
        (state.cfAccessClientSecret == null);
  }

  Future<void> _deleteCfAccessKeys() async {
    await _storage.delete(key: _keyCfId);
    await _storage.delete(key: _keyCfSecret);
  }
}
