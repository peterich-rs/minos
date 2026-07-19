import 'package:minos/data/repositories/thread_repository.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'thread_list_provider.g.dart';

/// Loads and caches the paged thread list. First build requests the
/// freshest 50 threads; [refresh] reruns `list_threads` with the same
/// params.
@Riverpod(keepAlive: false)
class ThreadList extends _$ThreadList {
  @override
  Future<List<ThreadSummary>> build() async {
    return ref.read(threadRepositoryProvider).listThreads();
  }

  Future<void> refresh() async {
    final previous = state;
    try {
      state = AsyncValue.data(
        await ref.read(threadRepositoryProvider).listThreads(),
      );
    } catch (error, stackTrace) {
      if (previous.hasValue) {
        state = previous;
        Error.throwWithStackTrace(error, stackTrace);
      }
      state = AsyncValue.error(error, stackTrace);
    }
  }
}
