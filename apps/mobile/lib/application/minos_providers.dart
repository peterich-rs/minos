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

/// Display name of the currently paired peer, sourced from the QR's
/// `host_display_name` at pair time. `null` when no pairing exists or
/// the name was never recorded (e.g. pairings made before this field
/// was added).
final peerDisplayNameProvider = FutureProvider<String?>((ref) {
  return ref.watch(minosCoreProvider).peerDisplayName();
});

final runtimeAgentDescriptorsProvider = FutureProvider<List<AgentDescriptor>>((
  ref,
) {
  return ref.watch(minosCoreProvider).listClis();
});

/// Paired Macs for the current account. Drives the Partners list. Refresh
/// happens via `ref.invalidate(pairedMacsProvider)` after a forget /
/// successful pair — there is no polling stream yet, the user can pull
/// the partners tab to refresh.
final pairedMacsProvider = FutureProvider<List<HostSummaryDto>>((ref) {
  return ref.watch(minosCoreProvider).listPairedHosts();
});

/// Routing target for `Forward` envelopes. `null` means no Mac is selected
/// — the daemon falls back to broadcast-style fan-out when this is unset.
@riverpod
class ActiveMac extends _$ActiveMac {
  @override
  Future<String?> build() {
    return ref.watch(minosCoreProvider).activeHost();
  }

  /// Set [macId] as the routing target. Updates state optimistically; if
  /// the FRB call fails the state surfaces the error and re-reads the
  /// core-side truth.
  Future<void> setActive(String macId) async {
    final previous = state;
    state = AsyncValue.data(macId);
    try {
      await ref.read(minosCoreProvider).setActiveHost(macId);
    } catch (e, st) {
      state = AsyncValue.error(e, st);
      try {
        state = AsyncValue.data(await ref.read(minosCoreProvider).activeHost());
      } catch (_) {
        state = previous;
      }
    }
  }

  /// Re-read the active mac from the core; used after a forget so the
  /// cached value doesn't point at a no-longer-paired Mac.
  Future<void> refresh() async {
    state = const AsyncValue.loading();
    try {
      state = AsyncValue.data(await ref.read(minosCoreProvider).activeHost());
    } catch (e, st) {
      state = AsyncValue.error(e, st);
    }
  }
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

/// Owns the pairing submission lifecycle. The outcome is a plain
/// `AsyncValue<bool>` (true on successful pair) — v2 pairing does not
/// return a typed response body to the caller.
@riverpod
class PairingController extends _$PairingController {
  @override
  FutureOr<bool> build() => false;

  /// Submit a raw QR JSON payload to the Rust core, updating [state] with
  /// loading / data / error as the call resolves. `displayName` is the
  /// `host_display_name` already extracted from the QR by the scanner UI;
  /// it is mirrored into the Dart secure store on success so the partners
  /// list can show the peer name instead of a generic "Agent Runtime".
  Future<void> submit(String qrJson, {String? displayName}) async {
    state = const AsyncValue.loading();
    try {
      final core = ref.read(minosCoreProvider);
      await core.pairWithQrJson(qrJson);
      try {
        await core.setPeerDisplayName(displayName);
      } catch (_) {
        // Best-effort: a keychain write failure here should not undo the
        // successful pair — the partner row will fall back to a generic
        // label until the name can be resolved another way.
      }
      ref.invalidate(hasPersistedPairingProvider);
      ref.invalidate(peerDisplayNameProvider);
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
