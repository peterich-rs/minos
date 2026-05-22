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
    required String threadId,
    required Map<String, dynamic> decision,
  }) async {
    logFlutterInfo(
      'thread_commands',
      'sendApprovalDecision requested threadId=$threadId requestId=$requestId',
    );
    try {
      await _repository.sendApprovalDecision(
        requestId: requestId,
        threadId: threadId,
        decision: decision,
      );
      logFlutterDebug(
        'thread_commands',
        'sendApprovalDecision accepted threadId=$threadId requestId=$requestId',
      );
    } catch (error, stackTrace) {
      logFlutterError(
        'thread_commands',
        'sendApprovalDecision failed threadId=$threadId requestId=$requestId',
        error: error,
        stackTrace: stackTrace,
      );
      rethrow;
    }
  }

  Future<void> deleteThread({required String threadId}) async {
    logFlutterInfo(
      'thread_commands',
      'deleteThread requested threadId=$threadId',
    );
    try {
      await _repository.deleteThread(threadId: threadId);
      logFlutterDebug(
        'thread_commands',
        'deleteThread succeeded threadId=$threadId',
      );
    } catch (error, stackTrace) {
      logFlutterError(
        'thread_commands',
        'deleteThread failed threadId=$threadId',
        error: error,
        stackTrace: stackTrace,
      );
      rethrow;
    }
  }
}
