import 'dart:convert';
import 'dart:io';

import 'package:minos/domain/agent_profile.dart';
import 'package:minos/infrastructure/app_paths.dart';

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
        return AgentWorkspaceState.bootstrap();
      }
      final raw = await file.readAsString();
      if (raw.trim().isEmpty) {
        return AgentWorkspaceState.bootstrap();
      }
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, Object?>) {
        return AgentWorkspaceState.bootstrap();
      }
      return AgentWorkspaceState.fromJson(decoded).normalized();
    } catch (_) {
      return AgentWorkspaceState.bootstrap();
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
