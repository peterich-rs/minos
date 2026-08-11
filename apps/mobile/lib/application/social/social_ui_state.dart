import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'social_ui_state.g.dart';

/// Currently open social chat conversation (focused for unread / markRead).
/// Distinct from "has timeline window" (provider alive with messages).
@Riverpod(keepAlive: true)
class FocusedSocialConversationId extends _$FocusedSocialConversationId {
  @override
  String? build() => null;

  void set(String? conversationId) => state = conversationId;

  void clear() => state = null;
}

/// Last realtime subscription limit breach (R4). Shell can toast/banner.
class SubscriptionLimitNotice {
  const SubscriptionLimitNotice({
    required this.limit,
    required this.current,
    required this.atMs,
  });
  final int limit;
  final int current;
  final int atMs;
}

@Riverpod(keepAlive: true)
class SubscriptionLimitNoticeController
    extends _$SubscriptionLimitNoticeController {
  @override
  SubscriptionLimitNotice? build() => null;

  void publish(SubscriptionLimitNotice notice) => state = notice;

  void clear() => state = null;
}

/// Compatibility alias for existing listeners.
final subscriptionLimitNoticeProvider =
    subscriptionLimitNoticeControllerProvider;

final socialProfileProvider = FutureProvider<MyProfileResponse>((ref) {
  return ref.watch(socialRepositoryProvider).myProfile();
});

/// Human members derived from unified participants (ADR 0021).
final conversationMembersProvider = FutureProvider.family
    .autoDispose<List<UserSummary>, String>((ref, conversationId) async {
      final participants = await ref.watch(
        conversationParticipantsProvider(conversationId).future,
      );
      return participants.humans;
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
