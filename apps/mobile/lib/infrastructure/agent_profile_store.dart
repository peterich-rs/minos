import 'dart:convert';
import 'dart:io';

import 'package:minos/domain/agent_profile.dart';
import 'package:minos/infrastructure/app_paths.dart';

/// Local cache for agent launch preferences on this device.
///
/// **Product bot identity SSOT is Hub `agents`** (global bot user + digital body).
/// See `docs/superpowers/specs/global-bot-identity-design.md`.
/// This file may still hold device-local drafts/cache, but must not mint a
/// second multi-end bot identity. Prefer Hub register/update + membership.

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
