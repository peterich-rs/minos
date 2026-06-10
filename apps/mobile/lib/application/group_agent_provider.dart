import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/data/repositories/group_agent_repository.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/domain/group_member.dart';
import 'package:minos/src/rust/api/minos.dart';

final conversationAgentMembersProvider =
    FutureProvider.family<List<AgentSummary>, String>((
      ref,
      conversationId,
    ) async {
      return ref
          .read(groupAgentRepositoryProvider)
          .listConversationAgents(conversationId);
    });
final groupAgentsProvider = Provider.family<List<AgentProfile>, String>((
  ref,
  conversationId,
) {
  final agentSummaries =
      ref
          .watch(conversationAgentMembersProvider(conversationId))
          .asData
          ?.value ??
      const <AgentSummary>[];
  if (agentSummaries.isEmpty) {
    return const <AgentProfile>[];
  }

  final workspaceState = ref
      .watch(agentProfilesControllerProvider)
      .asData
      ?.value;
  return agentSummaries
      .map((summary) => _resolveProfile(summary, workspaceState))
      .toList(growable: false);
});

final groupMentionableMembersProvider =
    Provider.family<List<GroupMember>, String>((ref, conversationId) {
      final agents = ref.watch(groupAgentsProvider(conversationId));
      return agents.map(GroupMember.fromAgent).toList(growable: false);
    });

AgentProfile _resolveProfile(
  AgentSummary summary,
  AgentWorkspaceState? workspaceState,
) {
  if (workspaceState != null) {
    for (final profile in workspaceState.profiles) {
      if (profile.agentId == summary.agentId) {
        return profile;
      }
    }
  }

  return AgentProfile(
    id: 'server-${summary.agentId}',
    agentId: summary.agentId,
    name: summary.name,
    description: summary.description,
    runtimeAgent: _runtimeAgentFromString(summary.runtimeAgent),
    model: summary.model,
    workspacePath: summary.workspacePath,
    reasoningEffort: AgentReasoningEffort.medium,
    environmentVariables: const <AgentEnvironmentVariable>[],
    createdAtMs: summary.createdAtMs.toInt(),
    updatedAtMs: summary.updatedAtMs.toInt(),
  );
}

AgentName _runtimeAgentFromString(String value) {
  return switch (value) {
    'codex' => AgentName.codex,
    'claude' => AgentName.claude,
    'gemini' => AgentName.gemini,
    _ => AgentName.codex,
  };
}
