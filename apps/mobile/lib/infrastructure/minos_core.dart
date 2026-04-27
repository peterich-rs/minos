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
  @visibleForTesting
  static Future<MobileClient> resolveClient({
    required SecurePairingStore secure,
    required MobileClient Function() buildFresh,
    required MobileClient Function(PersistedPairingState) buildFromPersisted,
  }) async {
    final persisted = await secure.loadState();
    if (persisted == null) return buildFresh();

    final client = buildFromPersisted(persisted);
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
  }) => _client.register(email: email, password: password);

  @override
  Future<AuthSummary> login({
    required String email,
    required String password,
  }) => _client.login(email: email, password: password);

  @override
  Future<void> refreshSession() => _client.refreshSession();

  @override
  Future<void> logout() => _client.logout();

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
