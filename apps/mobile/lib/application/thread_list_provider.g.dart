// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'thread_list_provider.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Loads and caches the paged thread list. First build requests the
/// freshest 50 threads; [refresh] reruns `list_threads` with the same
/// params.

@ProviderFor(ThreadList)
final threadListProvider = ThreadListProvider._();

/// Loads and caches the paged thread list. First build requests the
/// freshest 50 threads; [refresh] reruns `list_threads` with the same
/// params.
final class ThreadListProvider
    extends $AsyncNotifierProvider<ThreadList, List<ThreadSummary>> {
  /// Loads and caches the paged thread list. First build requests the
  /// freshest 50 threads; [refresh] reruns `list_threads` with the same
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

String _$threadListHash() => r'38fb6f23e74dcd59a9c0e7534703ab1a0bfa630f';

/// Loads and caches the paged thread list. First build requests the
/// freshest 50 threads; [refresh] reruns `list_threads` with the same
/// params.

abstract class _$ThreadList extends $AsyncNotifier<List<ThreadSummary>> {
  FutureOr<List<ThreadSummary>> build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref =
        this.ref as $Ref<AsyncValue<List<ThreadSummary>>, List<ThreadSummary>>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<AsyncValue<List<ThreadSummary>>, List<ThreadSummary>>,
              AsyncValue<List<ThreadSummary>>,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}
