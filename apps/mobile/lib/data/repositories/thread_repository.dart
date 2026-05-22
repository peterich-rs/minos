import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

final threadRepositoryProvider = Provider<ThreadRepository>((ref) {
  return ThreadRepository(ref.watch(minosCoreServiceProvider));
});

class ThreadRepository {
  const ThreadRepository(this._core);

  final MinosCoreProtocol _core;

  Stream<UiEventFrame> get uiEvents => _core.uiEvents;

  Future<List<ThreadSummary>> listThreads({int limit = 50}) async {
    final response = await _core.listThreads(ListThreadsParams(limit: limit));
    return response.threads;
  }

  Future<ReadThreadResponse> readThread({
    required String threadId,
    int limit = 500,
  }) {
    return _core.readThread(ReadThreadParams(threadId: threadId, limit: limit));
  }

  Future<void> interruptThread({required String threadId}) {
    return _core.interruptThread(threadId: threadId);
  }

  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) {
    return _core.sendUserMessage(sessionId: sessionId, text: text);
  }

  Future<void> sendApprovalDecision({
    required String requestId,
    required String threadId,
    required Map<String, dynamic> decision,
  }) {
    return _core.sendApprovalDecision(
      requestId: requestId,
      threadId: threadId,
      decision: decision,
    );
  }

  Future<void> deleteThread({required String threadId}) {
    return _core.deleteThread(threadId: threadId);
  }
}
