import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/infrastructure/secure_pairing_store.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/src/rust/frb_generated.dart';

/// The one place in the Dart codebase allowed to import the frb-generated
/// [MobileClient]. Everything above this layer depends on
/// [MinosCoreProtocol] instead.
class MinosCore implements MinosCoreProtocol {
  MinosCore._(this._client, this._secure);

  final MobileClient _client;
  final SecurePairingStore _secure;

  /// Construct and initialize the core. Must be awaited before any other
  /// Riverpod provider reads it.
  static Future<MinosCore> init({
    required String selfName,
    required String logDir,
    SecurePairingStore? secureStore,
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
    final client = MobileClient(selfName: selfName);
    return MinosCore._(client, secureStore ?? SecurePairingStore());
  }

  @override
  Future<void> pairWithQrJson(String qrJson) async {
    await _client.pairWithQrJson(qrJson: qrJson);
    // Mirror the QR's persistable fields into the Keychain. The Rust side
    // holds them in its (in-memory) pairing store for the life of the
    // process; the Keychain copy is what survives app restarts so the
    // next `MobileClient` can pick up where we left off.
    await _secure.saveFromQrJson(qrJson);
  }

  @override
  Future<void> forgetPeer() async {
    await _client.forgetPeer();
    await _secure.clearAll();
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
}
