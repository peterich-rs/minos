import 'dart:async';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/src/rust/api/minos.dart'
    show
        AgentName,
        MinosError,
        UiEventFrame,
        UiEventMessage_Error,
        UiEventMessage_MessageCompleted,
        UiEventMessage_ThreadClosed;
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
    final core = ref.watch(minosCoreProvider);
    _eventsSub = core.uiEvents.listen(_onUiEvent);
    ref.onDispose(() => _eventsSub?.cancel());
    return const SessionIdle();
  }

  void _onUiEvent(UiEventFrame frame) {
    final s = state;
    if (s is SessionSending) {
      state = _nextStateForFrame(
        frame,
        threadId: frame.threadId,
        agent: s.agent,
      );
      return;
    }
    if (s is! SessionStreaming || s.threadId != frame.threadId) return;

    state = _nextStateForFrame(frame, threadId: s.threadId, agent: s.agent);
  }

  ActiveSession _nextStateForFrame(
    UiEventFrame frame, {
    required String threadId,
    required AgentName agent,
  }) {
    switch (frame.ui) {
      case UiEventMessage_MessageCompleted():
        return SessionAwaitingInput(threadId: threadId, agent: agent);
      case UiEventMessage_ThreadClosed():
        return SessionSuspended(threadId: threadId, agent: agent);
      case UiEventMessage_Error(:final message):
        return SessionError(
          threadId: threadId,
          error: MinosError.agentStartFailed(reason: message),
        );
      default:
        return SessionStreaming(threadId: threadId, agent: agent);
    }
  }

  /// Dispatch a user message.
  ///
  /// When the current state already carries a `thread_id`, a successful send
  /// immediately re-enters [SessionStreaming]. Brand-new conversations stay in
  /// [SessionSending] until the first matching UI frame binds the thread id.
  Future<MinosError?> send({
    required AgentName agent,
    required String text,
    required Future<void> Function() dispatch,
  }) async {
    final previous = state;
    state = SessionSending(agent: agent, text: text);
    try {
      await dispatch();
      final threadId = switch (previous) {
        SessionStreaming(threadId: final t) => t,
        SessionAwaitingInput(threadId: final t) => t,
        SessionSuspended(threadId: final t) => t,
        SessionError(threadId: final t?) => t,
        _ => null,
      };
      if (threadId != null) {
        state = SessionStreaming(threadId: threadId, agent: agent);
      }
      return null;
    } on MinosError catch (e) {
      state = _restoreAfterSendFailure(previous, e);
      return e;
    }
  }

  ActiveSession _restoreAfterSendFailure(
    ActiveSession previous,
    MinosError error,
  ) {
    return switch (previous) {
      SessionStreaming(threadId: final t, agent: final a) => SessionStreaming(
        threadId: t,
        agent: a,
      ),
      SessionAwaitingInput(threadId: final t, agent: final a) =>
        SessionAwaitingInput(threadId: t, agent: a),
      SessionSuspended(threadId: final t, agent: final a) => SessionSuspended(
        threadId: t,
        agent: a,
      ),
      SessionError(threadId: final t?, :final error) => SessionError(
        threadId: t,
        error: error,
      ),
      _ => SessionError(error: error),
    };
  }

  /// Best-effort interrupt. Failures preserve the current `thread_id` in a
  /// [SessionError] so the UI can still recover.
  Future<void> stop() async {
    final s = state;
    final (String? threadId, AgentName? agent) = switch (s) {
      SessionStreaming(threadId: final t, agent: final a) => (t, a),
      SessionAwaitingInput(threadId: final t, agent: final a) => (t, a),
      _ => (null, null),
    };
    if (threadId == null || agent == null) return;

    try {
      await ref.read(minosCoreProvider).interruptThread(threadId: threadId);
      state = SessionSuspended(threadId: threadId, agent: agent);
    } on MinosError catch (error) {
      state = SessionError(threadId: threadId, error: error);
      return;
    }
  }

  /// Clear any thread-bound session state before routing the user into a
  /// fresh chat composer.
  void reset() {
    state = const SessionIdle();
  }
}
