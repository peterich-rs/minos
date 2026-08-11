import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/conversations_sort.dart';
import 'package:minos/application/flutter_log.dart';
import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/application/im_outbox_worker.dart';
import 'package:minos/application/social/social_conversation_notifier.dart';
import 'package:minos/application/social/social_realtime_sync.dart';
import 'package:minos/application/social/social_ui_state.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/message_sender_ext.dart';
import 'package:minos/src/rust/api/minos.dart';

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
      ref.invalidate(conversationParticipantsProvider(conversationId));
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
        message.sender.accountIdOrNull == myAccountId;
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
    // Reactions are conversation-only; do not bump sidebar unread.
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
      logFlutterWarn(
        'social_providers',
        'inbox social apply failed (cursor held)',
        error: e,
        stackTrace: st,
      );
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
