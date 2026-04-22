import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/src/rust/frb_generated.dart';

/// The one place in the Dart codebase allowed to import the frb-generated
/// [MobileClient]. Everything above this layer depends on
/// [MinosCoreProtocol] instead.
class MinosCore implements MinosCoreProtocol {
  MinosCore._(this._client);

  final MobileClient _client;

  /// Construct and initialize the core. Must be awaited before any other
  /// Riverpod provider reads it.
  static Future<MinosCore> init({
    required String selfName,
    required String logDir,
  }) async {
    await RustLib.init();
    await initLogging(logDir: logDir);
    final client = MobileClient(selfName: selfName);
    return MinosCore._(client);
  }

  @override
  Future<PairResponse> pairWithJson(String qrJson) =>
      _client.pairWithJson(qrJson: qrJson);

  @override
  Stream<ConnectionState> get states => _client.subscribeState();

  @override
  ConnectionState get current => _client.currentState();
}
