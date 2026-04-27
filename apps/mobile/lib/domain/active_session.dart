import 'package:flutter/foundation.dart' show immutable;

import 'package:minos/src/rust/api/minos.dart' show AgentName, MinosError;

/// Dart-owned mobile-side state machine for the agent dispatch lifecycle.
///
/// State transitions (driven by [ActiveSessionController]):
/// ```
///   Idle ──start()──> Starting ──Rust ack──> Streaming
///                          \                      │
///                           \──error──> Error     │
///                                                 ▼
///                          Streaming ──MessageCompleted──> AwaitingInput
///                                                 │
///                                       send()────┴──> Streaming
///                                                 │
///                                  stop()/ThreadClosed──> Stopped
/// ```
///
/// `threadId` is the daemon-issued `session_id` (per
/// `crates/minos-protocol/src/messages.rs:50`).
@immutable
sealed class ActiveSession {
  const ActiveSession();
}

/// No agent session is in flight on this device. The chat input gates on
/// this state to mean "Send" should call `start_agent` instead of
/// `send_user_message`.
class SessionIdle extends ActiveSession {
  const SessionIdle();

  @override
  bool operator ==(Object other) => other is SessionIdle;

  @override
  int get hashCode => (SessionIdle).hashCode;
}

/// We've called `start_agent` and are waiting for the daemon to mint a
/// `session_id`. The prompt is held here so we can re-show it in the
/// chat surface before the first `MessageStarted` echoes back.
class SessionStarting extends ActiveSession {
  final AgentName agent;
  final String prompt;
  const SessionStarting({required this.agent, required this.prompt});

  @override
  bool operator ==(Object other) =>
      other is SessionStarting &&
      other.agent == agent &&
      other.prompt == prompt;

  @override
  int get hashCode => Object.hash(agent, prompt);
}

/// Agent is actively producing tokens; UI shows the streaming cursor.
class SessionStreaming extends ActiveSession {
  final String threadId;
  final AgentName agent;
  const SessionStreaming({required this.threadId, required this.agent});

  @override
  bool operator ==(Object other) =>
      other is SessionStreaming &&
      other.threadId == threadId &&
      other.agent == agent;

  @override
  int get hashCode => Object.hash(threadId, agent);
}

/// Streaming finished cleanly via `MessageCompleted`; the input bar is
/// re-enabled so the user can send a follow-up.
class SessionAwaitingInput extends ActiveSession {
  final String threadId;
  final AgentName agent;
  const SessionAwaitingInput({required this.threadId, required this.agent});

  @override
  bool operator ==(Object other) =>
      other is SessionAwaitingInput &&
      other.threadId == threadId &&
      other.agent == agent;

  @override
  int get hashCode => Object.hash(threadId, agent);
}

/// The thread has been closed (user hit Stop, daemon completed, or host
/// crashed). The next "Send" tap will start a brand-new session.
class SessionStopped extends ActiveSession {
  final String threadId;
  const SessionStopped(this.threadId);

  @override
  bool operator ==(Object other) =>
      other is SessionStopped && other.threadId == threadId;

  @override
  int get hashCode => threadId.hashCode;
}

/// Terminal failure on the dispatch path. `threadId` is null when the
/// failure happened before the daemon assigned one (e.g. the
/// `start_agent` RPC itself failed).
class SessionError extends ActiveSession {
  final String? threadId;
  final MinosError error;
  const SessionError({this.threadId, required this.error});

  @override
  bool operator ==(Object other) =>
      other is SessionError &&
      other.threadId == threadId &&
      other.error == error;

  @override
  int get hashCode => Object.hash(threadId, error);
}
