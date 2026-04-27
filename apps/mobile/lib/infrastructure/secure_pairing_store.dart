import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:minos/infrastructure/cf_access_config.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Keychain-backed persistence for the mobile pairing state.
///
/// The Rust `minos_mobile::MobileClient` keeps this state in its own
/// in-memory `MobilePairingStore` for the lifetime of the process. This
/// Dart-side store is what survives an app cold-start; it persists the
/// backend URL, the Minos device credential, **and** (since Phase 4) the
/// account auth tuple so the Rust core can rehydrate
/// `auth_session` synchronously on cold launch via
/// `MobileClient::new_with_persisted_state` and emit
/// `AuthStateFrame::Authenticated` immediately.
///
/// CF Access credentials normally come from build-time env
/// (`--dart-define`), but QR-carried credentials are kept in Keychain
/// when no build-time pair exists so real-device pairing can reconnect
/// after an app restart.
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

  // Phase 4 auth fields. All five are written/read as a tuple — partial
  // snapshots are wiped on the next [loadState] call.
  static const _keyAccessToken = 'minos.access_token';
  static const _keyAccessExpiresAtMs = 'minos.access_expires_at_ms';
  static const _keyRefreshToken = 'minos.refresh_token';
  static const _keyAccountId = 'minos.account_id';
  static const _keyAccountEmail = 'minos.account_email';

  Future<PersistedPairingState?> loadState() async {
    final backendUrl = await _storage.read(key: _keyBackendUrl);
    final deviceId = await _storage.read(key: _keyDeviceId);
    final deviceSecret = await _storage.read(key: _keyDeviceSecret);
    final storedCfId = await _storage.read(key: _keyCfId);
    final storedCfSecret = await _storage.read(key: _keyCfSecret);
    final accessToken = await _storage.read(key: _keyAccessToken);
    final accessExpiresStr = await _storage.read(key: _keyAccessExpiresAtMs);
    final refreshToken = await _storage.read(key: _keyRefreshToken);
    final accountId = await _storage.read(key: _keyAccountId);
    final accountEmail = await _storage.read(key: _keyAccountEmail);

    final hasAnyValue =
        backendUrl != null ||
        deviceId != null ||
        deviceSecret != null ||
        storedCfId != null ||
        storedCfSecret != null ||
        accessToken != null ||
        accessExpiresStr != null ||
        refreshToken != null ||
        accountId != null ||
        accountEmail != null;
    if (!hasAnyValue) return null;

    final accessExpiresAtMs = accessExpiresStr == null
        ? null
        : int.tryParse(accessExpiresStr);

    final state = PersistedPairingState(
      backendUrl: backendUrl,
      deviceId: deviceId,
      deviceSecret: deviceSecret,
      cfAccessClientId: _cfAccessConfig.clientId ?? storedCfId,
      cfAccessClientSecret: _cfAccessConfig.clientSecret ?? storedCfSecret,
      accessToken: accessToken,
      accessExpiresAtMs: accessExpiresAtMs,
      refreshToken: refreshToken,
      accountId: accountId,
      accountEmail: accountEmail,
    );

    if (!_isResumable(state) || !_hasCompleteCfAccess(state) || !_hasCompleteAuth(state)) {
      // Either core resume is impossible, the CF Access tuple is half-set,
      // or the auth tuple is half-set. Wipe everything so the next launch
      // gets a clean slate; partial state is never useful.
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

    if (_hasCompleteAuth(state)) {
      await _writeOrDelete(_keyAccessToken, state.accessToken);
      await _writeOrDelete(
        _keyAccessExpiresAtMs,
        state.accessExpiresAtMs?.toString(),
      );
      await _writeOrDelete(_keyRefreshToken, state.refreshToken);
      await _writeOrDelete(_keyAccountId, state.accountId);
      await _writeOrDelete(_keyAccountEmail, state.accountEmail);
    } else {
      await _deleteAuthKeys();
    }
  }

  /// Wipe every key this store owns. Called from `forgetPeer`.
  Future<void> clearAll() async {
    await _storage.delete(key: _keyBackendUrl);
    await _storage.delete(key: _keyDeviceId);
    await _storage.delete(key: _keyDeviceSecret);
    await _deleteCfAccessKeys();
    await _deleteAuthKeys();
  }

  /// Wipe only the auth tuple — used by `logout` so the device credential
  /// stays valid for the next account login on the same physical device.
  Future<void> clearAuth() async {
    await _deleteAuthKeys();
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

  /// All five auth fields must be present together — either every one is
  /// set (logged-in snapshot) or none (paired-but-unauthenticated, e.g.
  /// after a `logout`). A half-set tuple is treated as corruption.
  bool _hasCompleteAuth(PersistedPairingState state) {
    final allSet = state.accessToken != null &&
        state.accessExpiresAtMs != null &&
        state.refreshToken != null &&
        state.accountId != null &&
        state.accountEmail != null;
    final allMissing = state.accessToken == null &&
        state.accessExpiresAtMs == null &&
        state.refreshToken == null &&
        state.accountId == null &&
        state.accountEmail == null;
    return allSet || allMissing;
  }

  Future<void> _deleteCfAccessKeys() async {
    await _storage.delete(key: _keyCfId);
    await _storage.delete(key: _keyCfSecret);
  }

  Future<void> _deleteAuthKeys() async {
    await _storage.delete(key: _keyAccessToken);
    await _storage.delete(key: _keyAccessExpiresAtMs);
    await _storage.delete(key: _keyRefreshToken);
    await _storage.delete(key: _keyAccountId);
    await _storage.delete(key: _keyAccountEmail);
  }
}
