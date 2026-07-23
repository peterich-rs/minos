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

  Future<List<SessionSummary>> listThreads({int limit = 50}) async {
    final response = await _core.listThreads(ListSessionsParams(limit: limit));
    return response.sessions;
  }

  Future<ReadSessionResponse> readThread({
    required String sessionId,
    int limit = 500,
  }) {
    return _core.readThread(ReadSessionParams(sessionId: sessionId, limit: limit));
  }

  Future<void> interruptThread({required String sessionId}) {
    return _core.interruptThread(sessionId: sessionId);
  }

  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) {
    return _core.sendUserMessage(sessionId: sessionId, text: text);
  }

  Future<void> sendApprovalDecision({
    required String requestId,
    required String sessionId,
    required Map<String, dynamic> decision,
  }) {
    return _core.sendApprovalDecision(
      requestId: requestId,
      sessionId: sessionId,
      decision: decision,
    );
  }

  Future<void> respondOpencodeQuestion({
    required String sessionId,
    required String questionId,
    required List<List<String>> answers,
  }) {
    return _core.respondOpencodeQuestion(
      sessionId: sessionId,
      questionId: questionId,
      answers: answers,
    );
  }

  Future<void> deleteThread({required String sessionId}) {
    return _core.deleteThread(sessionId: sessionId);
  }
}
