import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/data/repositories/group_agent_repository.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/domain/group_member.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Unified conversation participants (humans ∪ bots). Membership-first SSOT
/// for @ picker and roster reads (ADR 0021).
final conversationParticipantsProvider = FutureProvider.family
    .autoDispose<ConversationParticipantsResponse, String>((
      ref,
      conversationId,
    ) async {
      return ref
          .read(groupAgentRepositoryProvider)
          .listConversationParticipants(conversationId);
    });

/// Bot members derived from [conversationParticipantsProvider].
final conversationAgentMembersProvider =
    FutureProvider.family<List<AgentSummary>, String>((
      ref,
      conversationId,
    ) async {
      final participants = await ref.watch(
        conversationParticipantsProvider(conversationId).future,
      );
      return participants.agents;
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

/// @-mentionable participants currently in the conversation (membership-first).
/// Humans exclude the viewer; agents are all bot members.
final groupMentionableMembersProvider =
    Provider.family<List<GroupMember>, String>((ref, conversationId) {
      final participants = ref
          .watch(conversationParticipantsProvider(conversationId))
          .asData
          ?.value;
      if (participants == null) {
        return const <GroupMember>[];
      }

      final humans = participants.humans
          .map(GroupMember.fromUser)
          .toList(growable: false);
      // Prefer Hub AgentSummary.display_name for bot cards / @ picker labels.
      final agents = participants.agents
          .map(GroupMember.fromAgentSummary)
          .toList(growable: false);
      return <GroupMember>[...agents, ...humans];
    });

AgentProfile _resolveProfile(
  AgentSummary summary,
  AgentWorkspaceState? workspaceState,
) {
  if (workspaceState != null) {
    for (final profile in workspaceState.profiles) {
      if (profile.agentId == summary.agentId) {
        // Prefer Hub display_name when present (global bot face).
        final display = summary.displayName.trim().isNotEmpty
            ? summary.displayName.trim()
            : summary.name;
        if (display.isNotEmpty && display != profile.name) {
          return profile.copyWith(name: display);
        }
        return profile;
      }
    }
  }

  final displayName = summary.displayName.trim().isNotEmpty
      ? summary.displayName.trim()
      : summary.name;
  return AgentProfile(
    id: 'server-${summary.agentId}',
    agentId: summary.agentId,
    name: displayName,
    description: summary.description,
    runtimeAgent: _runtimeAgentFromString(summary.runtimeAgent),
    model: summary.model,
    workspacePath: summary.workspacePath,
    reasoningEffort: _reasoningEffortFromString(summary.defaultReasoningEffort),
    environmentVariables: const <AgentEnvironmentVariable>[],
    createdAtMs: summary.createdAtMs.toInt(),
    updatedAtMs: summary.updatedAtMs.toInt(),
  );
}

AgentReasoningEffort _reasoningEffortFromString(String value) {
  return switch (value.trim().toLowerCase()) {
    'low' => AgentReasoningEffort.low,
    'high' => AgentReasoningEffort.high,
    _ => AgentReasoningEffort.medium,
  };
}

AgentName _runtimeAgentFromString(String value) {
  return switch (value) {
    'codex' => AgentName.codex,
    'claude' => AgentName.claude,
    'gemini' => AgentName.gemini,
    'opencode' => AgentName.opencode,
    'grok' => AgentName.grok,
    _ => AgentName.codex,
  };
}
