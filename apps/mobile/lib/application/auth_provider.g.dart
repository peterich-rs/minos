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
///
/// Phase 8.9: on the first `Authenticated` transition, the controller
/// also kicks the Rust WS reconnect path via `resumePersistedSession()`
/// so the chat surface lights up without a separate trigger.
///
/// Phase 11.3 — cross-account migration sequence (manual smoke #7+8):
///   1. `register` / `login` go through [MinosCore.register] /
///      [MinosCore.login], which compare the freshly minted
///      `account_id` against the prior persisted snapshot.
///   2. If the prior `account_id` differs (a different account is
///      logging in on a previously paired device), `MinosCore` calls
///      `forgetPeer()` BEFORE persisting the new auth tuple. This wipes
///      the device-side pairing tuple so the route gate flips to
///      `pairing` for the new account.
///   3. The first `Authenticated` frame fires; this controller calls
///      `resumePersistedSession()` which is a no-op when pairing was
///      just dropped (and any error is swallowed).
///   4. The router observes `hasPersistedPairing == false` and shows
///      [PairingPage] for the new account to scan a fresh QR.

@ProviderFor(AuthController)
final authControllerProvider = AuthControllerProvider._();

/// Mirrors the Rust-side `AuthState` watch-channel into the Dart UI tier.
///
/// The Rust forwarder (`subscribe_auth_state`) emits the current cached
/// frame immediately on subscribe, then every transition. The provider
/// returns [AuthBootstrapping] from `build()`; the very first frame from
/// the stream replaces it on the next microtask. Components watching
/// this provider should treat [AuthBootstrapping] as "show splash".
///
/// Phase 8.9: on the first `Authenticated` transition, the controller
/// also kicks the Rust WS reconnect path via `resumePersistedSession()`
/// so the chat surface lights up without a separate trigger.
///
/// Phase 11.3 — cross-account migration sequence (manual smoke #7+8):
///   1. `register` / `login` go through [MinosCore.register] /
///      [MinosCore.login], which compare the freshly minted
///      `account_id` against the prior persisted snapshot.
///   2. If the prior `account_id` differs (a different account is
///      logging in on a previously paired device), `MinosCore` calls
///      `forgetPeer()` BEFORE persisting the new auth tuple. This wipes
///      the device-side pairing tuple so the route gate flips to
///      `pairing` for the new account.
///   3. The first `Authenticated` frame fires; this controller calls
///      `resumePersistedSession()` which is a no-op when pairing was
///      just dropped (and any error is swallowed).
///   4. The router observes `hasPersistedPairing == false` and shows
///      [PairingPage] for the new account to scan a fresh QR.
final class AuthControllerProvider
    extends $NotifierProvider<AuthController, AuthState> {
  /// Mirrors the Rust-side `AuthState` watch-channel into the Dart UI tier.
  ///
  /// The Rust forwarder (`subscribe_auth_state`) emits the current cached
  /// frame immediately on subscribe, then every transition. The provider
  /// returns [AuthBootstrapping] from `build()`; the very first frame from
  /// the stream replaces it on the next microtask. Components watching
  /// this provider should treat [AuthBootstrapping] as "show splash".
  ///
  /// Phase 8.9: on the first `Authenticated` transition, the controller
  /// also kicks the Rust WS reconnect path via `resumePersistedSession()`
  /// so the chat surface lights up without a separate trigger.
  ///
  /// Phase 11.3 — cross-account migration sequence (manual smoke #7+8):
  ///   1. `register` / `login` go through [MinosCore.register] /
  ///      [MinosCore.login], which compare the freshly minted
  ///      `account_id` against the prior persisted snapshot.
  ///   2. If the prior `account_id` differs (a different account is
  ///      logging in on a previously paired device), `MinosCore` calls
  ///      `forgetPeer()` BEFORE persisting the new auth tuple. This wipes
  ///      the device-side pairing tuple so the route gate flips to
  ///      `pairing` for the new account.
  ///   3. The first `Authenticated` frame fires; this controller calls
  ///      `resumePersistedSession()` which is a no-op when pairing was
  ///      just dropped (and any error is swallowed).
  ///   4. The router observes `hasPersistedPairing == false` and shows
  ///      [PairingPage] for the new account to scan a fresh QR.
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

String _$authControllerHash() => r'3506eef1100b655e5d50d8308afe305401dc8822';

/// Mirrors the Rust-side `AuthState` watch-channel into the Dart UI tier.
///
/// The Rust forwarder (`subscribe_auth_state`) emits the current cached
/// frame immediately on subscribe, then every transition. The provider
/// returns [AuthBootstrapping] from `build()`; the very first frame from
/// the stream replaces it on the next microtask. Components watching
/// this provider should treat [AuthBootstrapping] as "show splash".
///
/// Phase 8.9: on the first `Authenticated` transition, the controller
/// also kicks the Rust WS reconnect path via `resumePersistedSession()`
/// so the chat surface lights up without a separate trigger.
///
/// Phase 11.3 — cross-account migration sequence (manual smoke #7+8):
///   1. `register` / `login` go through [MinosCore.register] /
///      [MinosCore.login], which compare the freshly minted
///      `account_id` against the prior persisted snapshot.
///   2. If the prior `account_id` differs (a different account is
///      logging in on a previously paired device), `MinosCore` calls
///      `forgetPeer()` BEFORE persisting the new auth tuple. This wipes
///      the device-side pairing tuple so the route gate flips to
///      `pairing` for the new account.
///   3. The first `Authenticated` frame fires; this controller calls
///      `resumePersistedSession()` which is a no-op when pairing was
///      just dropped (and any error is swallowed).
///   4. The router observes `hasPersistedPairing == false` and shows
///      [PairingPage] for the new account to scan a fresh QR.

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
