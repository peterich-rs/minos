import 'package:minos/src/rust/api/minos.dart';

/// Thin Dart-only contract around the frb-generated [MobileClient]. Letting
/// the application / presentation layers depend on this protocol (rather than
/// the Rust-owned opaque class) keeps the layers mockable in unit tests.
abstract class MinosCoreProtocol {
  /// Submit a raw QR v2 JSON payload to the Rust core. Completes when the
  /// `Pair` RPC returns; the Rust side persists the minted `DeviceSecret`
  /// in its pairing store before this future resolves.
  Future<void> pairWithQrJson(String qrJson);

  /// Forget the paired backend; drops credentials and tears down the WS.
  Future<void> forgetPeer();

  /// Whether the durable store contains enough state to represent an
  /// already-paired device, even if the current WebSocket is offline.
  Future<bool> hasPersistedPairing();

  /// Paged thread summaries for the paired agent-host.
  Future<ListThreadsResponse> listThreads(ListThreadsParams params);

  /// Translated UI event history for one thread.
  Future<ReadThreadResponse> readThread(ReadThreadParams params);

  /// Hot stream of [ConnectionState] transitions, starting with the current
  /// value.
  Stream<ConnectionState> get connectionStates;

  /// Hot stream of live [UiEventFrame]s fanned out by the backend.
  Stream<UiEventFrame> get uiEvents;

  /// Synchronous snapshot of the current [ConnectionState].
  ConnectionState get currentConnectionState;
}
