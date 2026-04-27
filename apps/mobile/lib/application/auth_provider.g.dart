// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'auth_provider.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Mirrors the Rust-side `AuthState` watch-channel into the Dart UI tier.
///
/// The Rust forwarder (`subscribe_auth_state`) emits the current cached
/// frame immediately on subscribe, then every transition. The provider
/// returns [AuthBootstrapping] from `build()`; the very first frame from
/// the stream replaces it on the next microtask. Components watching
/// this provider should treat [AuthBootstrapping] as "show splash".

@ProviderFor(AuthController)
final authControllerProvider = AuthControllerProvider._();

/// Mirrors the Rust-side `AuthState` watch-channel into the Dart UI tier.
///
/// The Rust forwarder (`subscribe_auth_state`) emits the current cached
/// frame immediately on subscribe, then every transition. The provider
/// returns [AuthBootstrapping] from `build()`; the very first frame from
/// the stream replaces it on the next microtask. Components watching
/// this provider should treat [AuthBootstrapping] as "show splash".
final class AuthControllerProvider
    extends $NotifierProvider<AuthController, AuthState> {
  /// Mirrors the Rust-side `AuthState` watch-channel into the Dart UI tier.
  ///
  /// The Rust forwarder (`subscribe_auth_state`) emits the current cached
  /// frame immediately on subscribe, then every transition. The provider
  /// returns [AuthBootstrapping] from `build()`; the very first frame from
  /// the stream replaces it on the next microtask. Components watching
  /// this provider should treat [AuthBootstrapping] as "show splash".
  AuthControllerProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'authControllerProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$authControllerHash();

  @$internal
  @override
  AuthController create() => AuthController();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(AuthState value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<AuthState>(value),
    );
  }
}

String _$authControllerHash() => r'b003b10b1359ad4475ebaad097469bafad608ab1';

/// Mirrors the Rust-side `AuthState` watch-channel into the Dart UI tier.
///
/// The Rust forwarder (`subscribe_auth_state`) emits the current cached
/// frame immediately on subscribe, then every transition. The provider
/// returns [AuthBootstrapping] from `build()`; the very first frame from
/// the stream replaces it on the next microtask. Components watching
/// this provider should treat [AuthBootstrapping] as "show splash".

abstract class _$AuthController extends $Notifier<AuthState> {
  AuthState build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<AuthState, AuthState>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<AuthState, AuthState>,
              AuthState,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}
