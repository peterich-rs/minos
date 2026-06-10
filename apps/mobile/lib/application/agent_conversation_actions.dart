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
  ref.invalidate(conversationMembersProvider(conversation.conversationId));
  ref.invalidate(conversationAgentMembersProvider(conversation.conversationId));

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
      return profile;
    }
  }

  final registered = await repository.registerAgent(
    name: profile.name,
    description: profile.description,
    runtimeAgent: profile.runtimeAgent.name,
    model: profile.model,
  );
  if (registered.agentId == profile.agentId) {
    return profile;
  }
  return ref
      .read(agentProfilesControllerProvider.notifier)
      .syncServerAgentId(profileId: profile.id, agentId: registered.agentId);
}
