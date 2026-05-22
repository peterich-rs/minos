import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/src/rust/api/minos.dart';

final socialActionsProvider = Provider<SocialActions>((ref) {
  return SocialActions(ref);
});

class SocialActions {
  SocialActions(this._ref);

  final Ref _ref;

  SocialRepository get _repository => _ref.read(socialRepositoryProvider);

  Future<ConversationResponse> ensureDirectConversation({
    required String friendAccountId,
  }) {
    return _repository.ensureDirectConversation(
      friendAccountId: friendAccountId,
    );
  }

  Future<void> setMinosId({required String minosId}) {
    return _repository.setMinosId(minosId: minosId);
  }

  Future<ConversationResponse> createGroupConversation({
    required String title,
    required List<String> memberAccountIds,
  }) {
    return _repository.createGroupConversation(
      title: title,
      memberAccountIds: memberAccountIds,
    );
  }

  Future<void> rejectFriendRequest({required String requestId}) {
    return _repository.rejectFriendRequest(requestId: requestId);
  }

  Future<void> acceptFriendRequest({required String requestId}) {
    return _repository.acceptFriendRequest(requestId: requestId);
  }

  Future<void> createFriendRequest({required String targetMinosId}) {
    return _repository.createFriendRequest(targetMinosId: targetMinosId);
  }

  Future<void> addGroupMember({
    required String conversationId,
    required String memberAccountId,
  }) {
    return _repository.addGroupMember(
      conversationId: conversationId,
      memberAccountId: memberAccountId,
    );
  }

  Future<void> addAgentToConversation({
    required String conversationId,
    required String agentId,
  }) {
    return _repository.addAgentToConversation(
      conversationId: conversationId,
      agentId: agentId,
    );
  }

  Future<void> removeAgentFromConversation({
    required String conversationId,
    required String agentId,
  }) {
    return _repository.removeAgentFromConversation(
      conversationId: conversationId,
      agentId: agentId,
    );
  }
}
