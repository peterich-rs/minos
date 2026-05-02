import 'dart:async';

import 'package:riverpod_annotation/riverpod_annotation.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/thread_list_provider.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/src/rust/api/minos.dart'
    show
        AgentName,
        MinosError,
        UiEventFrame,
        UiEventMessage_Error,
        UiEventMessage_MessageCompleted,
        UiEventMessage_ThreadClosed;

part 'active_session_provider.g.dart';

/// Drives the [ActiveSession] state machine off `core.uiEvents` and
/// the explicit `start/send/stop` actions.
///
/// We intentionally only react to events whose `threadId` matches our
/// current `SessionStreaming.threadId` — other threads' fan-out frames
/// (e.g. a paired Mac running an unrelated session) must not poison the
/// mobile-side machine.
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
    // Only the streaming-on-this-thread state reacts to incoming events;
    // Idle / Starting / AwaitingInput / Stopped / Error all wait for the
    // next explicit transition.
    if (s is! SessionStreaming || s.threadId != frame.threadId) return;

    switch (frame.ui) {
      case UiEventMessage_MessageCompleted():
        state = SessionAwaitingInput(threadId: s.threadId, agent: s.agent);
      case UiEventMessage_ThreadClosed():
        state = SessionStopped(s.threadId);
      case UiEventMessage_Error(:final message):
        state = SessionError(
          threadId: s.threadId,
          error: MinosError.agentStartFailed(reason: message),
        );
      default:
        break;
    }
  }

  /// Kick off a brand-new agent session. Transitions through
  /// [SessionStarting] before settling into [SessionStreaming] (success)
  /// or [SessionError] (Rust-side dispatch failure). The initial user
  /// message is sent separately after the session id is minted.
  Future<MinosError?> start({
    required AgentName agent,
    required String prompt,
  }) async {
    state = SessionStarting(agent: agent, prompt: prompt);
    try {
      final resp = await ref
          .read(minosCoreProvider)
          .startAgent(agent: agent, prompt: prompt);
      ref.invalidate(threadListProvider);
      state = SessionStreaming(threadId: resp.sessionId, agent: agent);
      return null;
    } on MinosError catch (e) {
      state = SessionError(error: e);
      return e;
    }
  }

  /// Start a fresh session and deliver its first user message before exposing
  /// the thread id to the chat view. That keeps the initial history read from
  /// racing ahead of the confirmed user-message event.
  Future<MinosError?> startAndSend({
    required AgentName agent,
    required String prompt,
  }) async {
    state = SessionStarting(agent: agent, prompt: prompt);
    String? startedThreadId;
    try {
      final resp = await ref
          .read(minosCoreProvider)
          .startAgent(agent: agent, prompt: prompt);
      startedThreadId = resp.sessionId;
      ref.invalidate(threadListProvider);
      await ref
          .read(minosCoreProvider)
          .sendUserMessage(sessionId: resp.sessionId, text: prompt);
      state = SessionStreaming(threadId: resp.sessionId, agent: agent);
      return null;
    } on MinosError catch (e) {
      state = SessionError(threadId: startedThreadId, error: e);
      return e;
    }
  }

  /// Send a follow-up user message into the active session. No-op when
  /// the machine isn't in [SessionStreaming] or [SessionAwaitingInput].
  Future<MinosError?> send(String text) async {
    final s = state;
    final (String threadId, AgentName agent) = switch (s) {
      SessionStreaming(threadId: final t, agent: final a) => (t, a),
      SessionAwaitingInput(threadId: final t, agent: final a) => (t, a),
      _ => ('', AgentName.codex),
    };
    if (threadId.isEmpty) return null;

    state = SessionStreaming(threadId: threadId, agent: agent);
    try {
      await ref
          .read(minosCoreProvider)
          .sendUserMessage(sessionId: threadId, text: text);
      return null;
    } on MinosError catch (e) {
      state = SessionAwaitingInput(threadId: threadId, agent: agent);
      return e;
    }
  }

  /// Send into a known thread id, regardless of the current global state.
  ///
  /// Existing thread pages use this path for follow-ups after Stop/Error or
  /// after app navigation. The daemon/runtime treats the `sessionId` as the
  /// resume target, so the mobile UI does not start a new agent for an
  /// already-minted session.
  Future<MinosError?> sendToThread({
    required String threadId,
    required AgentName agent,
    required String text,
  }) async {
    state = SessionStreaming(threadId: threadId, agent: agent);
    try {
      await ref
          .read(minosCoreProvider)
          .sendUserMessage(sessionId: threadId, text: text);
      return null;
    } on MinosError catch (e) {
      state = SessionAwaitingInput(threadId: threadId, agent: agent);
      return e;
    }
  }

  /// Best-effort stop. Errors from the daemon are swallowed; the local
  /// machine still transitions to [SessionStopped] so the UI doesn't
  /// hang in a half-streaming state.
  Future<void> stop() async {
    final s = state;
    final String? threadId = switch (s) {
      SessionStreaming(threadId: final t) => t,
      SessionAwaitingInput(threadId: final t) => t,
      _ => null,
    };
    if (threadId == null) return;

    try {
      await ref.read(minosCoreProvider).closeThread(threadId: threadId);
    } on MinosError {
      // best-effort
    }
    state = SessionStopped(threadId);
  }

  /// Clear any thread-bound session state before routing the user into a
  /// fresh chat composer.
  void reset() {
    state = const SessionIdle();
  }
}
