// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'thread_list_provider.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Loads and caches the paged session list. First build requests the
/// freshest 50 sessions; [refresh] reruns `list_sessions` with the same
/// params.

@ProviderFor(ThreadList)
final threadListProvider = ThreadListProvider._();

/// Loads and caches the paged session list. First build requests the
/// freshest 50 sessions; [refresh] reruns `list_sessions` with the same
/// params.
final class ThreadListProvider
    extends $AsyncNotifierProvider<ThreadList, List<SessionSummary>> {
  /// Loads and caches the paged session list. First build requests the
  /// freshest 50 sessions; [refresh] reruns `list_sessions` with the same
  /// params.
  ThreadListProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'threadListProvider',
        isAutoDispose: true,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$threadListHash();

  @$internal
  @override
  ThreadList create() => ThreadList();
}

String _$threadListHash() => r'a5101bb48da9838337a8659ebf38f551901c0219';

/// Loads and caches the paged session list. First build requests the
/// freshest 50 sessions; [refresh] reruns `list_sessions` with the same
/// params.

abstract class _$ThreadList extends $AsyncNotifier<List<SessionSummary>> {
  FutureOr<List<SessionSummary>> build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref =
        this.ref
            as $Ref<AsyncValue<List<SessionSummary>>, List<SessionSummary>>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<
                AsyncValue<List<SessionSummary>>,
                List<SessionSummary>
              >,
              AsyncValue<List<SessionSummary>>,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}
