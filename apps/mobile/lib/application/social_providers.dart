import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'social_providers.g.dart';

final socialProfileProvider = FutureProvider<MyProfileResponse>((ref) {
  return ref.watch(socialRepositoryProvider).myProfile();
});

@riverpod
class SocialSearchQuery extends _$SocialSearchQuery {
  @override
  String build() {
    return '';
  }

  void update(String value) {
    state = value.trim();
  }
}

final socialSearchProvider = FutureProvider.family
    .autoDispose<List<UserSummary>, String>((ref, query) async {
      final trimmed = query.trim();
      if (trimmed.isEmpty) return const <UserSummary>[];
      return ref.watch(socialRepositoryProvider).searchUsers(minosId: trimmed);
    });

final conversationMembersProvider = FutureProvider.family
    .autoDispose<List<UserSummary>, String>((ref, conversationId) async {
      return ref
          .watch(socialRepositoryProvider)
          .conversationMembers(conversationId: conversationId);
    });

@riverpod
class SocialReplyDraft extends _$SocialReplyDraft {
  @override
  String? build(String conversationId) {
    return null;
  }

  void select(String localId) {
    state = localId;
  }

  void clear() {
    state = null;
  }
}

final socialReplyMessageProvider = Provider.family<SocialChatMessage?, String>((
  ref,
  conversationId,
) {
  final localId = ref.watch(socialReplyDraftProvider(conversationId));
  if (localId == null) {
    return null;
  }
  final messages = ref
      .watch(socialConversationProvider(conversationId))
      .messages;
  for (final message in messages) {
    if (message.localId == localId && message.canReply) {
      return message;
    }
  }
  return null;
});

final socialUnreadCountProvider = Provider<int>((ref) {
  return ref
      .watch(conversationsProvider)
      .maybeWhen(
        data: (response) => response.conversations.fold<int>(
          0,
          (total, conversation) => total + conversation.unreadCount,
        ),
        orElse: () => 0,
      );
});

const Object _socialConversationUnset = Object();

class SocialConversationState {
  const SocialConversationState({
    required this.myAccountId,
    required this.messages,
    required this.isLoading,
    required this.error,
  });

  const SocialConversationState.initial()
    : myAccountId = null,
      messages = const <SocialChatMessage>[],
      isLoading = true,
      error = null;

  final String? myAccountId;
  final List<SocialChatMessage> messages;
  final bool isLoading;
  final Object? error;

  SocialConversationState copyWith({
    String? myAccountId,
    List<SocialChatMessage>? messages,
    bool? isLoading,
    Object? error = _socialConversationUnset,
  }) {
    return SocialConversationState(
      myAccountId: myAccountId ?? this.myAccountId,
      messages: messages ?? this.messages,
      isLoading: isLoading ?? this.isLoading,
      error: identical(error, _socialConversationUnset) ? this.error : error,
    );
  }
}

@riverpod
class SocialConversation extends _$SocialConversation {
  StreamSubscription<SocialEventFrame>? _eventsSub;

  late final String _conversationId;

  @override
  SocialConversationState build(String conversationId) {
    const initialState = SocialConversationState.initial();
    _conversationId = conversationId;
    _eventsSub?.cancel();
    _eventsSub = ref
        .read(socialRepositoryProvider)
        .socialEvents
        .listen(
          _onSocialEvent,
          onError: (Object error, StackTrace stackTrace) =>
              ref.invalidateSelf(),
          onDone: ref.invalidateSelf,
        );
    ref.onDispose(() => _eventsSub?.cancel());
    unawaited(_load(seedState: initialState));
    return initialState;
  }

  Future<void> refresh() => _load();

  Future<void> sendMessage(
    String text, {
    SocialChatMessage? replyToMessage,
  }) async {
    final trimmed = text.trim();
    if (trimmed.isEmpty) {
      return;
    }

    final repository = ref.read(socialRepositoryProvider);
    final replyPreview = _replyPreviewForMessage(replyToMessage);
    final pending = await repository.insertPendingMessage(
      conversationId: _conversationId,
      sender: await _localSender(),
      text: trimmed,
      replyTo: replyPreview,
    );
    await repository.touchConversationPreview(
      conversationId: _conversationId,
      preview: trimmed,
      createdAtMs: pending.createdAtMs,
    );
    state = state.copyWith(
      messages: await repository.loadMessages(_conversationId),
      error: null,
    );
    ref.invalidate(conversationsProvider);

    try {
      final message = await repository.sendChatMessage(
        conversationId: _conversationId,
        text: trimmed,
        replyToMessageId: replyPreview?.messageId,
      );
      await repository.markMessageSent(
        localId: pending.localId,
        message: message,
      );
      await repository.touchConversationPreview(
        conversationId: _conversationId,
        preview: message.text,
        createdAtMs: repository.platformInt64ToIntValue(message.createdAtMs),
      );
      state = state.copyWith(
        messages: await repository.loadMessages(_conversationId),
      );
      ref.invalidate(conversationsProvider);
    } catch (error) {
      await repository.markMessageFailed(pending.localId);
      state = state.copyWith(
        messages: await repository.loadMessages(_conversationId),
      );
      rethrow;
    }
  }

  Future<void> retryMessage(String localId) async {
    SocialChatMessage? target;
    for (final message in state.messages) {
      if (message.localId == localId) {
        target = message;
        break;
      }
    }
    if (target == null ||
        target.deliveryState != SocialMessageDeliveryState.failed) {
      return;
    }

    final repository = ref.read(socialRepositoryProvider);
    await repository.markMessageSending(localId);
    state = state.copyWith(
      messages: await repository.loadMessages(_conversationId),
    );

    try {
      final replyToMessageId = target.replyTo?.recalledAtMs == null
          ? target.replyTo?.messageId
          : null;
      final message = await repository.sendChatMessage(
        conversationId: _conversationId,
        text: target.text,
        replyToMessageId: replyToMessageId,
      );
      await repository.markMessageSent(localId: localId, message: message);
      await repository.touchConversationPreview(
        conversationId: _conversationId,
        preview: message.text,
        createdAtMs: repository.platformInt64ToIntValue(message.createdAtMs),
      );
      state = state.copyWith(
        messages: await repository.loadMessages(_conversationId),
      );
      ref.invalidate(conversationsProvider);
    } catch (error) {
      await repository.markMessageFailed(localId);
      state = state.copyWith(
        messages: await repository.loadMessages(_conversationId),
      );
      rethrow;
    }
  }

  Future<void> recallMessage(String localId) async {
    SocialChatMessage? target;
    for (final message in state.messages) {
      if (message.localId == localId) {
        target = message;
        break;
      }
    }
    if (target == null || !target.canRecall || target.serverMessageId == null) {
      return;
    }

    final repository = ref.read(socialRepositoryProvider);
    final message = await repository.recallChatMessage(
      conversationId: _conversationId,
      messageId: target.serverMessageId!,
    );
    await repository.upsertRemoteMessage(message);
    await repository.touchConversationPreview(
      conversationId: _conversationId,
      preview: message.text,
      createdAtMs: repository.platformInt64ToIntValue(message.createdAtMs),
    );
    state = state.copyWith(
      messages: await repository.loadMessages(_conversationId),
    );
    final replyDraft = ref.read(socialReplyDraftProvider(_conversationId));
    if (replyDraft == localId) {
      ref.read(socialReplyDraftProvider(_conversationId).notifier).clear();
    }
    ref.invalidate(conversationsProvider);
  }

  Future<void> _load({SocialConversationState? seedState}) async {
    final repository = ref.read(socialRepositoryProvider);
    final previous = seedState ?? state;
    final cachedMessages = await repository.loadMessages(_conversationId);
    final cachedAccountId = await repository.loadCurrentAccountId();
    state = previous.copyWith(
      myAccountId: cachedAccountId ?? previous.myAccountId,
      messages: cachedMessages.isEmpty ? previous.messages : cachedMessages,
      isLoading: true,
      error: null,
    );
    try {
      final profile = await repository.myProfile();
      await repository.saveCurrentAccountId(profile.accountId);
      final response = await repository.listChatMessages(
        conversationId: _conversationId,
        limit: 100,
      );
      await repository.upsertRemoteMessages(
        conversationId: _conversationId,
        messages: response.messages,
      );
      await repository.markConversationRead(conversationId: _conversationId);
      state = SocialConversationState(
        myAccountId: profile.accountId,
        messages: await repository.loadMessages(_conversationId),
        isLoading: false,
        error: null,
      );
      ref.invalidate(conversationsProvider);
    } catch (error) {
      state = SocialConversationState(
        myAccountId: cachedAccountId ?? previous.myAccountId,
        messages: cachedMessages.isEmpty ? previous.messages : cachedMessages,
        isLoading: false,
        error: error,
      );
    }
  }

  void _onSocialEvent(SocialEventFrame frame) {
    if (frame.conversationId != _conversationId) {
      return;
    }
    unawaited(_applyRemoteMessage(frame.message));
  }

  Future<void> _applyRemoteMessage(ChatMessageSummary message) async {
    final repository = ref.read(socialRepositoryProvider);
    await repository.upsertRemoteMessage(message);
    await repository.touchConversationPreview(
      conversationId: message.conversationId,
      preview: message.text,
      createdAtMs: repository.platformInt64ToIntValue(message.createdAtMs),
    );
    state = state.copyWith(
      messages: await repository.loadMessages(_conversationId),
      error: null,
    );
    unawaited(_markConversationRead());
    ref.invalidate(conversationsProvider);
  }

  Future<void> _markConversationRead() async {
    try {
      await ref
          .read(socialRepositoryProvider)
          .markConversationRead(conversationId: _conversationId);
      ref.invalidate(conversationsProvider);
    } catch (_) {}
  }

  Future<UserSummary> _localSender() async {
    final accountId =
        state.myAccountId ??
        await ref.read(socialRepositoryProvider).loadCurrentAccountId() ??
        'local-self';
    return UserSummary(accountId: accountId, minosId: 'me', displayName: '我');
  }

  ChatMessageReplySummary? _replyPreviewForMessage(SocialChatMessage? message) {
    if (message == null ||
        !message.canReply ||
        message.serverMessageId == null) {
      return null;
    }
    return ChatMessageReplySummary(
      messageId: message.serverMessageId!,
      sender: message.sender,
      text: message.text,
      recalledAtMs: message.recalledAtMs == null
          ? null
          : ref
                .read(socialRepositoryProvider)
                .platformInt64FromIntValue(message.recalledAtMs!),
    );
  }
}

final friendRequestsProvider =
    AsyncNotifierProvider<FriendRequestsController, FriendRequestsResponse>(
      FriendRequestsController.new,
    );

class FriendRequestsController extends AsyncNotifier<FriendRequestsResponse> {
  @override
  Future<FriendRequestsResponse> build() {
    return ref.watch(socialRepositoryProvider).friendRequests();
  }

  Future<void> refresh() async {
    state = AsyncValue.data(
      await ref.read(socialRepositoryProvider).friendRequests(),
    );
  }
}

final friendsProvider =
    AsyncNotifierProvider<FriendsController, FriendsResponse>(
      FriendsController.new,
    );

class FriendsController extends AsyncNotifier<FriendsResponse> {
  @override
  Future<FriendsResponse> build() {
    return ref.watch(socialRepositoryProvider).friends();
  }

  Future<void> refresh() async {
    state = AsyncValue.data(await ref.read(socialRepositoryProvider).friends());
  }
}

final conversationsProvider =
    AsyncNotifierProvider<ConversationsController, ConversationsResponse>(
      ConversationsController.new,
    );

class ConversationsController extends AsyncNotifier<ConversationsResponse> {
  StreamSubscription<SocialEventFrame>? _eventsSub;

  @override
  Future<ConversationsResponse> build() async {
    _eventsSub ??= ref
        .read(socialRepositoryProvider)
        .socialEvents
        .listen(
          (_) {
            ref.invalidateSelf();
          },
          onError: (Object error, StackTrace stackTrace) =>
              ref.invalidateSelf(),
          onDone: ref.invalidateSelf,
        );
    ref.onDispose(() => _eventsSub?.cancel());

    final repository = ref.read(socialRepositoryProvider);
    final cached = await repository.loadConversations();
    if (cached != null && cached.conversations.isNotEmpty) {
      unawaited(_refreshFromRemote());
      return cached;
    }

    try {
      return await _fetchRemoteConversations();
    } catch (_) {
      if (cached != null) {
        return cached;
      }
      rethrow;
    }
  }

  Future<void> refresh() async {
    state = AsyncValue.data(await _fetchRemoteConversations());
  }

  Future<ConversationsResponse> _fetchRemoteConversations() async {
    final repository = ref.read(socialRepositoryProvider);
    final response = await repository.conversations();
    await repository.saveConversations(response.conversations);
    return response;
  }

  Future<void> _refreshFromRemote() async {
    try {
      state = AsyncValue.data(await _fetchRemoteConversations());
    } catch (_) {}
  }
}
