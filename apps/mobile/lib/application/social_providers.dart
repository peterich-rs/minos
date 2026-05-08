import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/src/rust/api/minos.dart';

part 'social_providers.g.dart';

final socialProfileProvider = FutureProvider<MyProfileResponse>((ref) {
  return ref.watch(minosCoreProvider).myProfile();
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
      return ref.watch(minosCoreProvider).searchUsers(minosId: trimmed);
    });

final conversationMembersProvider = FutureProvider.family
    .autoDispose<List<UserSummary>, String>((ref, conversationId) async {
      final response = await ref
          .watch(minosCoreProvider)
          .conversationMembers(conversationId: conversationId);
      return response.members;
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
    required this.isSending,
    required this.error,
  });

  const SocialConversationState.initial()
    : myAccountId = null,
      messages = const <ChatMessageSummary>[],
      isLoading = true,
      isSending = false,
      error = null;

  final String? myAccountId;
  final List<ChatMessageSummary> messages;
  final bool isLoading;
  final bool isSending;
  final Object? error;

  SocialConversationState copyWith({
    String? myAccountId,
    List<ChatMessageSummary>? messages,
    bool? isLoading,
    bool? isSending,
    Object? error = _socialConversationUnset,
  }) {
    return SocialConversationState(
      myAccountId: myAccountId ?? this.myAccountId,
      messages: messages ?? this.messages,
      isLoading: isLoading ?? this.isLoading,
      isSending: isSending ?? this.isSending,
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
    _conversationId = conversationId;
    _eventsSub?.cancel();
    _eventsSub = ref.read(minosCoreProvider).socialEvents.listen(
      _onSocialEvent,
    );
    ref.onDispose(() => _eventsSub?.cancel());
    unawaited(_load());
    return const SocialConversationState.initial();
  }

  Future<void> refresh() => _load();

  Future<void> sendMessage(String text) async {
    final trimmed = text.trim();
    if (trimmed.isEmpty || state.isSending) {
      return;
    }

    state = state.copyWith(isSending: true, error: null);
    try {
      final message = await ref.read(minosCoreProvider).sendChatMessage(
        conversationId: _conversationId,
        text: trimmed,
      );
      state = state.copyWith(
        messages: _mergeMessage(state.messages, message),
        isSending: false,
      );
      ref.invalidate(conversationsProvider);
    } catch (error) {
      state = state.copyWith(isSending: false);
      rethrow;
    }
  }

  Future<void> _load() async {
    final previous = state;
    state = previous.copyWith(isLoading: true, error: null);
    try {
      final core = ref.read(minosCoreProvider);
      final profile = await core.myProfile();
      final response = await core.listChatMessages(
        conversationId: _conversationId,
        limit: 100,
      );
      await core.markConversationRead(conversationId: _conversationId);
      state = SocialConversationState(
        myAccountId: profile.accountId,
        messages: response.messages,
        isLoading: false,
        isSending: false,
        error: null,
      );
      ref.invalidate(conversationsProvider);
    } catch (error) {
      state = SocialConversationState(
        myAccountId: previous.myAccountId,
        messages: previous.messages,
        isLoading: false,
        isSending: previous.isSending,
        error: error,
      );
    }
  }

  void _onSocialEvent(SocialEventFrame frame) {
    if (frame.conversationId != _conversationId) {
      return;
    }

    final nextMessages = _mergeMessage(state.messages, frame.message);
    if (identical(nextMessages, state.messages)) {
      return;
    }

    state = state.copyWith(messages: nextMessages, error: null);
    unawaited(_markConversationRead());
  }

  Future<void> _markConversationRead() async {
    try {
      await ref
          .read(minosCoreProvider)
          .markConversationRead(conversationId: _conversationId);
      ref.invalidate(conversationsProvider);
    } catch (_) {}
  }

  List<ChatMessageSummary> _mergeMessage(
    List<ChatMessageSummary> existing,
    ChatMessageSummary incoming,
  ) {
    if (existing.any((message) => message.messageId == incoming.messageId)) {
      return existing;
    }
    return <ChatMessageSummary>[...existing, incoming];
  }
}

final friendRequestsProvider =
    AsyncNotifierProvider<FriendRequestsController, FriendRequestsResponse>(
      FriendRequestsController.new,
    );

class FriendRequestsController extends AsyncNotifier<FriendRequestsResponse> {
  @override
  Future<FriendRequestsResponse> build() {
    return ref.watch(minosCoreProvider).friendRequests();
  }

  Future<void> refresh() async {
    state = AsyncValue.data(await ref.read(minosCoreProvider).friendRequests());
  }
}

final friendsProvider =
    AsyncNotifierProvider<FriendsController, FriendsResponse>(
      FriendsController.new,
    );

class FriendsController extends AsyncNotifier<FriendsResponse> {
  @override
  Future<FriendsResponse> build() {
    return ref.watch(minosCoreProvider).friends();
  }

  Future<void> refresh() async {
    state = AsyncValue.data(await ref.read(minosCoreProvider).friends());
  }
}

final conversationsProvider =
    AsyncNotifierProvider<ConversationsController, ConversationsResponse>(
      ConversationsController.new,
    );

class ConversationsController extends AsyncNotifier<ConversationsResponse> {
  StreamSubscription<SocialEventFrame>? _eventsSub;

  @override
  Future<ConversationsResponse> build() {
    _eventsSub ??= ref.read(minosCoreProvider).socialEvents.listen((_) {
      ref.invalidateSelf();
    });
    ref.onDispose(() => _eventsSub?.cancel());
    return ref.watch(minosCoreProvider).conversations();
  }

  Future<void> refresh() async {
    state = AsyncValue.data(await ref.read(minosCoreProvider).conversations());
  }
}
