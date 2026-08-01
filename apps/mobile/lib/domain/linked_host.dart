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
  });

  final String hostInstallationId;
  final String hostDisplayName;
  final int linkedAtMs;
  final bool online;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is LinkedHost &&
          hostInstallationId == other.hostInstallationId &&
          hostDisplayName == other.hostDisplayName &&
          linkedAtMs == other.linkedAtMs &&
          online == other.online;

  @override
  int get hashCode =>
      Object.hash(hostInstallationId, hostDisplayName, linkedAtMs, online);

  @override
  String toString() =>
      'LinkedHost($hostInstallationId, $hostDisplayName, online=$online)';
}
