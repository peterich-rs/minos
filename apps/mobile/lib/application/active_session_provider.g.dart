// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'active_session_provider.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Drives the [ActiveSession] state machine off `core.uiEvents` and
/// the explicit `start/send/stop` actions.
///
/// We intentionally only react to events whose `threadId` matches our
/// current `SessionStreaming.threadId` — other threads' fan-out frames
/// (e.g. a paired Mac running an unrelated session) must not poison the
/// mobile-side machine.

@ProviderFor(ActiveSessionController)
final activeSessionControllerProvider = ActiveSessionControllerProvider._();

/// Drives the [ActiveSession] state machine off `core.uiEvents` and
/// the explicit `start/send/stop` actions.
///
/// We intentionally only react to events whose `threadId` matches our
/// current `SessionStreaming.threadId` — other threads' fan-out frames
/// (e.g. a paired Mac running an unrelated session) must not poison the
/// mobile-side machine.
final class ActiveSessionControllerProvider
    extends $NotifierProvider<ActiveSessionController, ActiveSession> {
  /// Drives the [ActiveSession] state machine off `core.uiEvents` and
  /// the explicit `start/send/stop` actions.
  ///
  /// We intentionally only react to events whose `threadId` matches our
  /// current `SessionStreaming.threadId` — other threads' fan-out frames
  /// (e.g. a paired Mac running an unrelated session) must not poison the
  /// mobile-side machine.
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
    r'bacd67127466f617147ca784e9a751fb9a6ef637';

/// Drives the [ActiveSession] state machine off `core.uiEvents` and
/// the explicit `start/send/stop` actions.
///
/// We intentionally only react to events whose `threadId` matches our
/// current `SessionStreaming.threadId` — other threads' fan-out frames
/// (e.g. a paired Mac running an unrelated session) must not poison the
/// mobile-side machine.

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
