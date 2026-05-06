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
  @override
  Future<ConversationsResponse> build() {
    return ref.watch(minosCoreProvider).conversations();
  }

  Future<void> refresh() async {
    state = AsyncValue.data(await ref.read(minosCoreProvider).conversations());
  }
}
