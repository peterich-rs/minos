import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/active_session_provider.dart';
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
  final conversation = await repository.createGroupConversation(
    title: profile.name,
    memberAccountIds: const <String>[],
  );
  await repository.addAgentToConversation(
    conversationId: conversation.conversationId,
    agentId: profile.agentId,
  );

  ref.read(activeSessionControllerProvider.notifier).reset();
  ref.invalidate(conversationsProvider);
  ref.invalidate(conversationMembersProvider(conversation.conversationId));
  ref.invalidate(conversationAgentMembersProvider(conversation.conversationId));

  return conversation;
}
