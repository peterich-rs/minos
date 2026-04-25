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

@ProviderFor(ThreadEvents)
final threadEventsProvider = ThreadEventsFamily._();

/// Loads the translated history for one thread and keeps it live by
/// listening to the backend's fan-out. Per-thread watermark dedup keeps
/// the view consistent with the backend's raw_events seq (spec §9.1).
final class ThreadEventsProvider
    extends $AsyncNotifierProvider<ThreadEvents, List<UiEventMessage>> {
  /// Loads the translated history for one thread and keeps it live by
  /// listening to the backend's fan-out. Per-thread watermark dedup keeps
  /// the view consistent with the backend's raw_events seq (spec §9.1).
  ThreadEventsProvider._({
    required ThreadEventsFamily super.from,
    required String super.argument,
  }) : super(
         retry: null,
         name: r'threadEventsProvider',
         isAutoDispose: true,
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

String _$threadEventsHash() => r'99c9d8462c82b33882caf8d5265e2c23cb6b5d4c';

/// Loads the translated history for one thread and keeps it live by
/// listening to the backend's fan-out. Per-thread watermark dedup keeps
/// the view consistent with the backend's raw_events seq (spec §9.1).

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
        isAutoDispose: true,
      );

  /// Loads the translated history for one thread and keeps it live by
  /// listening to the backend's fan-out. Per-thread watermark dedup keeps
  /// the view consistent with the backend's raw_events seq (spec §9.1).

  ThreadEventsProvider call(String threadId) =>
      ThreadEventsProvider._(argument: threadId, from: this);

  @override
  String toString() => r'threadEventsProvider';
}

/// Loads the translated history for one thread and keeps it live by
/// listening to the backend's fan-out. Per-thread watermark dedup keeps
/// the view consistent with the backend's raw_events seq (spec §9.1).

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
