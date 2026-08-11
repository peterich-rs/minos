import 'dart:async';
import 'dart:convert';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/social/social_ui_state.dart';
import 'package:minos/data/repositories/realtime_events_repository.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/src/rust/api/minos.dart';

final friendsProvider =
    AsyncNotifierProvider<FriendsController, FriendsResponse>(
      FriendsController.new,
    );

class FriendsController extends AsyncNotifier<FriendsResponse> {
  @override
  Future<FriendsResponse> build() {
    ref.watch(friendRequestRealtimeSyncProvider);
    return ref.watch(socialRepositoryProvider).friends();
  }

  Future<void> refresh() async {
    state = AsyncValue.data(await ref.read(socialRepositoryProvider).friends());
  }
}

/// T2 FriendRequestUpdated durable → refresh friend list (HTTP).
/// Also surfaces subscription_limit_exceeded for shell notice (R4).
final friendRequestRealtimeSyncProvider = Provider<void>((ref) {
  final repo = ref.watch(realtimeEventsRepositoryProvider);
  final sub = repo.uiEvents.listen((frame) {
    final ui = frame.ui;
    if (ui is! UiEventMessage_Raw) return;
    if (ui.kind == 'friend_request_updated') {
      unawaited(ref.read(friendsProvider.notifier).refresh());
      return;
    }
    if (ui.kind == 'subscription_limit_exceeded') {
      int limit = 0;
      int current = 0;
      try {
        final payload = jsonDecode(ui.payloadJson) as Map<String, dynamic>?;
        limit = (payload?['limit'] as num?)?.toInt() ?? 0;
        current = (payload?['current'] as num?)?.toInt() ?? 0;
      } catch (_) {}
      ref
          .read(subscriptionLimitNoticeControllerProvider.notifier)
          .publish(
            SubscriptionLimitNotice(
              limit: limit,
              current: current,
              atMs: DateTime.now().millisecondsSinceEpoch,
            ),
          );
    }
  });
  ref.onDispose(sub.cancel);
});
