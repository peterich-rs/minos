import 'package:permission_handler/permission_handler.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

part 'minos_providers.g.dart';

/// Root provider for the Rust core. Must be overridden in `main()` with a
/// concrete [MinosCore] instance once `init()` has completed.
@Riverpod(keepAlive: true)
MinosCoreProtocol minosCore(Ref ref) {
  throw UnimplementedError(
    'minosCoreProvider must be overridden in main() with a concrete '
    'MinosCore instance',
  );
}

/// Hot stream of connection-state transitions sourced from the Rust core.
@Riverpod(keepAlive: true)
Stream<ConnectionState> connectionState(Ref ref) {
  return ref.watch(minosCoreProvider).states;
}

/// Camera permission status + action helpers. The notifier is the single
/// source of truth for the permission state driving the pairing UI.
@riverpod
class CameraPermission extends _$CameraPermission {
  @override
  Future<PermissionStatus> build() => check();

  /// Re-read the current permission status from the OS.
  Future<PermissionStatus> check() async {
    final status = await Permission.camera.status;
    state = AsyncValue.data(status);
    return status;
  }

  /// Trigger the OS permission prompt.
  Future<PermissionStatus> request() async {
    state = const AsyncValue.loading();
    final status = await Permission.camera.request();
    state = AsyncValue.data(status);
    return status;
  }

  /// Open the iOS Settings app so the user can grant a permanently-denied
  /// permission.
  Future<bool> openSettings() => openAppSettings();
}

/// Owns the pairing submission lifecycle. Exposes the latest [PairResponse]
/// (or error) so the router can transition to [HomePage] on success.
@riverpod
class PairingController extends _$PairingController {
  @override
  FutureOr<PairResponse?> build() => null;

  /// Submit a raw QR JSON payload to the Rust core, updating [state] with
  /// loading / data / error as the call resolves.
  Future<void> submit(String qrJson) async {
    state = const AsyncValue.loading();
    try {
      final response = await ref.read(minosCoreProvider).pairWithJson(qrJson);
      state = AsyncValue.data(response);
    } on MinosError catch (e, st) {
      state = AsyncValue.error(e, st);
    }
  }
}
