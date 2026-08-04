// Mobile IM Outbox types + pure status-machine policy (shared with Desktop).
// SQL persistence lives in SocialCacheStore (`social_cache.db` table `im_outbox`).

const int kImOutboxMaxPermanentAttempts = 8;
const int kImOutboxBaseBackoffMs = 1500;
/// Cap exponential backoff so long offline keeps retrying without terminal.
const int kImOutboxMaxBackoffMs = 5 * 60 * 1000;
const int kImOutboxStaleInflightMs = 45 * 1000;

enum ImOutboxStatus {
  pending,
  inflight,
  acked,
  failedTerminal,
}

enum ImOutboxKind {
  userMessage,
  reactionToggle,
  /// Legacy / foreign kind (e.g. old agent_result). Never flushed as user send.
  unsupported,
}

/// Permanent client/business errors may exhaust to terminal.
/// Transient network/connection errors stay pending forever (capped backoff).
enum OutboxFailureClass {
  transient,
  permanent,
}

class ImOutboxEntry {
  const ImOutboxEntry({
    required this.clientOpId,
    required this.kind,
    required this.conversationId,
    required this.payloadJson,
    required this.status,
    required this.attempts,
    required this.nextAttemptAtMs,
    required this.createdAtMs,
    required this.updatedAtMs,
    this.lastError,
  });

  /// = client_message_id for user_message; = client_op_id for reaction_toggle.
  final String clientOpId;
  final ImOutboxKind kind;
  final String conversationId;
  final String payloadJson;
  final ImOutboxStatus status;
  final int attempts;
  final int nextAttemptAtMs;
  final String? lastError;
  final int createdAtMs;
  final int updatedAtMs;

  ImOutboxEntry copyWith({
    ImOutboxStatus? status,
    int? attempts,
    int? nextAttemptAtMs,
    int? updatedAtMs,
    Object? lastError = _unset,
    String? payloadJson,
  }) {
    return ImOutboxEntry(
      clientOpId: clientOpId,
      kind: kind,
      conversationId: conversationId,
      payloadJson: payloadJson ?? this.payloadJson,
      status: status ?? this.status,
      attempts: attempts ?? this.attempts,
      nextAttemptAtMs: nextAttemptAtMs ?? this.nextAttemptAtMs,
      createdAtMs: createdAtMs,
      updatedAtMs: updatedAtMs ?? this.updatedAtMs,
      lastError: identical(lastError, _unset)
          ? this.lastError
          : lastError as String?,
    );
  }
}

const Object _unset = Object();

class OutboxFailureOutcome {
  const OutboxFailureOutcome({
    required this.status,
    required this.nextAttemptAtMs,
    required this.failureClass,
  });

  final ImOutboxStatus status;
  final int nextAttemptAtMs;
  final OutboxFailureClass failureClass;
}

String imOutboxKindWire(ImOutboxKind kind) {
  switch (kind) {
    case ImOutboxKind.userMessage:
      return 'user_message';
    case ImOutboxKind.reactionToggle:
      return 'reaction_toggle';
    case ImOutboxKind.unsupported:
      return 'unsupported';
  }
}

ImOutboxKind imOutboxKindFromWire(String raw) {
  switch (raw) {
    case 'reaction_toggle':
      return ImOutboxKind.reactionToggle;
    case 'user_message':
      return ImOutboxKind.userMessage;
    default:
      // agent_result and any other unknown wire → permanent fail path, not
      // user_message flush (avoids empty_text / invalid_payload noise).
      return ImOutboxKind.unsupported;
  }
}

String imOutboxStatusWire(ImOutboxStatus status) {
  switch (status) {
    case ImOutboxStatus.pending:
      return 'pending';
    case ImOutboxStatus.inflight:
      return 'inflight';
    case ImOutboxStatus.acked:
      return 'acked';
    case ImOutboxStatus.failedTerminal:
      return 'failed_terminal';
  }
}

ImOutboxStatus imOutboxStatusFromWire(String raw) {
  switch (raw) {
    case 'inflight':
      return ImOutboxStatus.inflight;
    case 'acked':
      return ImOutboxStatus.acked;
    case 'failed_terminal':
      return ImOutboxStatus.failedTerminal;
    case 'pending':
    default:
      return ImOutboxStatus.pending;
  }
}

int imOutboxBackoffMs(int attempts) {
  final exp = attempts < 6 ? attempts : 6;
  final raw = kImOutboxBaseBackoffMs * (1 << exp);
  return raw > kImOutboxMaxBackoffMs ? kImOutboxMaxBackoffMs : raw;
}

/// Classify flush errors for terminal vs infinite-retry policy.
OutboxFailureClass classifyOutboxFailure(String error) {
  final e = error.toLowerCase();
  // Explicit permanent labels from our worker / validation.
  if (e.contains('invalid_payload') ||
      e.contains('empty_text') ||
      e.contains('invalid payload') ||
      e.contains('permission') ||
      e.contains('forbidden') ||
      e.contains('unauthorized') ||
      e.contains('not found') ||
      e.contains('http 4') ||
      e.contains('status: 4') ||
      e.contains('statuscode.4') ||
      RegExp(r'\b4\d\d\b').hasMatch(e)) {
    // 408 / 429 are transient despite 4xx shape.
    if (e.contains('408') ||
        e.contains('429') ||
        e.contains('timeout') ||
        e.contains('too many')) {
      return OutboxFailureClass.transient;
    }
    return OutboxFailureClass.permanent;
  }
  // Network / connection / auth-session gaps stay retryable.
  return OutboxFailureClass.transient;
}

/// Resolve next status after a failed flush attempt.
///
/// - **Transient**: always `pending` with capped backoff (never terminal).
/// - **Permanent**: `failed_terminal` after [kImOutboxMaxPermanentAttempts].
OutboxFailureOutcome resolveOutboxFailure({
  required int attempts,
  required String error,
  required int nowMs,
}) {
  final failureClass = classifyOutboxFailure(error);
  if (failureClass == OutboxFailureClass.transient) {
    return OutboxFailureOutcome(
      status: ImOutboxStatus.pending,
      nextAttemptAtMs: nowMs + imOutboxBackoffMs(attempts),
      failureClass: failureClass,
    );
  }
  if (attempts >= kImOutboxMaxPermanentAttempts) {
    return OutboxFailureOutcome(
      status: ImOutboxStatus.failedTerminal,
      nextAttemptAtMs: nowMs,
      failureClass: failureClass,
    );
  }
  return OutboxFailureOutcome(
    status: ImOutboxStatus.pending,
    nextAttemptAtMs: nowMs + imOutboxBackoffMs(attempts),
    failureClass: failureClass,
  );
}

bool isStaleInflight({
  required int updatedAtMs,
  required int nowMs,
  int staleMs = kImOutboxStaleInflightMs,
}) {
  return nowMs - updatedAtMs >= staleMs;
}

/// In-memory outbox for unit tests — same status machine as SQL store.
class ImOutboxMemory {
  final Map<String, ImOutboxEntry> _entries = <String, ImOutboxEntry>{};

  List<ImOutboxEntry> get snapshot =>
      _entries.values.toList(growable: false)..sort(
        (a, b) => a.createdAtMs.compareTo(b.createdAtMs),
      );

  void enqueueUserMessage({
    required String clientOpId,
    required String conversationId,
    required String payloadJson,
    required int nowMs,
  }) {
    final existing = _entries[clientOpId];
    if (existing != null) {
      if (existing.status == ImOutboxStatus.acked) {
        return;
      }
      _entries[clientOpId] = existing.copyWith(
        payloadJson: payloadJson,
        status: ImOutboxStatus.pending,
        nextAttemptAtMs: nowMs,
        updatedAtMs: nowMs,
        lastError: null,
      );
      return;
    }
    _entries[clientOpId] = ImOutboxEntry(
      clientOpId: clientOpId,
      kind: ImOutboxKind.userMessage,
      conversationId: conversationId,
      payloadJson: payloadJson,
      status: ImOutboxStatus.pending,
      attempts: 0,
      nextAttemptAtMs: nowMs,
      createdAtMs: nowMs,
      updatedAtMs: nowMs,
    );
  }

  int reclaimStaleInflight(int nowMs) {
    var n = 0;
    for (final e in _entries.values.toList()) {
      if (e.status != ImOutboxStatus.inflight) continue;
      if (!isStaleInflight(updatedAtMs: e.updatedAtMs, nowMs: nowMs)) {
        continue;
      }
      _entries[e.clientOpId] = e.copyWith(
        status: ImOutboxStatus.pending,
        nextAttemptAtMs: nowMs,
        updatedAtMs: nowMs,
        lastError: e.lastError ?? 'stale_inflight_reclaimed',
      );
      n += 1;
    }
    return n;
  }

  List<ImOutboxEntry> listDue(int nowMs) {
    reclaimStaleInflight(nowMs);
    return snapshot
        .where(
          (e) =>
              e.status == ImOutboxStatus.pending && e.nextAttemptAtMs <= nowMs,
        )
        .toList(growable: false);
  }

  void markInflight(String clientOpId, int nowMs) {
    final e = _entries[clientOpId];
    if (e == null || e.status == ImOutboxStatus.acked) return;
    _entries[clientOpId] = e.copyWith(
      status: ImOutboxStatus.inflight,
      attempts: e.attempts + 1,
      updatedAtMs: nowMs,
    );
  }

  void markAcked(String clientOpId, int nowMs) {
    final e = _entries[clientOpId];
    if (e == null) return;
    _entries[clientOpId] = e.copyWith(
      status: ImOutboxStatus.acked,
      updatedAtMs: nowMs,
      lastError: null,
    );
  }

  ImOutboxStatus markFailed(String clientOpId, String error, int nowMs) {
    final e = _entries[clientOpId];
    if (e == null) return ImOutboxStatus.pending;
    if (e.status == ImOutboxStatus.acked) return ImOutboxStatus.acked;
    final outcome = resolveOutboxFailure(
      attempts: e.attempts,
      error: error,
      nowMs: nowMs,
    );
    _entries[clientOpId] = e.copyWith(
      status: outcome.status,
      nextAttemptAtMs: outcome.nextAttemptAtMs,
      updatedAtMs: nowMs,
      lastError: error,
    );
    return outcome.status;
  }

  /// Startup: all inflight → pending; return client_op_ids that still cover
  /// messages (for reconcile tests).
  Set<String> reclaimAllInflightOnStartup(int nowMs) {
    final covered = <String>{};
    for (final e in _entries.values.toList()) {
      if (e.status == ImOutboxStatus.inflight) {
        _entries[e.clientOpId] = e.copyWith(
          status: ImOutboxStatus.pending,
          nextAttemptAtMs: nowMs,
          updatedAtMs: nowMs,
        );
      }
      final s = _entries[e.clientOpId]!.status;
      if (s == ImOutboxStatus.pending || s == ImOutboxStatus.inflight) {
        covered.add(e.clientOpId);
      }
    }
    return covered;
  }

  /// Sending rows whose localId is not covered by pending/inflight outbox
  /// should become failed (manual retry).
  List<String> strandedSendingLocalIds({
    required List<String> sendingLocalIds,
    required Set<String> coveredOutboxIds,
  }) {
    return sendingLocalIds
        .where((id) => !coveredOutboxIds.contains(id))
        .toList(growable: false);
  }
}
