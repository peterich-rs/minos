import 'package:minos/domain/linked_host.dart';

/// Pure JSON → [LinkedHost] mapping for `GET /v1/hosts`.
///
/// Backend wire shape (ResponseEnvelope):
/// ```json
/// {
///   "data": {
///     "hosts": [
///       {
///         "host_device_id": "...",
///         "host_display_name": "My Mac",
///         "linked_at_ms": 123,
///         "online": true
///       }
///     ]
///   },
///   "request_id": "..."
/// }
/// ```
///
/// Also accepts a bare `{ "hosts": [...] }` body (no envelope) for tests.
List<LinkedHost> mapHostsListResponse(Object? json) {
  if (json is! Map) {
    throw FormatException(
      'GET /v1/hosts expected object, got ${json.runtimeType}',
    );
  }
  final root = Map<String, dynamic>.from(json);
  final data = root['data'];
  final hostsNode = data is Map
      ? Map<String, dynamic>.from(data)['hosts']
      : root['hosts'];
  if (hostsNode is! List) {
    throw const FormatException('GET /v1/hosts missing hosts array');
  }
  return hostsNode.map(mapHostSummaryJson).toList(growable: false);
}

LinkedHost mapHostSummaryJson(Object? json) {
  if (json is! Map) {
    throw FormatException('host row expected object, got ${json.runtimeType}');
  }
  final map = Map<String, dynamic>.from(json);
  final id = map['host_device_id']?.toString().trim() ?? '';
  if (id.isEmpty) {
    throw const FormatException('host row missing host_device_id');
  }
  final name = map['host_display_name']?.toString() ?? '';
  final linkedRaw = map['linked_at_ms'] ?? map['paired_at_ms'];
  final linkedAtMs = _asInt(linkedRaw);
  final online = map['online'] == true;
  final lastSeenRaw = map['last_seen_at_ms'];
  final lastSeenAtMs = _asInt(lastSeenRaw);
  return LinkedHost(
    hostInstallationId: id,
    hostDisplayName: name,
    linkedAtMs: linkedAtMs,
    online: online,
    lastSeenAtMs: lastSeenAtMs > 0 ? lastSeenAtMs : linkedAtMs,
  );
}

int _asInt(Object? value) {
  if (value is int) return value;
  if (value is num) return value.toInt();
  if (value is String) return int.tryParse(value) ?? 0;
  return 0;
}
