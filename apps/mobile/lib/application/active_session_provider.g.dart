// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'active_session_provider.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Drives the [ActiveSession] state machine off `core.uiEvents` and
/// the explicit `send/stop` actions.
///
/// While in [SessionSending] we bind the in-flight conversation to the
/// first UI frame that arrives, because the send path no longer returns a
/// daemon-issued `session_id` synchronously.

@ProviderFor(ActiveSessionController)
final activeSessionControllerProvider = ActiveSessionControllerProvider._();

/// Drives the [ActiveSession] state machine off `core.uiEvents` and
/// the explicit `send/stop` actions.
///
/// While in [SessionSending] we bind the in-flight conversation to the
/// first UI frame that arrives, because the send path no longer returns a
/// daemon-issued `session_id` synchronously.
final class ActiveSessionControllerProvider
    extends $NotifierProvider<ActiveSessionController, ActiveSession> {
  /// Drives the [ActiveSession] state machine off `core.uiEvents` and
  /// the explicit `send/stop` actions.
  ///
  /// While in [SessionSending] we bind the in-flight conversation to the
  /// first UI frame that arrives, because the send path no longer returns a
  /// daemon-issued `session_id` synchronously.
  ActiveSessionControllerProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'activeSessionControllerProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$activeSessionControllerHash();

  @$internal
  @override
  ActiveSessionController create() => ActiveSessionController();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(ActiveSession value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<ActiveSession>(value),
    );
  }
}

String _$activeSessionControllerHash() =>
    r'9b56fb51c3db5bb2768d545eeb1cf6825f20bccf';

/// Drives the [ActiveSession] state machine off `core.uiEvents` and
/// the explicit `send/stop` actions.
///
/// While in [SessionSending] we bind the in-flight conversation to the
/// first UI frame that arrives, because the send path no longer returns a
/// daemon-issued `session_id` synchronously.

abstract class _$ActiveSessionController extends $Notifier<ActiveSession> {
  ActiveSession build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<ActiveSession, ActiveSession>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<ActiveSession, ActiveSession>,
              ActiveSession,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}
