import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/src/rust/api/minos.dart';

final socialProfileProvider = FutureProvider<MyProfileResponse>((ref) {
  return ref.watch(minosCoreProvider).myProfile();
});

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
