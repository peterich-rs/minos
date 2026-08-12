import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/flutter_log.dart';
import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/application/im_outbox_worker.dart';
import 'package:minos/application/social/social_conversation_state.dart';
import 'package:minos/application/social/social_inbox_notifier.dart';
import 'package:minos/application/social/social_ui_state.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/mention_extract.dart';
import 'package:minos/domain/message_sender_ext.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/domain/social_message_order.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'social_conversation_notifier.g.dart';

@riverpod
class SocialConversation extends _$SocialConversation {
  static const int _pageSize = 100;
  static const Duration _markReadDebounce = Duration(milliseconds: 400);

  StreamSubscription<SocialEventFrame>? _eventsSub;
  Timer? _markReadTimer;
  /// Serialize apply+ack so concurrent frames cannot race durable cursors.
  Future<void> _durableApplyChain = Future<void>.value();

  late final String _conversationId;

  @override
  SocialConversationState build(String conversationId) {
    final initialState = SocialConversationState.initial();
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
    final sender = await _localSender();
    // Optimistic structured mentions from current roster so local cache/reload
    // before Hub ack still carries bot targets (Hub remains SSOT after upsert).
    final participants = ref
        .read(conversationParticipantsProvider(_conversationId))
        .asData
        ?.value;
    final optimistic = extractOptimisticMentions(
      text: trimmed,
      selfAccountId: sender.identityId,
      humans: (participants?.humans ?? const [])
          .map(
            (h) => MentionHumanRef(accountId: h.accountId, minosId: h.minosId),
          )
          .toList(growable: false),
      agents: (participants?.agents ?? const [])
          .where(
            (a) =>
                a.status.trim().isEmpty || a.status.toLowerCase() == 'active',
          )
          .map(
            (a) => MentionAgentRef(
              agentId: a.agentId,
              runtimeAgent: a.runtimeAgent,
              name: a.name,
            ),
          )
          .toList(growable: false),
    );
    final pending = await repository.insertPendingMessageWithOutbox(
      conversationId: _conversationId,
      sender: sender,
      text: trimmed,
      replyTo: replyPreview,
      mentionedAccountIds: optimistic.accountIds,
      mentionedAgentIds: optimistic.agentIds,
      structuredMentions: optimistic.structuredMentions,
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
    // Rebuild structured mentions from body + current roster (Hub validates).
    final participants = ref
        .read(conversationParticipantsProvider(_conversationId))
        .asData
        ?.value;
    final selfAccountId = await repository.loadCurrentAccountId();
    final optimistic = extractOptimisticMentions(
      text: target.text,
      selfAccountId: selfAccountId,
      humans: (participants?.humans ?? const [])
          .map(
            (h) => MentionHumanRef(accountId: h.accountId, minosId: h.minosId),
          )
          .toList(growable: false),
      agents: (participants?.agents ?? const [])
          .where(
            (a) =>
                a.status.trim().isEmpty || a.status.toLowerCase() == 'active',
          )
          .map(
            (a) => MentionAgentRef(
              agentId: a.agentId,
              runtimeAgent: a.runtimeAgent,
              name: a.name,
            ),
          )
          .toList(growable: false),
    );
    await repository.enqueueUserMessageOutbox(
      clientMessageId: clientMessageId,
      conversationId: _conversationId,
      text: target.text,
      replyToMessageId: replyToMessageId,
      structuredMentions: optimistic.structuredMentions,
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
      _durableApplyChain = _durableApplyChain.catchError((_) {}).then((_) async {
        await _applyRemoteReactions(mid, frame.message.reactions);
        await _ackDurable(frame);
      });
      unawaited(_durableApplyChain);
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
    _durableApplyChain = _durableApplyChain.catchError((_) {}).then((_) async {
      await _applyRemoteMessage(frame.message);
      await _ackDurable(frame);
    });
    unawaited(_durableApplyChain);
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
      logFlutterWarn(
        'social_providers',
        'ackDurableApplied failed',
        error: e,
        stackTrace: st,
      );
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

  /// Toggle reaction via Intent Outbox; optimistic UI then worker drain.
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
      await ref
          .read(socialRepositoryProvider)
          .markConversationRead(
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

  Future<MessageSender> _localSender() async {
    final accountId =
        state.myAccountId ??
        await ref.read(socialRepositoryProvider).loadCurrentAccountId() ??
        'local-self';
    return MessageSender.account(
      accountId: accountId,
      minosId: 'me',
      displayName: '我',
    );
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
