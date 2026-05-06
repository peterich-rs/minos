import 'package:riverpod_annotation/riverpod_annotation.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/src/rust/api/minos.dart';

part 'thread_list_provider.g.dart';

/// Loads and caches the paged thread list. First build requests the
/// freshest 50 threads; [refresh] reruns `list_threads` with the same
/// params.
@Riverpod(keepAlive: false)
class ThreadList extends _$ThreadList {
  @override
  Future<List<ThreadSummary>> build() async {
    final core = ref.read(minosCoreProvider);
    final resp = await core.listThreads(const ListThreadsParams(limit: 50));
    return resp.threads;
  }

  Future<void> refresh() async {
    final previous = state;
    try {
      final core = ref.read(minosCoreProvider);
      final resp = await core.listThreads(const ListThreadsParams(limit: 50));
      state = AsyncValue.data(resp.threads);
    } catch (error, stackTrace) {
      if (previous.hasValue) {
        state = previous;
        Error.throwWithStackTrace(error, stackTrace);
      }
      state = AsyncValue.error(error, stackTrace);
    }
  }
}
