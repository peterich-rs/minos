import 'dart:async';
import 'dart:convert';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart' show StateProvider;
import 'package:minos/application/agent_activity_provider.dart';
import 'package:minos/application/conversations_sort.dart';
import 'package:minos/application/im_outbox_worker.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/data/repositories/thread_repository.dart';
import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/domain/social_message_order.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'social_providers.g.dart';

/// Currently open social chat conversation (focused for unread / markRead).
/// Distinct from "has timeline window" (provider alive with messages).
final focusedSocialConversationIdProvider = StateProvider<String?>((ref) {
  return null;
});

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

final subscriptionLimitNoticeProvider = StateProvider<SubscriptionLimitNotice?>(
  (ref) => null,
);

/// Consume `snapshot_required` UiEvent (Rust realtime) → TimelineSync / InboxSync.
///
/// Conversation topic: only reconcile when [socialConversationProvider] already
/// exists (chat open). Never cold-start autoDispose for a closed chat (would
/// race mark-read / clear-unread). Background: no-op; next open rebuilds.
/// Account topic: full inbox hydrate is OK.
final imSnapshotSyncProvider = Provider<void>((ref) {
  final repo = ref.watch(threadRepositoryProvider);
  final sub = repo.uiEvents.listen((frame) {
    final ui = frame.ui;
    if (ui is! UiEventMessage_Raw || ui.kind != 'snapshot_required') return;
    try {
      final map = jsonDecode(ui.payloadJson);
      if (map is! Map) return;
      final topic = map['topic']?.toString().trim() ?? '';
      if (topic.isEmpty) return;
      if (topic.startsWith('conversation:')) {
        final conversationId = topic.substring('conversation:'.length).trim();
        if (conversationId.isEmpty) return;
        final timeline = socialConversationProvider(conversationId);
        // Prefer exists: do not materialize autoDispose for closed chats.
        if (!ref.exists(timeline)) {
          return;
        }
        unawaited(ref.read(timeline.notifier).onSnapshotRequired());
        return;
      }
      if (topic.startsWith('account:')) {
        unawaited(ref.read(conversationsProvider.notifier).onAccountSnapshot());
      }
    } catch (_) {
      // Malformed snapshot payload — ignore; next hydrate corrects.
    }
  });
  ref.onDispose(sub.cancel);
});

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

/// Timeline window + meta for one conversation (TimelineSync).
class SocialConversationState {
  const SocialConversationState({
    required this.myAccountId,
    required this.messages,
    required this.isLoading,
    required this.error,
    this.minLoadedSeq,
    this.maxLoadedSeq,
    this.hasOlder = false,
    this.loadingOlder = false,
  });

  const SocialConversationState.initial()
    : myAccountId = null,
      messages = const <SocialChatMessage>[],
      isLoading = true,
      error = null,
      minLoadedSeq = null,
      maxLoadedSeq = null,
      hasOlder = false,
      loadingOlder = false;

  final String? myAccountId;
  final List<SocialChatMessage> messages;
  final int? minLoadedSeq;
  final int? maxLoadedSeq;
  final bool hasOlder;
  final bool loadingOlder;
  final bool isLoading;
  final Object? error;

  SocialConversationState copyWith({
    String? myAccountId,
    List<SocialChatMessage>? messages,
    int? minLoadedSeq,
    int? maxLoadedSeq,
    bool? hasOlder,
    bool? loadingOlder,
    bool? isLoading,
    Object? error = _socialConversationUnset,
  }) {
    return SocialConversationState(
      myAccountId: myAccountId ?? this.myAccountId,
      messages: messages ?? this.messages,
      minLoadedSeq: minLoadedSeq ?? this.minLoadedSeq,
      maxLoadedSeq: maxLoadedSeq ?? this.maxLoadedSeq,
      hasOlder: hasOlder ?? this.hasOlder,
      loadingOlder: loadingOlder ?? this.loadingOlder,
      isLoading: isLoading ?? this.isLoading,
      error: identical(error, _socialConversationUnset) ? this.error : error,
    );
  }

  SocialConversationState withMessages(List<SocialChatMessage> next) {
    return copyWith(
      messages: next,
      minLoadedSeq: minLoadedSeqOf(next),
      maxLoadedSeq: maxLoadedSeqOf(next),
    );
  }
}

@riverpod
class SocialConversation extends _$SocialConversation {
  static const int _pageSize = 100;
  static const Duration _markReadDebounce = Duration(milliseconds: 400);

  StreamSubscription<SocialEventFrame>? _eventsSub;
  Timer? _markReadTimer;

  late final String _conversationId;

  @override
  SocialConversationState build(String conversationId) {
    const initialState = SocialConversationState.initial();
    _conversationId = conversationId;
    unawaited(_eventsSub?.cancel() ?? Future<void>.value());
    _eventsSub = ref
        .read(socialRepositoryProvider)
        .socialEvents
        .listen(
          _onSocialEvent,
          // Soft: connection churn must not rebuild + mark-read the open chat.
          onError: (Object error, StackTrace stackTrace) {},
          onDone: () {},
        );
    // R3a: open chat subscribes conversation topic for full T1 live frames.
    // Account topic digests are inbox-only and must not drive this timeline.
    final repository = ref.read(socialRepositoryProvider);
    unawaited(repository.subscribeConversation(conversationId: conversationId));
    ref.onDispose(() {
      unawaited(_eventsSub?.cancel() ?? Future<void>.value());
      _markReadTimer?.cancel();
      unawaited(
        repository.unsubscribeConversation(conversationId: conversationId),
      );
    });
    unawaited(_load(seedState: initialState));
    return initialState;
  }

  Future<void> refresh() => _load();

  /// Cache-only reload after outbox ack (no REST, no mark-read, no subscribe).
  /// Used when the conversation is already open; never materialize closed chats.
  Future<void> reloadFromLocalCache() async {
    final repository = ref.read(socialRepositoryProvider);
    final messages = await repository.loadMessages(_conversationId);
    state = state.withMessages(messages).copyWith(error: null);
  }

  /// Older page via Hub `before_seq` (TimelineSync.loadOlder).
  Future<void> loadOlder() async {
    if (state.loadingOlder || !state.hasOlder) {
      return;
    }
    final minSeq = state.minLoadedSeq;
    if (minSeq == null || minSeq <= 1) {
      state = state.copyWith(hasOlder: false, loadingOlder: false);
      return;
    }

    state = state.copyWith(loadingOlder: true);
    final repository = ref.read(socialRepositoryProvider);
    try {
      final response = await repository.listChatMessages(
        conversationId: _conversationId,
        beforeSeq: minSeq,
        limit: _pageSize,
      );
      await repository.upsertRemoteMessages(
        conversationId: _conversationId,
        messages: response.messages,
      );
      final messages = await repository.loadMessages(_conversationId);
      final hasOlder =
          response.nextBeforeSeq != null ||
          response.messages.length >= _pageSize;
      state = state
          .withMessages(messages)
          .copyWith(hasOlder: hasOlder, loadingOlder: false, error: null);
    } catch (error) {
      state = state.copyWith(loadingOlder: false, error: error);
    }
  }

  /// SnapshotRequired range reconcile: keep window, forward fill + latest page.
  /// Timeline-only: does **not** mark-read or clear unread (caller must only
  /// invoke when this provider already exists / chat is open).
  Future<void> onSnapshotRequired() async {
    final repository = ref.read(socialRepositoryProvider);
    final prev = state.messages;
    final maxSeq = state.maxLoadedSeq ?? maxLoadedSeqOf(prev);
    try {
      if (maxSeq != null) {
        final forward = await repository.listChatMessages(
          conversationId: _conversationId,
          afterSeq: maxSeq,
          limit: _pageSize,
        );
        await repository.upsertRemoteMessages(
          conversationId: _conversationId,
          messages: forward.messages,
        );
      }
      final latest = await repository.listChatMessages(
        conversationId: _conversationId,
        limit: _pageSize,
      );
      await repository.upsertRemoteMessages(
        conversationId: _conversationId,
        messages: latest.messages,
      );
      final messages = await repository.loadMessages(_conversationId);
      final hasOlder =
          latest.nextBeforeSeq != null ||
          latest.messages.length >= _pageSize ||
          state.hasOlder;
      state = state
          .withMessages(messages)
          .copyWith(hasOlder: hasOlder, isLoading: false, error: null);
      // No mark-read here: open path / inbound debounce own unread.
    } catch (error) {
      state = state.copyWith(error: error, isLoading: false);
    }
  }

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
    final clientMessageId = pending.wireClientMessageId;
    await repository.enqueueUserMessageOutbox(
      clientMessageId: clientMessageId,
      conversationId: _conversationId,
      text: trimmed,
      replyToMessageId: replyPreview?.messageId,
    );
    await repository.touchConversationPreview(
      conversationId: _conversationId,
      preview: trimmed,
      createdAtMs: pending.createdAtMs,
    );
    final messages = await repository.loadMessages(_conversationId);
    state = state.withMessages(messages).copyWith(error: null);
    // Inbox patch without full REST.
    unawaited(
      ref
          .read(conversationsProvider.notifier)
          .patchPreview(
            conversationId: _conversationId,
            preview: trimmed,
            lastMessageAtMs: pending.createdAtMs,
            unreadCount: 0,
          ),
    );

    // Durable outbox owns transport; kick worker without blocking UI on REST.
    unawaited(ref.read(imOutboxWorkerProvider).ensureStarted());
    unawaited(ref.read(imOutboxWorkerProvider).flush());
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
    final clientMessageId = target.wireClientMessageId;
    final replyToMessageId = target.replyTo?.recalledAtMs == null
        ? target.replyTo?.messageId
        : null;

    await repository.markMessageSending(localId);
    // Reuse the same client_message_id — never mint a new key on retry.
    await repository.enqueueUserMessageOutbox(
      clientMessageId: clientMessageId,
      conversationId: _conversationId,
      text: target.text,
      replyToMessageId: replyToMessageId,
    );
    final messages = await repository.loadMessages(_conversationId);
    state = state.withMessages(messages);

    unawaited(ref.read(imOutboxWorkerProvider).ensureStarted());
    unawaited(ref.read(imOutboxWorkerProvider).flush());
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
    final messages = await repository.loadMessages(_conversationId);
    state = state.withMessages(messages);
    final replyDraft = ref.read(socialReplyDraftProvider(_conversationId));
    if (replyDraft == localId) {
      ref.read(socialReplyDraftProvider(_conversationId).notifier).clear();
    }
    unawaited(
      ref
          .read(conversationsProvider.notifier)
          .patchPreview(
            conversationId: _conversationId,
            preview: message.text,
            lastMessageAtMs: repository.platformInt64ToIntValue(
              message.createdAtMs,
            ),
          ),
    );
  }

  Future<void> _load({SocialConversationState? seedState}) async {
    final repository = ref.read(socialRepositoryProvider);
    final previous = seedState ?? state;
    final cachedMessages = await repository.loadMessages(_conversationId);
    final cachedAccountId = await repository.loadCurrentAccountId();
    state = previous
        .withMessages(
          cachedMessages.isEmpty ? previous.messages : cachedMessages,
        )
        .copyWith(
          myAccountId: cachedAccountId ?? previous.myAccountId,
          isLoading: true,
          error: null,
        );
    try {
      final profile = await repository.myProfile();
      await repository.saveCurrentAccountId(profile.accountId);
      final response = await repository.listChatMessages(
        conversationId: _conversationId,
        limit: _pageSize,
      );
      await repository.upsertRemoteMessages(
        conversationId: _conversationId,
        messages: response.messages,
      );
      await repository.clearUnread(_conversationId);
      final messages = await repository.loadMessages(_conversationId);
      final hasOlder =
          response.nextBeforeSeq != null ||
          response.messages.length >= _pageSize;
      state = SocialConversationState(
        myAccountId: profile.accountId,
        messages: messages,
        minLoadedSeq: minLoadedSeqOf(messages),
        maxLoadedSeq: maxLoadedSeqOf(messages),
        hasOlder: hasOlder,
        loadingOlder: false,
        isLoading: false,
        error: null,
      );
      // Mark-read after state has observed maxLoadedSeq (not before).
      unawaited(_markConversationReadNow());
      ref.invalidate(conversationAgentSessionsProvider(_conversationId));
      unawaited(
        ref
            .read(conversationsProvider.notifier)
            .applyMarkReadLocal(_conversationId),
      );
    } catch (error) {
      final fallback = cachedMessages.isEmpty
          ? previous.messages
          : cachedMessages;
      state = SocialConversationState(
        myAccountId: cachedAccountId ?? previous.myAccountId,
        messages: fallback,
        minLoadedSeq: minLoadedSeqOf(fallback),
        maxLoadedSeq: maxLoadedSeqOf(fallback),
        hasOlder: previous.hasOlder,
        loadingOlder: false,
        isLoading: false,
        error: error,
      );
    }
  }

  void _onSocialEvent(SocialEventFrame frame) {
    if (frame.conversationId != _conversationId) {
      return;
    }
    // Account T2 digests are for inbox only — never timeline.
    if (frame.kind == 'inbox_digest' || frame.kind == 'inbox_recall') {
      return;
    }
    if (frame.kind == 'reaction_updated') {
      final mid = frame.message.messageId.trim();
      if (mid.isEmpty) return;
      unawaited(
        _applyRemoteReactions(mid, frame.message.reactions).then((_) {
          return _ackDurable(frame);
        }),
      );
      return;
    }
    // Full T1 conversation frames only.
    if (frame.kind != 'message') {
      return;
    }
    // Empty shell already filtered in Rust; belt-and-suspenders here.
    if (frame.message.messageId.trim().isEmpty) {
      return;
    }
    unawaited(
      _applyRemoteMessage(frame.message).then((_) => _ackDurable(frame)),
    );
  }

  Future<void> _ackDurable(SocialEventFrame frame) async {
    final topic = frame.topic.trim();
    final seq = frame.topicSeq.toInt();
    if (topic.isEmpty || seq <= 0) return;
    try {
      await ref
          .read(minosCoreServiceProvider)
          .ackDurableApplied(topic: topic, topicSeq: seq);
    } catch (e, st) {
      // Hold cursor on failure — do not swallow without log.
      // ignore: avoid_print
      print('ackDurableApplied failed: $e\n$st');
    }
  }

  Future<void> _applyRemoteReactions(
    String messageId,
    List<ReactionGroup> reactions,
  ) async {
    final repository = ref.read(socialRepositoryProvider);
    await repository.updateMessageReactions(
      conversationId: _conversationId,
      messageId: messageId,
      reactions: reactions,
    );
    final messages = await repository.loadMessages(_conversationId);
    state = state.withMessages(messages).copyWith(error: null);
  }

  /// Toggle reaction via Intent Outbox (C5.2); optimistic UI then worker drain.
  Future<void> toggleReaction({
    required String messageId,
    required String emoji,
  }) async {
    final mid = messageId.trim();
    final em = emoji.trim();
    if (mid.isEmpty || em.isEmpty) return;
    final repository = ref.read(socialRepositoryProvider);
    final clientOpId =
        'react-${DateTime.now().microsecondsSinceEpoch}-${mid.hashCode.abs()}';
    // Optimistic: flip reacted_by_me for this emoji in local list.
    final nextMessages = state.messages
        .map((m) {
          if (m.serverMessageId != mid && m.localId != mid) return m;
          return m.copyWith(reactions: _optimisticToggle(m.reactions, em));
        })
        .toList(growable: false);
    state = state.withMessages(nextMessages);

    await repository.enqueueReactionToggleOutbox(
      clientOpId: clientOpId,
      conversationId: _conversationId,
      messageId: mid,
      emoji: em,
    );
    final worker = ref.read(imOutboxWorkerProvider);
    unawaited(worker.ensureStarted());
    unawaited(worker.flush());
  }

  List<ReactionGroup> _optimisticToggle(
    List<ReactionGroup> prev,
    String emoji,
  ) {
    final list = List<ReactionGroup>.from(prev);
    final idx = list.indexWhere((g) => g.emoji == emoji);
    if (idx < 0) {
      list.add(
        ReactionGroup(
          emoji: emoji,
          count: 1,
          reactedByMe: true,
          actors: const <ReactionActor>[],
        ),
      );
      return list;
    }
    final g = list[idx];
    if (g.reactedByMe) {
      final count = g.count - 1;
      if (count <= 0) {
        list.removeAt(idx);
      } else {
        list[idx] = ReactionGroup(
          emoji: g.emoji,
          count: count,
          reactedByMe: false,
          actors: g.actors,
        );
      }
    } else {
      list[idx] = ReactionGroup(
        emoji: g.emoji,
        count: g.count + 1,
        reactedByMe: true,
        actors: g.actors,
      );
    }
    return list;
  }

  Future<void> _applyRemoteMessage(ChatMessageSummary message) async {
    final repository = ref.read(socialRepositoryProvider);
    await repository.upsertRemoteMessage(message);
    await repository.touchConversationPreview(
      conversationId: message.conversationId,
      preview: message.text,
      createdAtMs: repository.platformInt64ToIntValue(message.createdAtMs),
      unreadCount: 0,
    );
    // Incremental merge: prefer single-row into in-memory list when possible.
    final messages = await repository.loadMessages(_conversationId);
    state = state.withMessages(messages).copyWith(error: null);
    _scheduleMarkRead();
    ref.invalidate(conversationAgentSessionsProvider(_conversationId));
    // Inbox patch: focused → unread 0, no full REST.
    unawaited(
      ref
          .read(conversationsProvider.notifier)
          .patchFromInbound(
            message: message,
            focused: true,
            myAccountId: state.myAccountId,
          ),
    );
  }

  void _scheduleMarkRead() {
    _markReadTimer?.cancel();
    _markReadTimer = Timer(_markReadDebounce, () {
      unawaited(_markConversationReadNow());
    });
  }

  Future<void> _markConversationReadNow() async {
    // Only mark up to the highest Hub message_seq actually loaded/observed.
    // Skip HTTP when no observed seq (avoids silently marking unread rows).
    final observedSeq = state.maxLoadedSeq ?? maxLoadedSeqOf(state.messages);
    if (observedSeq == null || observedSeq <= 0) {
      // Still clear local badge; server watermark stays until we observe seq.
      try {
        await ref.read(socialRepositoryProvider).clearUnread(_conversationId);
        unawaited(
          ref
              .read(conversationsProvider.notifier)
              .applyMarkReadLocal(_conversationId),
        );
      } catch (_) {}
      return;
    }
    try {
      await ref.read(socialRepositoryProvider).markConversationRead(
            conversationId: _conversationId,
            readUpToMessageSeq: observedSeq,
          );
      await ref.read(socialRepositoryProvider).clearUnread(_conversationId);
      unawaited(
        ref
            .read(conversationsProvider.notifier)
            .applyMarkReadLocal(_conversationId),
      );
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
    ref.watch(friendRequestRealtimeSyncProvider);
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
    ref.watch(friendRequestRealtimeSyncProvider);
    return ref.watch(socialRepositoryProvider).friends();
  }

  Future<void> refresh() async {
    state = AsyncValue.data(await ref.read(socialRepositoryProvider).friends());
  }
}

/// T2 FriendRequestUpdated durable → refresh friend / request lists (HTTP).
final friendRequestRealtimeSyncProvider = Provider<void>((ref) {
  final repo = ref.watch(threadRepositoryProvider);
  final sub = repo.uiEvents.listen((frame) {
    final ui = frame.ui;
    if (ui is! UiEventMessage_Raw) return;
    if (ui.kind == 'friend_request_updated') {
      unawaited(ref.read(friendRequestsProvider.notifier).refresh());
      unawaited(ref.read(friendsProvider.notifier).refresh());
      return;
    }
    if (ui.kind == 'subscription_limit_exceeded') {
      // Non-silent: publish notice for shell banner/toast (R4).
      int limit = 0;
      int current = 0;
      try {
        final payload = jsonDecode(ui.payloadJson) as Map<String, dynamic>?;
        limit = (payload?['limit'] as num?)?.toInt() ?? 0;
        current = (payload?['current'] as num?)?.toInt() ?? 0;
      } catch (_) {}
      ref
          .read(subscriptionLimitNoticeProvider.notifier)
          .state = SubscriptionLimitNotice(
        limit: limit,
        current: current,
        atMs: DateTime.now().millisecondsSinceEpoch,
      );
    }
  });
  ref.onDispose(sub.cancel);
});

final conversationsProvider =
    AsyncNotifierProvider<ConversationsController, ConversationsResponse>(
      ConversationsController.new,
    );

/// InboxSync: incremental patch; full REST only hydrate / refresh / snapshot.
class ConversationsController extends AsyncNotifier<ConversationsResponse> {
  StreamSubscription<SocialEventFrame>? _eventsSub;

  @override
  Future<ConversationsResponse> build() async {
    // Arm SnapshotRequired consumer for app lifetime (Inbox/Timeline).
    ref.watch(imSnapshotSyncProvider);

    _eventsSub ??= ref
        .read(socialRepositoryProvider)
        .socialEvents
        .listen(
          (frame) {
            // Hot path: single-row patch — never invalidateSelf / full REST.
            unawaited(_onSocialEvent(frame));
          },
          onError: (Object error, StackTrace stackTrace) {
            // Connection errors: soft; next hydrate corrects.
          },
          onDone: () {},
        );
    ref.onDispose(() => _eventsSub?.cancel());

    // Start IM outbox worker once conversations hydrate; wire UI refresh hooks.
    final outbox = ref.read(imOutboxWorkerProvider);
    // Never materialize autoDispose open-chat for background drain — that
    // would subscribe + mark-read a conversation the user is not viewing.
    outbox.onConversationDirty = (conversationId) {
      final timeline = socialConversationProvider(conversationId);
      if (!ref.exists(timeline)) {
        return;
      }
      unawaited(ref.read(timeline.notifier).reloadFromLocalCache());
    };
    outbox.onInboxDirty = () {
      // Prefer patch if possible; full refresh only when worker cannot patch.
      unawaited(refresh());
    };
    unawaited(outbox.ensureStarted());

    final repository = ref.read(socialRepositoryProvider);
    final cached = await repository.loadConversations();
    if (cached != null && cached.conversations.isNotEmpty) {
      unawaited(_refreshFromRemote());
      return conversationsSortedByLastActive(cached);
    }

    try {
      return await _fetchRemoteConversations();
    } catch (_) {
      if (cached != null) {
        return conversationsSortedByLastActive(cached);
      }
      rethrow;
    }
  }

  Future<void> refresh() async {
    state = AsyncValue.data(await _fetchRemoteConversations());
  }

  /// Account SnapshotRequired / pull-to-refresh entry.
  Future<void> onAccountSnapshot() => refresh();

  Future<void> deleteConversation(String conversationId) async {
    final previous = state;
    final current = previous.asData?.value;
    if (current != null) {
      state = AsyncValue.data(
        conversationsSortedByLastActive(
          ConversationsResponse(
            conversations: current.conversations
                .where(
                  (conversation) =>
                      conversation.conversationId != conversationId,
                )
                .toList(growable: false),
          ),
        ),
      );
    }
    try {
      await ref
          .read(socialRepositoryProvider)
          .deleteConversation(conversationId: conversationId);
      ref.invalidate(socialConversationProvider(conversationId));
      ref.invalidate(conversationMembersProvider(conversationId));
    } catch (error, stackTrace) {
      state = previous;
      Error.throwWithStackTrace(error, stackTrace);
    }
  }

  Future<void> applyMarkReadLocal(String conversationId) async {
    final current = state.asData?.value;
    if (current == null) return;
    final next = current.conversations
        .map((c) {
          if (c.conversationId != conversationId) return c;
          return ConversationSummary(
            conversationId: c.conversationId,
            kind: c.kind,
            title: c.title,
            counterpart: c.counterpart,
            memberCount: c.memberCount,
            lastMessagePreview: c.lastMessagePreview,
            lastMessageAtMs: c.lastMessageAtMs,
            unreadCount: 0,
            unreadMentionCount: 0,
          );
        })
        .toList(growable: false);
    state = AsyncValue.data(
      conversationsSortedByLastActive(
        ConversationsResponse(conversations: next),
      ),
    );
    await ref.read(socialRepositoryProvider).clearUnread(conversationId);
  }

  Future<void> patchPreview({
    required String conversationId,
    required String preview,
    required int lastMessageAtMs,
    int? unreadCount,
  }) async {
    final repository = ref.read(socialRepositoryProvider);
    await repository.touchConversationPreview(
      conversationId: conversationId,
      preview: preview,
      createdAtMs: lastMessageAtMs,
      unreadCount: unreadCount,
    );
    final current = state.asData?.value;
    if (current == null) return;
    final exists = current.conversations.any(
      (c) => c.conversationId == conversationId,
    );
    List<ConversationSummary> next;
    if (exists) {
      next = current.conversations
          .map((c) {
            if (c.conversationId != conversationId) return c;
            final prevMs = repository.platformInt64ToIntValue(
              c.lastMessageAtMs,
            );
            final incoming = lastMessageAtMs > 0 ? lastMessageAtMs : 0;
            final nextMs = incoming > 0
                ? (incoming > prevMs ? incoming : prevMs)
                : prevMs;
            return ConversationSummary(
              conversationId: c.conversationId,
              kind: c.kind,
              title: c.title,
              counterpart: c.counterpart,
              memberCount: c.memberCount,
              lastMessagePreview: preview,
              lastMessageAtMs: repository.platformInt64FromIntValue(nextMs),
              unreadCount: unreadCount ?? c.unreadCount,
              unreadMentionCount: unreadCount == 0 ? 0 : c.unreadMentionCount,
            );
          })
          .toList(growable: false);
    } else {
      next = <ConversationSummary>[
        ConversationSummary(
          conversationId: conversationId,
          kind: ConversationKind.group,
          title: 'Conversation',
          memberCount: 0,
          lastMessagePreview: preview,
          lastMessageAtMs: repository.platformInt64FromIntValue(
            lastMessageAtMs > 0 ? lastMessageAtMs : 0,
          ),
          unreadCount: unreadCount ?? 0,
          unreadMentionCount: 0,
        ),
        ...current.conversations,
      ];
    }
    state = AsyncValue.data(
      conversationsSortedByLastActive(
        ConversationsResponse(conversations: next),
      ),
    );
  }

  Future<void> patchFromInbound({
    required ChatMessageSummary message,
    required bool focused,
    String? myAccountId,
  }) async {
    final repository = ref.read(socialRepositoryProvider);
    final conversationId = message.conversationId;
    if (conversationId.isEmpty || message.messageId.trim().isEmpty) {
      return;
    }
    final createdAtMs = repository.platformInt64ToIntValue(message.createdAtMs);
    final isRecall = message.recalledAtMs != null;
    final isOwn =
        myAccountId != null &&
        myAccountId.isNotEmpty &&
        message.sender.accountId == myAccountId;
    final mention =
        myAccountId != null &&
        message.mentionedAccountIds.contains(myAccountId);

    int unread = 0;
    int unreadMention = 0;
    final prev = await repository.loadConversation(conversationId);
    final prevMs = prev != null
        ? repository.platformInt64ToIntValue(prev.lastMessageAtMs)
        : 0;
    // Monotonic list clock: never regress when a recall/stale frame carries
    // an older createdAtMs than the current rail tip.
    final nextMs = createdAtMs > 0
        ? (createdAtMs > prevMs ? createdAtMs : prevMs)
        : prevMs;
    if (focused || isOwn) {
      unread = 0;
      unreadMention = 0;
      await repository.clearUnread(conversationId);
    } else if (!isRecall) {
      unread = (prev?.unreadCount ?? 0) + 1;
      unreadMention = (prev?.unreadMentionCount ?? 0) + (mention ? 1 : 0);
      await repository.touchConversationPreview(
        conversationId: conversationId,
        preview: message.text,
        createdAtMs: nextMs,
        unreadCount: unread,
        unreadMentionCount: unreadMention,
      );
    } else {
      // Recall: preview only; keep unread as-is (server digest is SSOT).
      unread = prev?.unreadCount ?? 0;
      unreadMention = prev?.unreadMentionCount ?? 0;
      await repository.touchConversationPreview(
        conversationId: conversationId,
        preview: message.text,
        createdAtMs: nextMs,
        unreadCount: unread,
        unreadMentionCount: unreadMention,
      );
    }
    if (focused || isOwn) {
      await repository.touchConversationPreview(
        conversationId: conversationId,
        preview: message.text,
        createdAtMs: nextMs,
        unreadCount: 0,
        unreadMentionCount: 0,
      );
    }

    final current = state.asData?.value;
    if (current == null) return;

    final exists = current.conversations.any(
      (c) => c.conversationId == conversationId,
    );
    final nextLastAt = repository.platformInt64FromIntValue(nextMs);
    List<ConversationSummary> next;
    if (exists) {
      next = current.conversations
          .map((c) {
            if (c.conversationId != conversationId) return c;
            return ConversationSummary(
              conversationId: c.conversationId,
              kind: c.kind,
              title: c.title,
              counterpart: c.counterpart,
              memberCount: c.memberCount,
              lastMessagePreview: message.text,
              lastMessageAtMs: nextLastAt,
              unreadCount: focused || isOwn ? 0 : unread,
              unreadMentionCount: focused || isOwn ? 0 : unreadMention,
            );
          })
          .toList(growable: false);
    } else {
      next = <ConversationSummary>[
        ConversationSummary(
          conversationId: conversationId,
          kind: ConversationKind.group,
          title: prev?.title ?? 'Conversation',
          counterpart: prev?.counterpart,
          memberCount: prev?.memberCount ?? 0,
          lastMessagePreview: message.text,
          lastMessageAtMs: nextLastAt,
          unreadCount: focused || isOwn ? 0 : (isRecall ? 0 : 1),
          unreadMentionCount: focused || isOwn
              ? 0
              : (isRecall ? 0 : (mention ? 1 : 0)),
        ),
        ...current.conversations,
      ];
    }
    state = AsyncValue.data(
      conversationsSortedByLastActive(
        ConversationsResponse(conversations: next),
      ),
    );
  }

  Future<void> _onSocialEvent(SocialEventFrame frame) async {
    // Reactions are conversation-only; do not bump sidebar unread (B6.2).
    // Open conversation controller acks conversation reaction topics.
    if (frame.kind == 'reaction_updated') {
      return;
    }
    // Inbox path: account T2 digests + conversation full (when open) both patch
    // rail preview/unread. Digests carry preview in message.text stub.
    if (frame.kind != 'message' &&
        frame.kind != 'inbox_digest' &&
        frame.kind != 'inbox_recall') {
      return;
    }
    if (frame.message.messageId.trim().isEmpty) {
      return;
    }
    final focusedId = ref.read(focusedSocialConversationIdProvider);
    final focused = focusedId != null && focusedId == frame.conversationId;
    // Open conversation controller also patches; still apply inbox for badge
    // when not focused. When focused, SocialConversation drives markRead.
    String? myAccountId;
    try {
      myAccountId = await ref
          .read(socialRepositoryProvider)
          .loadCurrentAccountId();
    } catch (_) {}
    try {
      await patchFromInbound(
        message: frame.message,
        focused: focused,
        myAccountId: myAccountId,
      );
      // Account digests must ack here (conversation controller ignores them).
      // Conversation message frames: open chat acks; when not open, inbox still
      // needs cursor advance after digest/rail patch.
      final topic = frame.topic.trim();
      final seq = frame.topicSeq.toInt();
      if (topic.isNotEmpty && seq > 0) {
        final isAccountTopic = topic.startsWith('account:');
        final isConversationMessage =
            frame.kind == 'message' && topic.startsWith('conversation:');
        // Open conversation owns conversation: topic ack for messages.
        if (isAccountTopic ||
            frame.kind == 'inbox_digest' ||
            frame.kind == 'inbox_recall' ||
            (isConversationMessage && !focused)) {
          await ref
              .read(minosCoreServiceProvider)
              .ackDurableApplied(topic: topic, topicSeq: seq);
        }
      }
    } catch (e, st) {
      // Do not ack on failure — resume will redeliver.
      // ignore: avoid_print
      print('inbox social apply failed (cursor held): $e\n$st');
    }
  }

  Future<ConversationsResponse> _fetchRemoteConversations() async {
    final repository = ref.read(socialRepositoryProvider);
    final response = await repository.conversations();
    await repository.saveConversations(response.conversations);
    return conversationsSortedByLastActive(response);
  }

  Future<void> _refreshFromRemote() async {
    try {
      state = AsyncValue.data(await _fetchRemoteConversations());
    } catch (_) {}
  }
}
