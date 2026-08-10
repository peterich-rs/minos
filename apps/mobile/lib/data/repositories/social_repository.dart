import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show PlatformInt64;

import 'package:minos/data/services/services.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/infrastructure/im_outbox_store.dart';
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

  /// Unified participants (humans ∪ bot agents). Preferred for @ picker and
  /// membership-first roster reads (ADR 0021).
  Future<ConversationParticipantsResponse> listConversationParticipants({
    required String conversationId,
  }) {
    return _core.listConversationParticipants(conversationId: conversationId);
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
    List<String> mentionedAccountIds = const <String>[],
    List<String> mentionedAgentIds = const <String>[],
  }) {
    return _cacheStore.insertPendingMessage(
      conversationId: conversationId,
      sender: sender,
      text: text,
      replyTo: replyTo,
      mentionedAccountIds: mentionedAccountIds,
      mentionedAgentIds: mentionedAgentIds,
    );
  }

  Future<void> touchConversationPreview({
    required String conversationId,
    required String preview,
    required int createdAtMs,
    int? unreadCount,
    int? unreadMentionCount,
  }) {
    return _cacheStore.touchConversationPreview(
      conversationId: conversationId,
      preview: preview,
      createdAtMs: createdAtMs,
      unreadCount: unreadCount,
      unreadMentionCount: unreadMentionCount,
    );
  }

  Future<void> upsertConversation(ConversationSummary conversation) {
    return _cacheStore.upsertConversation(conversation);
  }

  Future<ConversationSummary?> bumpUnread(
    String conversationId, {
    int unreadDelta = 1,
    bool mention = false,
  }) {
    return _cacheStore.bumpUnread(
      conversationId,
      unreadDelta: unreadDelta,
      mention: mention,
    );
  }

  Future<void> clearUnread(String conversationId) {
    return _cacheStore.clearUnread(conversationId);
  }

  Future<ConversationSummary?> loadConversation(String conversationId) {
    return _cacheStore.loadConversation(conversationId);
  }

  Future<ChatMessageSummary> sendChatMessage({
    required String conversationId,
    required String text,
    String? replyToMessageId,
    String? clientMessageId,
  }) {
    return _core.sendChatMessage(
      conversationId: conversationId,
      text: text,
      replyToMessageId: replyToMessageId,
      clientMessageId: clientMessageId,
    );
  }

  Future<void> enqueueUserMessageOutbox({
    required String clientMessageId,
    required String conversationId,
    required String text,
    String? replyToMessageId,
  }) {
    return _cacheStore.enqueueUserMessageOutbox(
      clientMessageId: clientMessageId,
      conversationId: conversationId,
      text: text,
      replyToMessageId: replyToMessageId,
    );
  }

  Future<void> enqueueReactionToggleOutbox({
    required String clientOpId,
    required String conversationId,
    required String messageId,
    required String emoji,
  }) {
    return _cacheStore.enqueueReactionToggleOutbox(
      clientOpId: clientOpId,
      conversationId: conversationId,
      messageId: messageId,
      emoji: emoji,
    );
  }

  Future<ToggleReactionResponse> toggleReaction({
    required String conversationId,
    required String messageId,
    required String emoji,
    required String clientOpId,
  }) {
    return _core.toggleReaction(
      conversationId: conversationId,
      messageId: messageId,
      emoji: emoji,
      clientOpId: clientOpId,
    );
  }

  Future<void> updateMessageReactions({
    required String conversationId,
    required String messageId,
    required List<ReactionGroup> reactions,
  }) {
    return _cacheStore.updateMessageReactions(
      conversationId: conversationId,
      messageId: messageId,
      reactions: reactions,
    );
  }

  Future<void> reclaimStaleOutbox() {
    return _cacheStore.reclaimStaleOutbox();
  }

  Future<List<ImOutboxEntry>> listDueOutbox({int? nowMs}) {
    return _cacheStore.listDueOutbox(nowMs: nowMs);
  }

  /// Per-conversation FIFO due lanes (see SocialCacheStore.listDueOutboxLanes).
  Future<List<List<ImOutboxEntry>>> listDueOutboxLanes({int? nowMs}) {
    return _cacheStore.listDueOutboxLanes(nowMs: nowMs);
  }

  Future<void> markOutboxInflight(String clientOpId) {
    return _cacheStore.markOutboxInflight(clientOpId);
  }

  Future<void> markOutboxAcked(String clientOpId) {
    return _cacheStore.markOutboxAcked(clientOpId);
  }

  Future<void> markOutboxFailed({
    required String clientOpId,
    required String error,
  }) {
    return _cacheStore.markOutboxFailed(clientOpId: clientOpId, error: error);
  }

  Future<SocialChatMessage?> loadMessageByClientMessageId(
    String clientMessageId,
  ) {
    return _cacheStore.loadMessageByClientMessageId(clientMessageId);
  }

  Future<void> reconcileSendingMessagesOnStartup() {
    return _cacheStore.reconcileSendingMessagesOnStartup();
  }

  Future<List<AgentSessionSummaryDto>> listAgentSessions({
    required String conversationId,
    int limit = 5,
  }) {
    return _core.listAgentSessions(
      conversationId: conversationId,
      limit: limit,
    );
  }

  Future<void> subscribeAgentSession({required String sessionId}) {
    return _core.subscribeAgentSession(sessionId: sessionId);
  }

  /// R3a: open-chat `conversation:{id}` full T1 frames.
  Future<void> subscribeConversation({required String conversationId}) {
    return _core.subscribeConversation(conversationId: conversationId);
  }

  /// R3a: leave open-chat conversation topic.
  Future<void> unsubscribeConversation({required String conversationId}) {
    return _core.unsubscribeConversation(conversationId: conversationId);
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
    int? beforeSeq,
    int? afterSeq,
    int limit = 100,
  }) {
    return _core.listChatMessages(
      conversationId: conversationId,
      beforeSeq: beforeSeq,
      afterSeq: afterSeq,
      limit: limit,
    );
  }

  Future<void> markConversationRead({
    required String conversationId,
    required int readUpToMessageSeq,
  }) {
    return _core.markConversationRead(
      conversationId: conversationId,
      readUpToMessageSeq: readUpToMessageSeq,
    );
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
    String? workspacePath,
  }) {
    return _core.registerAgent(
      name: name,
      description: description,
      runtimeAgent: runtimeAgent,
      model: model,
      workspacePath: workspacePath,
    );
  }

  Future<AgentSummary> updateAgent({
    required String agentId,
    required String name,
    required String description,
    required String runtimeAgent,
    required String model,
    String? workspacePath,
  }) {
    return _core.updateAgent(
      agentId: agentId,
      name: name,
      description: description,
      runtimeAgent: runtimeAgent,
      model: model,
      workspacePath: workspacePath,
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
