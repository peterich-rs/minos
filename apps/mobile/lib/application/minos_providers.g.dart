// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'minos_providers.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Root provider for the Rust core. Must be overridden in `main()` with a
/// concrete [MinosCore] instance once `init()` has completed.

@ProviderFor(minosCore)
final minosCoreProvider = MinosCoreProvider._();

/// Root provider for the Rust core. Must be overridden in `main()` with a
/// concrete [MinosCore] instance once `init()` has completed.

final class MinosCoreProvider
    extends
        $FunctionalProvider<
          MinosCoreProtocol,
          MinosCoreProtocol,
          MinosCoreProtocol
        >
    with $Provider<MinosCoreProtocol> {
  /// Root provider for the Rust core. Must be overridden in `main()` with a
  /// concrete [MinosCore] instance once `init()` has completed.
  MinosCoreProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'minosCoreProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$minosCoreHash();

  @$internal
  @override
  $ProviderElement<MinosCoreProtocol> $createElement(
    $ProviderPointer pointer,
  ) => $ProviderElement(pointer);

  @override
  MinosCoreProtocol create(Ref ref) {
    return minosCore(ref);
  }

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(MinosCoreProtocol value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<MinosCoreProtocol>(value),
    );
  }
}

String _$minosCoreHash() => r'5ec8eda43e87c21ec080a15cb9fb884ca0e18d03';

/// Hot stream of connection-state transitions sourced from the Rust core.

@ProviderFor(connectionState)
final connectionStateProvider = ConnectionStateProvider._();

/// Hot stream of connection-state transitions sourced from the Rust core.

final class ConnectionStateProvider
    extends
        $FunctionalProvider<
          AsyncValue<ConnectionState>,
          ConnectionState,
          Stream<ConnectionState>
        >
    with $FutureModifier<ConnectionState>, $StreamProvider<ConnectionState> {
  /// Hot stream of connection-state transitions sourced from the Rust core.
  ConnectionStateProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'connectionStateProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$connectionStateHash();

  @$internal
  @override
  $StreamProviderElement<ConnectionState> $createElement(
    $ProviderPointer pointer,
  ) => $StreamProviderElement(pointer);

  @override
  Stream<ConnectionState> create(Ref ref) {
    return connectionState(ref);
  }
}

String _$connectionStateHash() => r'7e2c58fcec3f59ae8890c0d5343a75bac0330ed8';

/// Camera permission status + action helpers. The notifier is the single
/// source of truth for the permission state driving the pairing UI.

@ProviderFor(CameraPermission)
final cameraPermissionProvider = CameraPermissionProvider._();

/// Camera permission status + action helpers. The notifier is the single
/// source of truth for the permission state driving the pairing UI.
final class CameraPermissionProvider
    extends $AsyncNotifierProvider<CameraPermission, PermissionStatus> {
  /// Camera permission status + action helpers. The notifier is the single
  /// source of truth for the permission state driving the pairing UI.
  CameraPermissionProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'cameraPermissionProvider',
        isAutoDispose: true,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$cameraPermissionHash();

  @$internal
  @override
  CameraPermission create() => CameraPermission();
}

String _$cameraPermissionHash() => r'cc4c7ba42f22844f4973e8045294581b0a300c89';

/// Camera permission status + action helpers. The notifier is the single
/// source of truth for the permission state driving the pairing UI.

abstract class _$CameraPermission extends $AsyncNotifier<PermissionStatus> {
  FutureOr<PermissionStatus> build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref =
        this.ref as $Ref<AsyncValue<PermissionStatus>, PermissionStatus>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<AsyncValue<PermissionStatus>, PermissionStatus>,
              AsyncValue<PermissionStatus>,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}

/// Owns the pairing submission lifecycle. The outcome is a plain
/// `AsyncValue<bool>` (true on successful pair) — v2 pairing does not
/// return a typed response body to the caller.

@ProviderFor(PairingController)
final pairingControllerProvider = PairingControllerProvider._();

/// Owns the pairing submission lifecycle. The outcome is a plain
/// `AsyncValue<bool>` (true on successful pair) — v2 pairing does not
/// return a typed response body to the caller.
final class PairingControllerProvider
    extends $AsyncNotifierProvider<PairingController, bool> {
  /// Owns the pairing submission lifecycle. The outcome is a plain
  /// `AsyncValue<bool>` (true on successful pair) — v2 pairing does not
  /// return a typed response body to the caller.
  PairingControllerProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'pairingControllerProvider',
        isAutoDispose: true,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$pairingControllerHash();

  @$internal
  @override
  PairingController create() => PairingController();
}

String _$pairingControllerHash() => r'175a1146f869b538ab5fd62cf1358baa51936ae6';

/// Owns the pairing submission lifecycle. The outcome is a plain
/// `AsyncValue<bool>` (true on successful pair) — v2 pairing does not
/// return a typed response body to the caller.

abstract class _$PairingController extends $AsyncNotifier<bool> {
  FutureOr<bool> build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<AsyncValue<bool>, bool>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<AsyncValue<bool>, bool>,
              AsyncValue<bool>,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}
