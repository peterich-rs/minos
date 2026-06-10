import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show PlatformInt64;

import 'package:minos/data/services/services.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';

final socialRepositoryProvider = Provider<SocialRepository>((ref) {
  return SocialRepository(
    core: ref.watch(minosCoreServiceProvider),
    cacheStore: ref.watch(socialCacheStoreProvider),
  );
});

class SocialRepository {
  const SocialRepository({
    required MinosCoreProtocol core,
    required SocialCacheStore cacheStore,
  }) : _core = core,
       _cacheStore = cacheStore;

  final MinosCoreProtocol _core;
  final SocialCacheStore _cacheStore;

  Stream<SocialEventFrame> get socialEvents => _core.socialEvents;

  Future<MyProfileResponse> myProfile() {
    return _core.myProfile();
  }

  Future<List<UserSummary>> searchUsers({required String minosId}) {
    return _core.searchUsers(minosId: minosId);
  }

  Future<void> setMinosId({required String minosId}) {
    return _core.setMinosId(minosId: minosId);
  }

  Future<List<UserSummary>> conversationMembers({
    required String conversationId,
  }) async {
    final response = await _core.conversationMembers(
      conversationId: conversationId,
    );
    return response.members;
  }

  Future<List<SocialChatMessage>> loadMessages(String conversationId) {
    return _cacheStore.loadMessages(conversationId);
  }

  Future<String?> loadCurrentAccountId() {
    return _cacheStore.loadCurrentAccountId();
  }

  Future<void> saveCurrentAccountId(String accountId) {
    return _cacheStore.saveCurrentAccountId(accountId);
  }

  int platformInt64ToIntValue(PlatformInt64 value) {
    return platformInt64ToInt(value);
  }

  PlatformInt64 platformInt64FromIntValue(int value) {
    return platformInt64FromInt(value);
  }

  Future<SocialChatMessage> insertPendingMessage({
    required String conversationId,
    required UserSummary sender,
    required String text,
    ChatMessageReplySummary? replyTo,
  }) {
    return _cacheStore.insertPendingMessage(
      conversationId: conversationId,
      sender: sender,
      text: text,
      replyTo: replyTo,
    );
  }

  Future<void> touchConversationPreview({
    required String conversationId,
    required String preview,
    required int createdAtMs,
  }) {
    return _cacheStore.touchConversationPreview(
      conversationId: conversationId,
      preview: preview,
      createdAtMs: createdAtMs,
    );
  }

  Future<ChatMessageSummary> sendChatMessage({
    required String conversationId,
    required String text,
    String? replyToMessageId,
  }) {
    return _core.sendChatMessage(
      conversationId: conversationId,
      text: text,
      replyToMessageId: replyToMessageId,
    );
  }

  Future<SocialChatMessage?> markMessageSent({
    required String localId,
    required ChatMessageSummary message,
  }) {
    return _cacheStore.markMessageSent(localId: localId, message: message);
  }

  Future<SocialChatMessage?> markMessageFailed(String localId) {
    return _cacheStore.markMessageFailed(localId);
  }

  Future<SocialChatMessage?> markMessageSending(String localId) {
    return _cacheStore.markMessageSending(localId);
  }

  Future<ChatMessageSummary> recallChatMessage({
    required String conversationId,
    required String messageId,
  }) {
    return _core.recallChatMessage(
      conversationId: conversationId,
      messageId: messageId,
    );
  }

  Future<void> upsertRemoteMessage(ChatMessageSummary message) {
    return _cacheStore.upsertRemoteMessage(message);
  }

  Future<void> upsertRemoteMessages({
    required String conversationId,
    required List<ChatMessageSummary> messages,
  }) {
    return _cacheStore.upsertRemoteMessages(
      conversationId: conversationId,
      messages: messages,
    );
  }

  Future<ListChatMessagesResponse> listChatMessages({
    required String conversationId,
    int limit = 100,
  }) {
    return _core.listChatMessages(conversationId: conversationId, limit: limit);
  }

  Future<void> markConversationRead({required String conversationId}) {
    return _core.markConversationRead(conversationId: conversationId);
  }

  Future<FriendRequestsResponse> friendRequests() {
    return _core.friendRequests();
  }

  Future<FriendsResponse> friends() {
    return _core.friends();
  }

  Future<AgentSummary> registerAgent({
    required String name,
    required String description,
    required String runtimeAgent,
    required String model,
  }) {
    return _core.registerAgent(
      name: name,
      description: description,
      runtimeAgent: runtimeAgent,
      model: model,
    );
  }

  Future<ListAgentsResponse> listAgents() {
    return _core.listAgents();
  }

  Future<ConversationsResponse> conversations() {
    return _core.conversations();
  }

  Future<void> deleteConversation({required String conversationId}) async {
    await _core.deleteConversation(conversationId: conversationId);
    await _cacheStore.deleteConversation(conversationId);
  }

  Future<ConversationsResponse?> loadConversations() {
    return _cacheStore.loadConversations();
  }

  Future<void> saveConversations(List<ConversationSummary> conversations) {
    return _cacheStore.saveConversations(conversations);
  }

  Future<void> createFriendRequest({required String targetMinosId}) {
    return _core.createFriendRequest(targetMinosId: targetMinosId);
  }

  Future<void> rejectFriendRequest({required String requestId}) {
    return _core.rejectFriendRequest(requestId: requestId);
  }

  Future<void> acceptFriendRequest({required String requestId}) {
    return _core.acceptFriendRequest(requestId: requestId);
  }

  Future<ConversationResponse> ensureDirectConversation({
    required String friendAccountId,
  }) {
    return _core.ensureDirectConversation(friendAccountId: friendAccountId);
  }

  Future<ConversationResponse> createGroupConversation({
    required String title,
    required List<String> memberAccountIds,
  }) {
    return _core.createGroupConversation(
      title: title,
      memberAccountIds: memberAccountIds,
    );
  }

  Future<void> addGroupMember({
    required String conversationId,
    required String memberAccountId,
  }) {
    return _core.addGroupMember(
      conversationId: conversationId,
      memberAccountId: memberAccountId,
    );
  }

  Future<void> removeGroupMember({
    required String conversationId,
    required String memberAccountId,
  }) {
    return _core.removeGroupMember(
      conversationId: conversationId,
      memberAccountId: memberAccountId,
    );
  }

  Future<void> addAgentToConversation({
    required String conversationId,
    required String agentId,
  }) {
    return _core.addAgentToConversation(
      conversationId: conversationId,
      agentId: agentId,
    );
  }

  Future<void> removeAgentFromConversation({
    required String conversationId,
    required String agentId,
  }) {
    return _core.removeAgentFromConversation(
      conversationId: conversationId,
      agentId: agentId,
    );
  }
}
