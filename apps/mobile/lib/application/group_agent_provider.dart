import 'dart:convert';
import 'dart:io';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/domain/group_member.dart';
import 'package:minos/infrastructure/app_paths.dart';

/// Manages the mapping of agents to group conversations.
/// Stored locally as a JSON file alongside agent profiles.
///
/// Structure: { conversationId: [agentId, ...] }
final groupAgentBindingsProvider =
    AsyncNotifierProvider<
      GroupAgentBindingsController,
      Map<String, List<String>>
    >(GroupAgentBindingsController.new);

class GroupAgentBindingsController
    extends AsyncNotifier<Map<String, List<String>>> {
  @override
  Future<Map<String, List<String>>> build() async {
    return _load();
  }

  Future<Map<String, List<String>>> _load() async {
    try {
      final file = File(await _bindingsFilePath());
      if (!await file.exists()) {
        return const <String, List<String>>{};
      }
      final raw = await file.readAsString();
      if (raw.trim().isEmpty) {
        return const <String, List<String>>{};
      }
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, Object?>) {
        return const <String, List<String>>{};
      }
      return decoded.map((key, value) {
        final list =
            (value as List<Object?>?)?.whereType<String>().toList(
              growable: false,
            ) ??
            const <String>[];
        return MapEntry(key, list);
      });
    } catch (_) {
      return const <String, List<String>>{};
    }
  }

  Future<void> _persist(Map<String, List<String>> data) async {
    state = AsyncValue.data(data);
    final file = File(await _bindingsFilePath());
    await file.parent.create(recursive: true);
    final payload = const JsonEncoder.withIndent('  ').convert(data);
    await file.writeAsString(payload, flush: true);
  }

  /// Add an agent to a group conversation.
  Future<void> addAgentToGroup({
    required String conversationId,
    required String agentId,
  }) async {
    final current = await future;
    final existing = current[conversationId] ?? <String>[];
    if (existing.contains(agentId)) return;
    final next = Map<String, List<String>>.from(current);
    next[conversationId] = [...existing, agentId];
    await _persist(next);
  }

  /// Remove an agent from a group conversation.
  Future<void> removeAgentFromGroup({
    required String conversationId,
    required String agentId,
  }) async {
    final current = await future;
    final existing = current[conversationId];
    if (existing == null || !existing.contains(agentId)) return;
    final next = Map<String, List<String>>.from(current);
    next[conversationId] = existing.where((id) => id != agentId).toList();
    if (next[conversationId]!.isEmpty) {
      next.remove(conversationId);
    }
    await _persist(next);
  }

  /// Get agent IDs for a specific conversation.
  List<String> agentIdsForConversation(String conversationId) {
    return state.asData?.value[conversationId] ?? const <String>[];
  }
}

/// Provides the list of agents added to a specific group conversation.
final groupAgentsProvider = Provider.family<List<AgentProfile>, String>((
  ref,
  conversationId,
) {
  final bindings = ref.watch(groupAgentBindingsProvider).asData?.value;
  if (bindings == null) return const <AgentProfile>[];
  final agentIds = bindings[conversationId] ?? const <String>[];
  if (agentIds.isEmpty) return const <AgentProfile>[];

  final workspaceState = ref
      .watch(agentProfilesControllerProvider)
      .asData
      ?.value;
  if (workspaceState == null) return const <AgentProfile>[];

  return agentIds
      .map((agentId) {
        for (final profile in workspaceState.profiles) {
          if (profile.agentId == agentId) return profile;
        }
        return null;
      })
      .whereType<AgentProfile>()
      .toList(growable: false);
});

/// Provides all mentionable members (users + agents) for a group conversation.
final groupMentionableMembersProvider =
    Provider.family<List<GroupMember>, String>((ref, conversationId) {
      final agents = ref.watch(groupAgentsProvider(conversationId));
      final agentMembers = agents
          .map((profile) => GroupMember.fromAgent(profile))
          .toList();
      return agentMembers;
    });

Future<String> _bindingsFilePath() async {
  final root = await minosAppDirectory();
  return '${root.path}/group_agent_bindings.json';
}
