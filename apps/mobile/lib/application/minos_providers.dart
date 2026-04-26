import 'package:permission_handler/permission_handler.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart' show FutureProvider;
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
  return ref.watch(minosCoreProvider).connectionStates;
}

final hasPersistedPairingProvider = FutureProvider<bool>((ref) {
  return ref.watch(minosCoreProvider).hasPersistedPairing();
});

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

/// Owns the pairing submission lifecycle. The outcome is a plain
/// `AsyncValue<bool>` (true on successful pair) — v2 pairing does not
/// return a typed response body to the caller.
@riverpod
class PairingController extends _$PairingController {
  @override
  FutureOr<bool> build() => false;

  /// Submit a raw QR JSON payload to the Rust core, updating [state] with
  /// loading / data / error as the call resolves.
  Future<void> submit(String qrJson) async {
    state = const AsyncValue.loading();
    try {
      await ref.read(minosCoreProvider).pairWithQrJson(qrJson);
      ref.invalidate(hasPersistedPairingProvider);
      state = const AsyncValue.data(true);
    } on MinosError catch (e, st) {
      state = AsyncValue.error(e, st);
    } catch (e, st) {
      // Non-MinosError (e.g. frb PanicException, raw StateError on missing
      // RustLib.init). Keep the UI out of the stuck-loading state.
      state = AsyncValue.error(e, st);
    }
  }
}
