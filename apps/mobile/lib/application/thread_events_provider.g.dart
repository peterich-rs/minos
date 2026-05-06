// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'thread_events_provider.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Loads the translated history for one thread and keeps it live by
/// listening to the backend's fan-out. Per-thread watermark dedup keeps
/// the view consistent with the backend's raw_events seq (spec §9.1).
///
/// `keepAlive: true` so navigating away from the chat page does not drop the
/// in-memory event list and live subscription. Re-entry then renders cached
/// history instantly instead of flashing a center spinner and re-fetching
/// from the daemon.

@ProviderFor(ThreadEvents)
final threadEventsProvider = ThreadEventsFamily._();

/// Loads the translated history for one thread and keeps it live by
/// listening to the backend's fan-out. Per-thread watermark dedup keeps
/// the view consistent with the backend's raw_events seq (spec §9.1).
///
/// `keepAlive: true` so navigating away from the chat page does not drop the
/// in-memory event list and live subscription. Re-entry then renders cached
/// history instantly instead of flashing a center spinner and re-fetching
/// from the daemon.
final class ThreadEventsProvider
    extends $AsyncNotifierProvider<ThreadEvents, List<UiEventMessage>> {
  /// Loads the translated history for one thread and keeps it live by
  /// listening to the backend's fan-out. Per-thread watermark dedup keeps
  /// the view consistent with the backend's raw_events seq (spec §9.1).
  ///
  /// `keepAlive: true` so navigating away from the chat page does not drop the
  /// in-memory event list and live subscription. Re-entry then renders cached
  /// history instantly instead of flashing a center spinner and re-fetching
  /// from the daemon.
  ThreadEventsProvider._({
    required ThreadEventsFamily super.from,
    required String super.argument,
  }) : super(
         retry: null,
         name: r'threadEventsProvider',
         isAutoDispose: false,
         dependencies: null,
         $allTransitiveDependencies: null,
       );

  @override
  String debugGetCreateSourceHash() => _$threadEventsHash();

  @override
  String toString() {
    return r'threadEventsProvider'
        ''
        '($argument)';
  }

  @$internal
  @override
  ThreadEvents create() => ThreadEvents();

  @override
  bool operator ==(Object other) {
    return other is ThreadEventsProvider && other.argument == argument;
  }

  @override
  int get hashCode {
    return argument.hashCode;
  }
}

String _$threadEventsHash() => r'055a9b14f3dbcc182db87ff8ebbb30e7e221bf95';

/// Loads the translated history for one thread and keeps it live by
/// listening to the backend's fan-out. Per-thread watermark dedup keeps
/// the view consistent with the backend's raw_events seq (spec §9.1).
///
/// `keepAlive: true` so navigating away from the chat page does not drop the
/// in-memory event list and live subscription. Re-entry then renders cached
/// history instantly instead of flashing a center spinner and re-fetching
/// from the daemon.

final class ThreadEventsFamily extends $Family
    with
        $ClassFamilyOverride<
          ThreadEvents,
          AsyncValue<List<UiEventMessage>>,
          List<UiEventMessage>,
          FutureOr<List<UiEventMessage>>,
          String
        > {
  ThreadEventsFamily._()
    : super(
        retry: null,
        name: r'threadEventsProvider',
        dependencies: null,
        $allTransitiveDependencies: null,
        isAutoDispose: false,
      );

  /// Loads the translated history for one thread and keeps it live by
  /// listening to the backend's fan-out. Per-thread watermark dedup keeps
  /// the view consistent with the backend's raw_events seq (spec §9.1).
  ///
  /// `keepAlive: true` so navigating away from the chat page does not drop the
  /// in-memory event list and live subscription. Re-entry then renders cached
  /// history instantly instead of flashing a center spinner and re-fetching
  /// from the daemon.

  ThreadEventsProvider call(String threadId) =>
      ThreadEventsProvider._(argument: threadId, from: this);

  @override
  String toString() => r'threadEventsProvider';
}

/// Loads the translated history for one thread and keeps it live by
/// listening to the backend's fan-out. Per-thread watermark dedup keeps
/// the view consistent with the backend's raw_events seq (spec §9.1).
///
/// `keepAlive: true` so navigating away from the chat page does not drop the
/// in-memory event list and live subscription. Re-entry then renders cached
/// history instantly instead of flashing a center spinner and re-fetching
/// from the daemon.

abstract class _$ThreadEvents extends $AsyncNotifier<List<UiEventMessage>> {
  late final _$args = ref.$arg as String;
  String get threadId => _$args;

  FutureOr<List<UiEventMessage>> build(String threadId);
  @$mustCallSuper
  @override
  void runBuild() {
    final ref =
        this.ref
            as $Ref<AsyncValue<List<UiEventMessage>>, List<UiEventMessage>>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<
                AsyncValue<List<UiEventMessage>>,
                List<UiEventMessage>
              >,
              AsyncValue<List<UiEventMessage>>,
              Object?,
              Object?
            >;
    element.handleCreate(ref, () => build(_$args));
  }
}
