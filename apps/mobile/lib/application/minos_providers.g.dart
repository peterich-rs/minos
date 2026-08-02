// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'minos_providers.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Hot stream of connection-state transitions sourced from the Rust core.
///
/// This is IM **account online** for this phone: live `/ws/client` to the hub.

@ProviderFor(connectionState)
final connectionStateProvider = ConnectionStateProvider._();

/// Hot stream of connection-state transitions sourced from the Rust core.
///
/// This is IM **account online** for this phone: live `/ws/client` to the hub.

final class ConnectionStateProvider
    extends
        $FunctionalProvider<
          AsyncValue<ConnectionState>,
          ConnectionState,
          Stream<ConnectionState>
        >
    with $FutureModifier<ConnectionState>, $StreamProvider<ConnectionState> {
  /// Hot stream of connection-state transitions sourced from the Rust core.
  ///
  /// This is IM **account online** for this phone: live `/ws/client` to the hub.
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
