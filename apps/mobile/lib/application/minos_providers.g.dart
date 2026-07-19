// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'minos_providers.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
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

String _$connectionStateHash() => r'976528f1afea84dc282e0b373ef213a6d7bfe773';

/// Routing target for `Forward` envelopes. `null` means no Mac is selected
/// — the daemon falls back to broadcast-style fan-out when this is unset.

@ProviderFor(ActiveMac)
final activeMacProvider = ActiveMacProvider._();

/// Routing target for `Forward` envelopes. `null` means no Mac is selected
/// — the daemon falls back to broadcast-style fan-out when this is unset.
final class ActiveMacProvider
    extends $AsyncNotifierProvider<ActiveMac, String?> {
  /// Routing target for `Forward` envelopes. `null` means no Mac is selected
  /// — the daemon falls back to broadcast-style fan-out when this is unset.
  ActiveMacProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'activeMacProvider',
        isAutoDispose: true,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$activeMacHash();

  @$internal
  @override
  ActiveMac create() => ActiveMac();
}

String _$activeMacHash() => r'1ecc8a916b71d3342a6629a74bee1ef546322a41';

/// Routing target for `Forward` envelopes. `null` means no Mac is selected
/// — the daemon falls back to broadcast-style fan-out when this is unset.

abstract class _$ActiveMac extends $AsyncNotifier<String?> {
  FutureOr<String?> build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<AsyncValue<String?>, String?>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<AsyncValue<String?>, String?>,
              AsyncValue<String?>,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}

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

String _$pairingControllerHash() => r'119bdabe0164366c449ffc418dd9ea8e1f8b765c';

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
