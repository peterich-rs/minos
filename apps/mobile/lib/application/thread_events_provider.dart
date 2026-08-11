import 'package:minos/application/flutter_log.dart';
import 'package:minos/data/repositories/thread_repository.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'thread_events_provider.g.dart';

/// Loads the translated history for one session and keeps it live by
/// listening to the backend's fan-out. Per-thread watermark dedup keeps
/// the view consistent with the backend's raw_events seq.
///
/// `keepAlive: true` so navigating away from the chat page does not drop the
/// in-memory event list and live subscription. Re-entry then renders cached
/// history instantly instead of flashing a center spinner and re-fetching
/// from the daemon.
@Riverpod(keepAlive: true)
class ThreadEvents extends _$ThreadEvents {
  BigInt _watermark = BigInt.zero;

  @override
  Future<List<UiEventMessage>> build(String sessionId) async {
    final repository = ref.read(threadRepositoryProvider);
    logFlutterDebug(
      'thread_events',
      'load session events sessionId=$sessionId',
    );

    final resp = await _readInitialPage(repository, sessionId);
    logFlutterDebug(
      'thread_events',
      'loaded initial session events sessionId=$sessionId count=${resp.uiEvents.length}',
    );
    if (resp.nextSeq != null) {
      _watermark = resp.nextSeq! - BigInt.one;
    } else if (resp.uiEvents.isNotEmpty) {
      // No next page — seed watermark from the seq the page ended at. We
      // don't carry seq inside UiEventMessage itself; the backend's live
      // fan-out will include seq on each frame and we'll only accept
      // strictly-greater seqs from there.
      _watermark = BigInt.zero;
    }

    final sub = repository.uiEvents.listen(
      (frame) {
        if (frame.sessionId != sessionId) return;
        if (frame.seq <= _watermark) return;
        _watermark = frame.seq;
        final prev = state.asData?.value ?? const <UiEventMessage>[];
        state = AsyncValue.data([...prev, frame.ui]);
      },
      onError: (Object error, StackTrace stackTrace) {
        logFlutterWarn(
          'thread_events',
          'live event stream failed sessionId=$sessionId',
          error: error,
          stackTrace: stackTrace,
        );
        ref.invalidateSelf();
      },
      onDone: () {
        logFlutterInfo(
          'thread_events',
          'live event stream completed sessionId=$sessionId',
        );
        ref.invalidateSelf();
      },
    );
    ref.onDispose(sub.cancel);

    return resp.uiEvents;
  }

  Future<ReadSessionResponse> _readInitialPage(
    ThreadRepository repository,
    String sessionId,
  ) async {
    const maxAttempts = 8;

    for (var attempt = 0; attempt < maxAttempts; attempt++) {
      try {
        return await repository.readThread(sessionId: sessionId);
      } on MinosError_SessionNotFound catch (error, stackTrace) {
        if (attempt == maxAttempts - 1) {
          logFlutterError(
            'thread_events',
            'readThread exhausted retries sessionId=$sessionId attempts=$maxAttempts',
            error: error,
            stackTrace: stackTrace,
          );
          rethrow;
        }
        logFlutterWarn(
          'thread_events',
          'readThread retry scheduled sessionId=$sessionId attempt=${attempt + 1}',
          error: error,
          stackTrace: stackTrace,
        );
        await Future<void>.delayed(Duration(milliseconds: 150 * (attempt + 1)));
      }
    }

    throw MinosError.sessionNotFound(sessionId: sessionId);
  }
}
