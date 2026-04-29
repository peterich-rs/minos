import 'package:riverpod_annotation/riverpod_annotation.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/src/rust/api/minos.dart';

part 'thread_events_provider.g.dart';

/// Loads the translated history for one thread and keeps it live by
/// listening to the backend's fan-out. Per-thread watermark dedup keeps
/// the view consistent with the backend's raw_events seq (spec §9.1).
@Riverpod(keepAlive: false)
class ThreadEvents extends _$ThreadEvents {
  BigInt _watermark = BigInt.zero;

  @override
  Future<List<UiEventMessage>> build(String threadId) async {
    final core = ref.read(minosCoreProvider);

    final resp = await _readInitialPage(core, threadId);
    if (resp.nextSeq != null) {
      _watermark = resp.nextSeq! - BigInt.one;
    } else if (resp.uiEvents.isNotEmpty) {
      // No next page — seed watermark from the seq the page ended at. We
      // don't carry seq inside UiEventMessage itself; the backend's live
      // fan-out will include seq on each frame and we'll only accept
      // strictly-greater seqs from there.
      _watermark = BigInt.zero;
    }

    final sub = core.uiEvents.listen((frame) {
      if (frame.threadId != threadId) return;
      if (frame.seq <= _watermark) return;
      _watermark = frame.seq;
      final prev = state.asData?.value ?? const <UiEventMessage>[];
      state = AsyncValue.data([...prev, frame.ui]);
    });
    ref.onDispose(sub.cancel);

    return resp.uiEvents;
  }

  Future<ReadThreadResponse> _readInitialPage(
    dynamic core,
    String threadId,
  ) async {
    const maxAttempts = 8;

    for (var attempt = 0; attempt < maxAttempts; attempt++) {
      try {
        return await core.readThread(
          ReadThreadParams(threadId: threadId, limit: 500),
        );
      } on MinosError_ThreadNotFound {
        if (attempt == maxAttempts - 1) {
          rethrow;
        }
        await Future<void>.delayed(Duration(milliseconds: 150 * (attempt + 1)));
      }
    }

    throw MinosError.threadNotFound(threadId: threadId);
  }
}
