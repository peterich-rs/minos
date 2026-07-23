import 'dart:async';

import 'package:minos/application/flutter_log.dart';
import 'package:minos/data/repositories/thread_repository.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/src/rust/api/minos.dart'
    show
        AgentName,
        MinosError,
        UiEventFrame,
        UiEventMessage_Error,
        UiEventMessage_MessageCompleted,
        UiEventMessage_SessionClosed;
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'active_session_provider.g.dart';

/// Drives the [ActiveSession] state machine off `core.uiEvents` and
/// the explicit `send/stop` actions.
///
/// While in [SessionSending] we bind the in-flight conversation to the
/// first UI frame that arrives, because the send path no longer returns a
/// daemon-issued `session_id` synchronously.
@Riverpod(keepAlive: true)
class ActiveSessionController extends _$ActiveSessionController {
  StreamSubscription<UiEventFrame>? _eventsSub;

  @override
  ActiveSession build() {
    _eventsSub = ref
        .watch(threadRepositoryProvider)
        .uiEvents
        .listen(_onUiEvent);
    ref.onDispose(() => _eventsSub?.cancel());
    return const SessionIdle();
  }

  void _onUiEvent(UiEventFrame frame) {
    final s = state;
    if (s is SessionSending) {
      state = _nextStateForFrame(
        frame,
        sessionId: frame.sessionId,
        agent: s.agent,
      );
      return;
    }
    if (s is! SessionStreaming || s.sessionId != frame.sessionId) return;

    state = _nextStateForFrame(frame, sessionId: s.sessionId, agent: s.agent);
  }

  ActiveSession _nextStateForFrame(
    UiEventFrame frame, {
    required String sessionId,
    required AgentName agent,
  }) {
    switch (frame.ui) {
      case UiEventMessage_MessageCompleted():
        return SessionAwaitingInput(sessionId: sessionId, agent: agent);
      case UiEventMessage_SessionClosed():
        return SessionSuspended(sessionId: sessionId, agent: agent);
      case UiEventMessage_Error(:final message):
        return SessionError(
          sessionId: sessionId,
          error: MinosError.agentStartFailed(reason: message),
        );
      default:
        return SessionStreaming(sessionId: sessionId, agent: agent);
    }
  }

  /// Dispatch a user message.
  ///
  /// When the current state already carries a `session_id`, a successful send
  /// immediately re-enters [SessionStreaming]. Brand-new conversations stay in
  /// [SessionSending] until the first matching UI frame binds the session id.
  Future<MinosError?> send({
    required AgentName agent,
    required String text,
    required Future<void> Function() dispatch,
  }) async {
    final previous = state;
    logFlutterInfo(
      'active_session',
      'send started agent=$agent textLength=${text.length} previous=${previous.runtimeType}',
    );
    state = SessionSending(agent: agent, text: text);
    try {
      await dispatch();
      final sessionId = switch (previous) {
        SessionStreaming(sessionId: final t) => t,
        SessionAwaitingInput(sessionId: final t) => t,
        SessionSuspended(sessionId: final t) => t,
        SessionError(sessionId: final t?) => t,
        _ => null,
      };
      if (sessionId != null) {
        state = SessionStreaming(sessionId: sessionId, agent: agent);
      }
      logFlutterDebug(
        'active_session',
        'send dispatched agent=$agent sessionId=${sessionId ?? '<pending>'}',
      );
      return null;
    } on MinosError catch (e, stackTrace) {
      logFlutterError(
        'active_session',
        'send failed agent=$agent',
        error: e,
        stackTrace: stackTrace,
      );
      state = _restoreAfterSendFailure(previous, e);
      return e;
    }
  }

  ActiveSession _restoreAfterSendFailure(
    ActiveSession previous,
    MinosError error,
  ) {
    return switch (previous) {
      SessionStreaming(sessionId: final t, agent: final a) => SessionStreaming(
        sessionId: t,
        agent: a,
      ),
      SessionAwaitingInput(sessionId: final t, agent: final a) =>
        SessionAwaitingInput(sessionId: t, agent: a),
      SessionSuspended(sessionId: final t, agent: final a) => SessionSuspended(
        sessionId: t,
        agent: a,
      ),
      SessionError(sessionId: final t?, :final error) => SessionError(
        sessionId: t,
        error: error,
      ),
      _ => SessionError(error: error),
    };
  }

  /// Best-effort interrupt. Failures preserve the current `session_id` in a
  /// [SessionError] so the UI can still recover.
  Future<void> stop() async {
    final s = state;
    final (String? sessionId, AgentName? agent) = switch (s) {
      SessionStreaming(sessionId: final t, agent: final a) => (t, a),
      SessionAwaitingInput(sessionId: final t, agent: final a) => (t, a),
      _ => (null, null),
    };
    if (sessionId == null || agent == null) return;

    try {
      await ref
          .read(threadRepositoryProvider)
          .interruptThread(sessionId: sessionId);
      state = SessionSuspended(sessionId: sessionId, agent: agent);
      logFlutterInfo(
        'active_session',
        'stop succeeded sessionId=$sessionId agent=$agent',
      );
    } on MinosError catch (error, stackTrace) {
      logFlutterError(
        'active_session',
        'stop failed sessionId=$sessionId agent=$agent',
        error: error,
        stackTrace: stackTrace,
      );
      state = SessionError(sessionId: sessionId, error: error);
      return;
    }
  }

  /// Clear any thread-bound session state before routing the user into a
  /// fresh chat composer.
  void reset() {
    logFlutterDebug('active_session', 'session reset to idle');
    state = const SessionIdle();
  }
}
