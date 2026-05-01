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
/// Phase 11.3 + ADR-0020 — cross-account migration sequence:
///   1. `register` / `login` go through [MinosCore.register] /
///      [MinosCore.login], which compare the freshly minted
///      `account_id` against the prior persisted snapshot.
///   2. If the prior `account_id` differs, `MinosCore` clears the
///      cached peer display name so a stale label from the previous
///      account doesn't briefly flash in the partners list. The
///      server-side `account_mac_pairings` rows are already
///      account-scoped, so the next `listPairedHosts` sync naturally
///      yields the new account's Macs.
///   3. The first `Authenticated` frame fires; this controller calls
///      `resumePersistedSession()` which spins up the WS for the new
///      account.
///   4. The Partners tab calls `listPairedHosts` and shows whatever
///      Macs are paired to the new account (possibly empty → the user
///      taps "Add partner" to scan a QR).

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
/// Phase 11.3 + ADR-0020 — cross-account migration sequence:
///   1. `register` / `login` go through [MinosCore.register] /
///      [MinosCore.login], which compare the freshly minted
///      `account_id` against the prior persisted snapshot.
///   2. If the prior `account_id` differs, `MinosCore` clears the
///      cached peer display name so a stale label from the previous
///      account doesn't briefly flash in the partners list. The
///      server-side `account_mac_pairings` rows are already
///      account-scoped, so the next `listPairedHosts` sync naturally
///      yields the new account's Macs.
///   3. The first `Authenticated` frame fires; this controller calls
///      `resumePersistedSession()` which spins up the WS for the new
///      account.
///   4. The Partners tab calls `listPairedHosts` and shows whatever
///      Macs are paired to the new account (possibly empty → the user
///      taps "Add partner" to scan a QR).
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
  /// Phase 11.3 + ADR-0020 — cross-account migration sequence:
  ///   1. `register` / `login` go through [MinosCore.register] /
  ///      [MinosCore.login], which compare the freshly minted
  ///      `account_id` against the prior persisted snapshot.
  ///   2. If the prior `account_id` differs, `MinosCore` clears the
  ///      cached peer display name so a stale label from the previous
  ///      account doesn't briefly flash in the partners list. The
  ///      server-side `account_mac_pairings` rows are already
  ///      account-scoped, so the next `listPairedHosts` sync naturally
  ///      yields the new account's Macs.
  ///   3. The first `Authenticated` frame fires; this controller calls
  ///      `resumePersistedSession()` which spins up the WS for the new
  ///      account.
  ///   4. The Partners tab calls `listPairedHosts` and shows whatever
  ///      Macs are paired to the new account (possibly empty → the user
  ///      taps "Add partner" to scan a QR).
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
/// Phase 11.3 + ADR-0020 — cross-account migration sequence:
///   1. `register` / `login` go through [MinosCore.register] /
///      [MinosCore.login], which compare the freshly minted
///      `account_id` against the prior persisted snapshot.
///   2. If the prior `account_id` differs, `MinosCore` clears the
///      cached peer display name so a stale label from the previous
///      account doesn't briefly flash in the partners list. The
///      server-side `account_mac_pairings` rows are already
///      account-scoped, so the next `listPairedHosts` sync naturally
///      yields the new account's Macs.
///   3. The first `Authenticated` frame fires; this controller calls
///      `resumePersistedSession()` which spins up the WS for the new
///      account.
///   4. The Partners tab calls `listPairedHosts` and shows whatever
///      Macs are paired to the new account (possibly empty → the user
///      taps "Add partner" to scan a QR).

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
