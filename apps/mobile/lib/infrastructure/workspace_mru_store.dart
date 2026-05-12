import 'dart:convert';
import 'dart:io';

import 'package:path_provider/path_provider.dart';

/// Per-host workspace MRU (Most Recently Used) store.
/// Persists to a JSON file with a cap of 8 entries per host.
/// Spec §5.4 — remember last-used workspaces per host.
class WorkspaceMruStore {
  static const int _cap = 8;
  static const String _fileName = 'workspace_mru.json';

  /// Load the MRU list for a given host. Returns up to [_cap] entries.
  Future<List<String>> load(String hostDeviceId) async {
    final data = await _readAll();
    final entries = data[hostDeviceId];
    if (entries is! List) return const [];
    return entries
        .whereType<String>()
        .where((s) => s.isNotEmpty)
        .take(_cap)
        .toList(growable: false);
  }

  /// Push a workspace to the front of the MRU for the given host.
  /// Deduplicates and caps at [_cap] entries.
  Future<List<String>> push(String hostDeviceId, String workspace) async {
    final trimmed = workspace.trim();
    if (trimmed.isEmpty) return load(hostDeviceId);

    final data = await _readAll();
    final current = await load(hostDeviceId);
    final next = <String>[
      trimmed,
      ...current.where((item) => item != trimmed),
    ].take(_cap).toList(growable: false);

    data[hostDeviceId] = next;
    await _writeAll(data);
    return next;
  }

  /// Clear the MRU for a given host.
  Future<void> clear(String hostDeviceId) async {
    final data = await _readAll();
    data.remove(hostDeviceId);
    await _writeAll(data);
  }

  Future<File> _file() async {
    final dir = await getApplicationDocumentsDirectory();
    return File('${dir.path}/$_fileName');
  }

  Future<Map<String, dynamic>> _readAll() async {
    try {
      final file = await _file();
      if (!await file.exists()) return {};
      final raw = await file.readAsString();
      if (raw.trim().isEmpty) return {};
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, dynamic>) return {};
      return decoded;
    } catch (_) {
      return {};
    }
  }

  Future<void> _writeAll(Map<String, dynamic> data) async {
    final file = await _file();
    await file.parent.create(recursive: true);
    await file.writeAsString(
      const JsonEncoder.withIndent('  ').convert(data),
      flush: true,
    );
  }
}
