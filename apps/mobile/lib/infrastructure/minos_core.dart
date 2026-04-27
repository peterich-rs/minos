import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:meta/meta.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/infrastructure/cf_access_config.dart';
import 'package:minos/infrastructure/secure_pairing_store.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/src/rust/frb_generated.dart';

/// The one place in the Dart codebase allowed to import the frb-generated
/// [MobileClient]. Everything above this layer depends on
/// [MinosCoreProtocol] instead.
class MinosCore implements MinosCoreProtocol {
  MinosCore._(this._client, this._secure, this._cfAccessConfig);

  factory MinosCore.forTesting({
    required MobileClient client,
    required SecurePairingStore secureStore,
    CfAccessConfig? cfAccessConfig,
  }) => MinosCore._(client, secureStore, cfAccessConfig ?? CfAccessConfig());

  final MobileClient _client;
  final SecurePairingStore _secure;
  final CfAccessConfig _cfAccessConfig;

  /// Construct and initialize the core. Must be awaited before any other
  /// Riverpod provider reads it.
  static Future<MinosCore> init({
    required String selfName,
    required String logDir,
    SecurePairingStore? secureStore,
    CfAccessConfig? cfAccessConfig,
  }) async {
    // On iOS the Rust pod force-loads `libminos_ffi_frb.a` into Runner, so
    // frb must resolve symbols from the current process instead of opening a
    // non-existent `minos_ffi_frb.framework/minos_ffi_frb`.
    final externalLibrary = Platform.isIOS
        ? ExternalLibrary.process(
            iKnowHowToUseIt: true,
            debugInfo: ' (libminos_ffi_frb.a is linked into Runner)',
          )
        : null;
    await RustLib.init(externalLibrary: externalLibrary);
    await initLogging(logDir: logDir);
    final accessConfig = cfAccessConfig ?? CfAccessConfig.fromEnvironment();
    final secure =
        secureStore ?? SecurePairingStore(cfAccessConfig: accessConfig);
    final client = await resolveClient(
      secure: secure,
      buildFresh: () => MobileClient(selfName: selfName),
      buildFromPersisted: (state) =>
          MobileClient.newWithPersistedState(selfName: selfName, state: state),
    );
    return MinosCore._(client, secure, accessConfig);
  }

  /// Decide which [MobileClient] to hand back to callers at startup,
  /// recovering from a stale persisted snapshot when resume fails.
  ///
  /// The recovery branch matters because the Rust client retains the
  /// persisted device id even when the secret is no longer valid: a
  /// subsequent `pair` would otherwise re-use that identity against an
  /// authenticated row on the backend and be rejected with 401. Dropping
  /// the snapshot lets the next pair attempt mint a fresh device.
  ///
  /// Phase 8.9: WS startup is now gated on the persisted auth tuple. If
  /// the snapshot has paired-device fields but no `accessToken`, we hand
  /// back the rehydrated client *without* calling `resumePersistedSession`
  /// — the AuthController's stream listener will trigger the WS resume
  /// after the user logs in (`AuthAuthenticated`).
  @visibleForTesting
  static Future<MobileClient> resolveClient({
    required SecurePairingStore secure,
    required MobileClient Function() buildFresh,
    required MobileClient Function(PersistedPairingState) buildFromPersisted,
  }) async {
    final persisted = await secure.loadState();
    if (persisted == null) return buildFresh();

    final client = buildFromPersisted(persisted);
    if (persisted.accessToken == null) {
      // Paired but logged out. Don't attempt the WS — let the AuthController
      // drive resume after the user authenticates.
      return client;
    }
    try {
      await client.resumePersistedSession();
      return client;
    } catch (error) {
      if (_shouldDiscardPersistedState(error)) {
        await secure.clearAll();
        return buildFresh();
      }
      return client;
    }
  }

  @override
  Future<void> pairWithQrJson(String qrJson) async {
    await _client.pairWithQrJson(qrJson: _cfAccessConfig.applyToQrJson(qrJson));
    try {
      await _secure.saveState(await _client.persistedPairingState());
    } catch (error, stackTrace) {
      await _rollbackFailedPersistedPairSave();
      Error.throwWithStackTrace(error, stackTrace);
    }
  }

  @override
  Future<void> forgetPeer() async {
    await _client.forgetPeer();
    await _secure.clearAll();
  }

  @override
  Future<bool> hasPersistedPairing() async {
    return await _secure.loadState() != null;
  }

  @override
  Future<ListThreadsResponse> listThreads(ListThreadsParams params) =>
      _client.listThreads(req: params);

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) =>
      _client.readThread(req: params);

  @override
  Stream<ConnectionState> get connectionStates => _client.subscribeState();

  @override
  Stream<UiEventFrame> get uiEvents => _client.subscribeUiEvents();

  @override
  ConnectionState get currentConnectionState => _client.currentState();

  // ---- Auth forwarders ----

  @override
  Future<AuthSummary> register({
    required String email,
    required String password,
  }) async {
    final summary = await _client.register(email: email, password: password);
    await _onAuthLanded(summary.accountId);
    return summary;
  }

  @override
  Future<AuthSummary> login({
    required String email,
    required String password,
  }) async {
    final summary = await _client.login(email: email, password: password);
    await _onAuthLanded(summary.accountId);
    return summary;
  }

  @override
  Future<void> refreshSession() => _client.refreshSession();

  @override
  Future<void> logout() async {
    await _client.logout();
    // Mirror the Rust-side wipe into the Dart keychain so a cold relaunch
    // doesn't rehydrate the dead session. The pairing tuple is left
    // intact so the next account login on this device can reuse it.
    await _secure.clearAuth();
  }

  /// Cross-account migration + post-auth persistence (Phase 11.3).
  ///
  /// After a successful `register` / `login` we have to:
  ///
  /// 1. Drop the existing pairing if the previously persisted snapshot
  ///    belonged to a *different* account. The Mac-side device row is
  ///    account-scoped, so reusing the prior `DeviceSecret` against a
  ///    new account would be rejected on the next WS upgrade — better to
  ///    force the user through pairing now than surface a confusing 401
  ///    later.
  /// 2. Mirror the freshly minted auth tuple from the Rust core into the
  ///    Dart keychain so a cold relaunch can rehydrate
  ///    `auth_session` synchronously and the AuthController's first
  ///    frame is already `Authenticated`.
  ///
  /// Best-effort throughout: the Rust side is the source of truth for
  /// the live session, so a keychain write failure does not invalidate
  /// the in-memory login. The next pair-or-resume cycle will recover.
  Future<void> _onAuthLanded(String newAccountId) async {
    final priorAccountId = (await _secure.loadState())?.accountId;
    if (priorAccountId != null && priorAccountId != newAccountId) {
      // Stale pairing belongs to a different account — drop it so the
      // route gate flips to `pairing` for the new account.
      try {
        await forgetPeer();
      } catch (_) {
        // Best effort: even if the WS-side teardown fails, the next
        // pair_consume call mints a fresh device secret.
      }
    }
    try {
      await _secure.saveState(await _client.persistedPairingState());
    } catch (_) {
      // Same rationale as above — the in-memory session is the source
      // of truth; persistence is a cold-launch optimisation.
    }
  }

  // ---- Agent dispatch forwarders ----

  @override
  Future<StartAgentResponse> startAgent({
    required AgentName agent,
    required String prompt,
  }) => _client.startAgent(agent: agent, prompt: prompt);

  @override
  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) => _client.sendUserMessage(sessionId: sessionId, text: text);

  @override
  Future<void> stopAgent() => _client.stopAgent();

  // ---- Lifecycle forwarders ----

  @override
  void notifyForegrounded() => _client.notifyForegrounded();

  @override
  void notifyBackgrounded() => _client.notifyBackgrounded();

  @override
  Stream<AuthStateFrame> get authStates => _client.subscribeAuthState();

  @override
  Future<void> resumePersistedSession() => _client.resumePersistedSession();

  Future<void> _rollbackFailedPersistedPairSave() async {
    try {
      await _client.forgetPeer();
    } catch (_) {
      // Best effort: if the session is already gone we still want to wipe any
      // partially persisted keychain snapshot before surfacing the failure.
    }
    try {
      await _secure.clearAll();
    } catch (_) {
      // Preserve the original persistence failure; the next launch will still
      // treat any leftover partial snapshot as non-resumable.
    }
  }

  static bool _shouldDiscardPersistedState(Object error) {
    return error is MinosError_DeviceNotTrusted ||
        error is MinosError_Unauthorized ||
        error is MinosError_StoreCorrupt;
  }
}
