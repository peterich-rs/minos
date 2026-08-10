import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';

Future<ConversationResponse> createAgentConversation(
  WidgetRef ref, {
  required AgentProfile profile,
}) async {
  if (profile.hostDeviceId case final hostId?) {
    await ref.read(activeMacProvider.notifier).setActive(hostId);
  }

  final repository = ref.read(socialRepositoryProvider);
  final serverProfile = await ensureServerAgentProfile(
    ref,
    repository,
    profile,
  );
  final conversation = await repository.createGroupConversation(
    title: profile.name,
    memberAccountIds: const <String>[],
  );
  await repository.addAgentToConversation(
    conversationId: conversation.conversationId,
    agentId: serverProfile.agentId,
  );

  ref.read(activeSessionControllerProvider.notifier).reset();
  ref.invalidate(conversationsProvider);
  ref.invalidate(conversationParticipantsProvider(conversation.conversationId));

  return conversation;
}

Future<AgentProfile> ensureServerAgentProfile(
  WidgetRef ref,
  SocialRepository repository,
  AgentProfile profile,
) async {
  final remoteAgents = await repository.listAgents();
  for (final agent in remoteAgents.agents) {
    if (agent.agentId == profile.agentId) {
      if (!_remoteAgentMatchesProfile(agent, profile)) {
        await repository.updateAgent(
          agentId: profile.agentId,
          name: profile.name,
          description: profile.description,
          runtimeAgent: profile.runtimeAgent.name,
          model: profile.model,
          workspacePath: profile.workspacePath,
          displayName: profile.name,
          defaultReasoningEffort: profile.reasoningEffort.name,
          // Round-trip Hub digital-body fields local draft does not own.
          systemPrompt: agent.systemPrompt,
          status: agent.status,
        );
      }
      return profile;
    }
  }

  // Hub agents is bot-identity SSOT; local profile only caches agent_id after mint.
  final registered = await repository.registerAgent(
    name: profile.name,
    description: profile.description,
    runtimeAgent: profile.runtimeAgent.name,
    model: profile.model,
    workspacePath: profile.workspacePath,
    displayName: profile.name,
    defaultReasoningEffort: profile.reasoningEffort.name,
    systemPrompt: '',
  );
  if (registered.agentId == profile.agentId) {
    return profile;
  }
  return ref
      .read(agentProfilesControllerProvider.notifier)
      .syncServerAgentId(profileId: profile.id, agentId: registered.agentId);
}

bool _remoteAgentMatchesProfile(AgentSummary agent, AgentProfile profile) {
  final remoteDisplay = agent.displayName.trim().isEmpty
      ? agent.name
      : agent.displayName.trim();
  return agent.name == profile.name &&
      remoteDisplay == profile.name &&
      agent.description == profile.description &&
      agent.runtimeAgent == profile.runtimeAgent.name &&
      agent.model == profile.model &&
      agent.defaultReasoningEffort.trim().toLowerCase() ==
          profile.reasoningEffort.name &&
      _trimmedOrEmpty(agent.workspacePath) ==
          _trimmedOrEmpty(profile.workspacePath);
}

String _trimmedOrEmpty(String? value) => value?.trim() ?? '';
