import 'package:flutter/foundation.dart' show immutable;

import 'package:minos/src/rust/api/minos.dart' show AgentName, MinosError;

/// Dart-owned mobile-side state machine for the agent dispatch lifecycle.
///
/// State transitions (driven by [ActiveSessionController]):
/// ```
///   Idle ──send()──> Sending ──first UI frame──> Streaming
///                     │                                │
///                     └────────error──────────────> Error
///                                                      ▼
///                         Streaming ──MessageCompleted──> AwaitingInput
///                            │                                 │
///                            ├──────────send()────────────────┘
///                            │
///                            └────stop()/ThreadClosed────> Suspended
/// ```
///
/// `threadId` is the daemon-issued `session_id` (per
/// `crates/minos-protocol/src/messages.rs:50`).
@immutable
sealed class ActiveSession {
  const ActiveSession();
}

/// No agent session is in flight on this device.
class SessionIdle extends ActiveSession {
  const SessionIdle();

  @override
  bool operator ==(Object other) => other is SessionIdle;

  @override
  int get hashCode => (SessionIdle).hashCode;
}

/// A message send has been accepted by the transport layer, but the page does
/// not have a bound `thread_id` yet. We stay here until the first live UI
/// frame arrives and lets the controller bind the session.
class SessionSending extends ActiveSession {
  final AgentName agent;
  final String text;
  const SessionSending({required this.agent, required this.text});

  @override
  bool operator ==(Object other) =>
      other is SessionSending && other.agent == agent && other.text == text;

  @override
  int get hashCode => Object.hash(agent, text);
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

/// The thread has been interrupted locally or suspended by the runtime.
/// Sending another message on the same conversation should resume it rather
/// than creating a brand-new session.
class SessionSuspended extends ActiveSession {
  final String threadId;
  final AgentName agent;
  const SessionSuspended({required this.threadId, required this.agent});

  @override
  bool operator ==(Object other) =>
      other is SessionSuspended &&
      other.threadId == threadId &&
      other.agent == agent;

  @override
  int get hashCode => Object.hash(threadId, agent);
}

/// Terminal failure on the dispatch path. `threadId` is null when the
/// failure happened before the runtime surfaced a thread id.
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
