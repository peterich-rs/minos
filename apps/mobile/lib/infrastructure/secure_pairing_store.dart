import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Keychain-backed persistence for the mobile pairing state.
///
/// The Rust `minos_mobile::MobileClient` keeps this state in its own
/// in-memory `MobilePairingStore` for the lifetime of the process. This
/// Dart-side store is what survives an app cold-start; it persists the
/// Minos device id **and** (since Phase 4) the account auth tuple
/// so the Rust core can rehydrate `auth_session` synchronously on cold
/// launch via `MobileClient::new_with_persisted_state` and emit
/// `AuthStateFrame::Authenticated` immediately.
///
/// ADR-0020 (server-centric pair): the iOS device no longer holds a
/// `device_secret` — the bearer token alone authenticates the WS, and
/// Mac partners are tracked server-side under `account_mac_pairings`.
/// The legacy `minos.device_secret` keychain entry is wiped on the next
/// cold launch by [loadState] (see "Legacy wipe" below).
///
/// Backend URL and Cloudflare Access credentials are no longer persisted
/// here — they live in `minos_mobile::build_config` (compile-time consts
/// populated by `option_env!` from the cargo-build env). Transport-edge
/// configuration never enters durable storage now.
class SecurePairingStore {
  SecurePairingStore({FlutterSecureStorage? storage})
    : _storage = storage ?? const FlutterSecureStorage();

  final FlutterSecureStorage _storage;

  static const _keyDeviceId = 'minos.device_id';

  // Phase 4 auth fields. All five are written/read as a tuple — partial
  // snapshots are wiped on the next [loadState] call.
  static const _keyAccessToken = 'minos.access_token';
  static const _keyAccessExpiresAtMs = 'minos.access_expires_at_ms';
  static const _keyRefreshToken = 'minos.refresh_token';
  static const _keyAccountId = 'minos.account_id';
  static const _keyAccountEmail = 'minos.account_email';

  // Display name from the scanned QR's `host_display_name`. UI-only —
  // does not gate resume, so it lives outside the snapshot validation.
  static const _keyPeerDisplayName = 'minos.peer_display_name';

  Future<PersistedPairingState?> loadState() async {
    // Legacy wipe: pre ADR-0020 keychain entry. Best-effort; idempotent.
    await _storage.delete(key: 'minos.device_secret');

    final deviceId = await _storage.read(key: _keyDeviceId);
    final accessToken = await _storage.read(key: _keyAccessToken);
    final accessExpiresStr = await _storage.read(key: _keyAccessExpiresAtMs);
    final refreshToken = await _storage.read(key: _keyRefreshToken);
    final accountId = await _storage.read(key: _keyAccountId);
    final accountEmail = await _storage.read(key: _keyAccountEmail);

    final hasAnyValue =
        deviceId != null ||
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
      deviceId: deviceId,
      accessToken: accessToken,
      accessExpiresAtMs: accessExpiresAtMs,
      refreshToken: refreshToken,
      accountId: accountId,
      accountEmail: accountEmail,
    );

    if (!_isValidSnapshot(state) || !_hasCompleteAuth(state)) {
      // Either the identity/auth tuple is incomplete or the auth tuple is
      // half-set. Wipe everything so the next launch gets a clean slate;
      // partial state is never useful.
      await clearAll();
      return null;
    }

    return state;
  }

  Future<void> saveState(PersistedPairingState state) async {
    await _writeOrDelete(_keyDeviceId, state.deviceId);

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

  Future<String?> loadPeerDisplayName() {
    return _storage.read(key: _keyPeerDisplayName);
  }

  Future<void> savePeerDisplayName(String? name) {
    final trimmed = name?.trim();
    if (trimmed == null || trimmed.isEmpty) {
      return _storage.delete(key: _keyPeerDisplayName);
    }
    return _storage.write(key: _keyPeerDisplayName, value: trimmed);
  }

  /// Wipe every key this store owns.
  Future<void> clearAll() async {
    await _storage.delete(key: _keyDeviceId);
    await _storage.delete(key: _keyPeerDisplayName);
    await _deleteAuthKeys();
  }

  /// Wipe only the auth tuple — used by `logout` so the device id stays
  /// valid for the next account login on the same physical device.
  Future<void> clearAuth() async {
    await _deleteAuthKeys();
  }

  Future<void> _writeOrDelete(String key, String? value) {
    if (value == null) {
      return _storage.delete(key: key);
    }
    return _storage.write(key: key, value: value);
  }

  /// Snapshot is valid iff a stable [deviceId] is recorded **and** the
  /// auth tuple is complete (handled separately by [_hasCompleteAuth]).
  /// Post ADR-0020 the device-secret clause is gone — bearer-only auth
  /// means a paired-but-logged-out branch keeps `deviceId` plus no auth
  /// keys, and an authenticated-pre-pair branch keeps `deviceId` plus
  /// the full auth tuple.
  bool _isValidSnapshot(PersistedPairingState state) {
    return state.deviceId != null;
  }

  /// All five auth fields must be present together — either every one is
  /// set (logged-in snapshot) or none (paired-but-unauthenticated, e.g.
  /// after a `logout`). A half-set tuple is treated as corruption.
  bool _hasCompleteAuth(PersistedPairingState state) {
    final allSet =
        state.accessToken != null &&
        state.accessExpiresAtMs != null &&
        state.refreshToken != null &&
        state.accountId != null &&
        state.accountEmail != null;
    final allMissing =
        state.accessToken == null &&
        state.accessExpiresAtMs == null &&
        state.refreshToken == null &&
        state.accountId == null &&
        state.accountEmail == null;
    return allSet || allMissing;
  }

  Future<void> _deleteAuthKeys() async {
    await _storage.delete(key: _keyAccessToken);
    await _storage.delete(key: _keyAccessExpiresAtMs);
    await _storage.delete(key: _keyRefreshToken);
    await _storage.delete(key: _keyAccountId);
    await _storage.delete(key: _keyAccountEmail);
  }
}
