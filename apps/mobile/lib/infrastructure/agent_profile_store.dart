import 'dart:convert';
import 'dart:io';

import 'package:minos/domain/agent_profile.dart';
import 'package:minos/infrastructure/app_paths.dart';

/// __SAFETY_ASSERT__: Agent profiles are strictly CLIENT-LOCAL.
/// They are never synced to or stored on the backend.
/// If you find yourself writing code that POSTs profile data to any backend
/// endpoint, you are violating the architecture contract (spec §5.5).
///
/// Profiles exist only in the local JSON file on the device.
/// The backend has no knowledge of profiles and must never receive them.

abstract class AgentProfileStore {
  Future<AgentWorkspaceState> load();

  Future<void> save(AgentWorkspaceState state);
}

class JsonFileAgentProfileStore implements AgentProfileStore {
  const JsonFileAgentProfileStore();

  @override
  Future<AgentWorkspaceState> load() async {
    try {
      final file = File(await agentProfilesFilePath());
      if (!await file.exists()) {
        return const AgentWorkspaceState.empty();
      }
      final raw = await file.readAsString();
      if (raw.trim().isEmpty) {
        return const AgentWorkspaceState.empty();
      }
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, Object?>) {
        return const AgentWorkspaceState.empty();
      }
      return AgentWorkspaceState.fromJson(decoded).normalized();
    } catch (_) {
      return const AgentWorkspaceState.empty();
    }
  }

  @override
  Future<void> save(AgentWorkspaceState state) async {
    final file = File(await agentProfilesFilePath());
    await file.parent.create(recursive: true);
    final payload = const JsonEncoder.withIndent('  ').convert(state.toJson());
    await file.writeAsString(payload, flush: true);
  }
}
