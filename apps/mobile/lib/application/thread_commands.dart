import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/flutter_log.dart';
import 'package:minos/data/repositories/thread_repository.dart';
import 'package:minos/src/rust/api/minos.dart';

final threadCommandsProvider = Provider<ThreadCommands>((ref) {
  return ThreadCommands(ref);
});

class ThreadCommands {
  ThreadCommands(this._ref);

  final Ref _ref;

  ThreadRepository get _repository => _ref.read(threadRepositoryProvider);

  Stream<UiEventFrame> get uiEvents => _repository.uiEvents;

  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) async {
    final effectiveSessionId = sessionId.isEmpty ? '<new>' : sessionId;
    logFlutterInfo(
      'thread_commands',
      'sendUserMessage requested sessionId=$effectiveSessionId textLength=${text.length}',
    );
    try {
      await _repository.sendUserMessage(sessionId: sessionId, text: text);
      logFlutterDebug(
        'thread_commands',
        'sendUserMessage accepted sessionId=$effectiveSessionId',
      );
    } catch (error, stackTrace) {
      logFlutterError(
        'thread_commands',
        'sendUserMessage failed sessionId=$effectiveSessionId textLength=${text.length}',
        error: error,
        stackTrace: stackTrace,
      );
      rethrow;
    }
  }

  Future<void> sendApprovalDecision({
    required String requestId,
    required String sessionId,
    required Map<String, dynamic> decision,
  }) async {
    logFlutterInfo(
      'thread_commands',
      'sendApprovalDecision requested sessionId=$sessionId requestId=$requestId',
    );
    try {
      await _repository.sendApprovalDecision(
        requestId: requestId,
        sessionId: sessionId,
        decision: decision,
      );
      logFlutterDebug(
        'thread_commands',
        'sendApprovalDecision accepted sessionId=$sessionId requestId=$requestId',
      );
    } catch (error, stackTrace) {
      logFlutterError(
        'thread_commands',
        'sendApprovalDecision failed sessionId=$sessionId requestId=$requestId',
        error: error,
        stackTrace: stackTrace,
      );
      rethrow;
    }
  }

  Future<void> respondOpencodeQuestion({
    required String sessionId,
    required String questionId,
    required List<List<String>> answers,
  }) async {
    logFlutterInfo(
      'thread_commands',
      'respondOpencodeQuestion requested sessionId=$sessionId questionId=$questionId',
    );
    try {
      await _repository.respondOpencodeQuestion(
        sessionId: sessionId,
        questionId: questionId,
        answers: answers,
      );
      logFlutterDebug(
        'thread_commands',
        'respondOpencodeQuestion accepted sessionId=$sessionId questionId=$questionId',
      );
    } catch (error, stackTrace) {
      logFlutterError(
        'thread_commands',
        'respondOpencodeQuestion failed sessionId=$sessionId questionId=$questionId',
        error: error,
        stackTrace: stackTrace,
      );
      rethrow;
    }
  }

  Future<void> deleteThread({required String sessionId}) async {
    logFlutterInfo(
      'thread_commands',
      'deleteThread requested sessionId=$sessionId',
    );
    try {
      await _repository.deleteThread(sessionId: sessionId);
      logFlutterDebug(
        'thread_commands',
        'deleteThread succeeded sessionId=$sessionId',
      );
    } catch (error, stackTrace) {
      logFlutterError(
        'thread_commands',
        'deleteThread failed sessionId=$sessionId',
        error: error,
        stackTrace: stackTrace,
      );
      rethrow;
    }
  }
}
