import 'package:flutter/foundation.dart' show immutable;

/// One account-linked host from `GET /v1/hosts` (Host Link, D02).
///
/// Distinct from the FRB [HostSummaryDto] so pure-Dart repositories and
/// unit tests do not depend on generated FFI types.
@immutable
class LinkedHost {
  const LinkedHost({
    required this.hostInstallationId,
    required this.hostDisplayName,
    required this.linkedAtMs,
    required this.online,
    this.lastSeenAtMs = 0,
  });

  final String hostInstallationId;
  final String hostDisplayName;
  final int linkedAtMs;

  /// Hub device online: live `/ws/host` for this installation on the server.
  final bool online;

  /// Durable last activity from hub (`last_seen_at_ms`). 0 when unknown.
  final int lastSeenAtMs;

  LinkedHost copyWith({
    String? hostInstallationId,
    String? hostDisplayName,
    int? linkedAtMs,
    bool? online,
    int? lastSeenAtMs,
  }) {
    return LinkedHost(
      hostInstallationId: hostInstallationId ?? this.hostInstallationId,
      hostDisplayName: hostDisplayName ?? this.hostDisplayName,
      linkedAtMs: linkedAtMs ?? this.linkedAtMs,
      online: online ?? this.online,
      lastSeenAtMs: lastSeenAtMs ?? this.lastSeenAtMs,
    );
  }

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is LinkedHost &&
          hostInstallationId == other.hostInstallationId &&
          hostDisplayName == other.hostDisplayName &&
          linkedAtMs == other.linkedAtMs &&
          online == other.online &&
          lastSeenAtMs == other.lastSeenAtMs;

  @override
  int get hashCode => Object.hash(
    hostInstallationId,
    hostDisplayName,
    linkedAtMs,
    online,
    lastSeenAtMs,
  );

  @override
  String toString() =>
      'LinkedHost($hostInstallationId, $hostDisplayName, online=$online, lastSeen=$lastSeenAtMs)';
}
