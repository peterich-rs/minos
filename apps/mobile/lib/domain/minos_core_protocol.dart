import 'package:minos/src/rust/api/minos.dart';

/// Thin Dart-only contract around the frb-generated [MobileClient]. Letting
/// the application / presentation layers depend on this protocol (rather than
/// the Rust-owned opaque class) keeps the layers mockable in unit tests.
abstract class MinosCoreProtocol {
  /// Submit a raw QR JSON payload to the Rust core.
  Future<PairResponse> pairWithJson(String qrJson);

  /// Hot stream of [ConnectionState] transitions, starting with the current
  /// value.
  Stream<ConnectionState> get states;

  /// Synchronous snapshot of the current [ConnectionState].
  ConnectionState get current;
}
