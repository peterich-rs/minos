import 'dart:async';
import 'dart:convert';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/data/repositories/runtime_repository.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/infrastructure/im_outbox_store.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Background IM Outbox drain: Connected edge + periodic tick.
///
/// KeepAlive so send path can fire-and-forget after enqueue.
///
/// Avoids importing social_providers to prevent circular deps; conversation
/// UI reloads from cache on social events / next open.
class ImOutboxWorker {
  ImOutboxWorker(this._ref);

  final Ref _ref;
  Timer? _timer;
  bool _flushing = false;
  StreamSubscription<ConnectionState>? _connSub;
  bool _started = false;
  bool _connected = false;

  /// Optional UI hooks set by social layer after providers are ready.
  void Function(String conversationId)? onConversationDirty;
  void Function()? onInboxDirty;

  void dispose() {
    _timer?.cancel();
    unawaited(_connSub?.cancel());
  }

  /// Call once after social layer is ready.
  Future<void> ensureStarted() async {
    if (_started) {
      return;
    }
    _started = true;
    final repository = _ref.read(socialRepositoryProvider);
    await repository.reconcileSendingMessagesOnStartup();

    final runtime = _ref.read(runtimeRepositoryProvider);
    _connected = runtime.currentConnectionState is ConnectionState_Connected;

    _timer?.cancel();
    _timer = Timer.periodic(const Duration(seconds: 2), (_) {
      unawaited(flush());
    });

    await _connSub?.cancel();
    _connSub = runtime.connectionStates.listen((state) {
      final was = _connected;
      _connected = state is ConnectionState_Connected;
      // Connected edge (and while online): drain pending after long offline.
      if (_connected) {
        unawaited(flush());
      } else if (was) {
        // Dropped offline — do not flush (avoids burning attempts).
      }
    });

    unawaited(flush());
  }

  Future<void> flush() async {
    if (_flushing) {
      return;
    }
    // No-op while offline — transient errors must not burn attempts.
    if (!_connected) {
      return;
    }
    _flushing = true;
    try {
      final repository = _ref.read(socialRepositoryProvider);
      // Per-conversation FIFO: on head failure, skip that lane's tail this
      // cycle; other conversations may still progress.
      final lanes = await repository.listDueOutboxLanes();
      for (final lane in lanes) {
        for (final entry in lane) {
          final ok = await _flushOne(repository, entry);
          if (!ok) break;
        }
      }
    } finally {
      _flushing = false;
    }
  }

  /// Returns true when the entry was acked (lane may continue).
  Future<bool> _flushOne(
    SocialRepository repository,
    ImOutboxEntry entry,
  ) async {
    switch (entry.kind) {
      case ImOutboxKind.userMessage:
        return _flushUserMessage(repository, entry);
      case ImOutboxKind.reactionToggle:
        return _flushReactionToggle(repository, entry);
      case ImOutboxKind.unsupported:
        await repository.markOutboxFailed(
          clientOpId: entry.clientOpId,
          error: 'unknown_kind',
        );
        return false;
    }
  }

  Future<bool> _flushUserMessage(
    SocialRepository repository,
    ImOutboxEntry entry,
  ) async {
    Map<String, dynamic> payload;
    try {
      payload = jsonDecode(entry.payloadJson) as Map<String, dynamic>;
    } catch (_) {
      await repository.markOutboxFailed(
        clientOpId: entry.clientOpId,
        error: 'invalid_payload_json',
      );
      return false;
    }

    final text = (payload['text'] as String?)?.trim() ?? '';
    if (text.isEmpty) {
      await repository.markOutboxFailed(
        clientOpId: entry.clientOpId,
        error: 'empty_text',
      );
      return false;
    }
    final replyTo = payload['reply_to_message_id'] as String?;
    // Structured MentionTarget list for AppendMessage (body never invents targets).
    final mentionsJson = _mentionsJsonFromPayload(payload);

    await repository.markOutboxInflight(entry.clientOpId);
    await repository.markMessageSending(entry.clientOpId);

    try {
      final message = await repository.sendChatMessage(
        conversationId: entry.conversationId,
        text: text,
        replyToMessageId: replyTo,
        clientMessageId: entry.clientOpId,
        mentionsJson: mentionsJson,
      );
      await repository.markMessageSent(
        localId: entry.clientOpId,
        message: message,
      );
      await repository.markOutboxAcked(entry.clientOpId);
      await repository.touchConversationPreview(
        conversationId: entry.conversationId,
        preview: message.text,
        createdAtMs: repository.platformInt64ToIntValue(message.createdAtMs),
      );
      onConversationDirty?.call(entry.conversationId);
      onInboxDirty?.call();
      return true;
    } catch (error) {
      await repository.markOutboxFailed(
        clientOpId: entry.clientOpId,
        error: error.toString(),
      );
      onConversationDirty?.call(entry.conversationId);
      return false;
    }
  }

  /// Encode outbox `mentions` array as JSON string for FFI, or null when empty.
  String? _mentionsJsonFromPayload(Map<String, dynamic> payload) {
    final raw = payload['mentions'];
    if (raw is! List || raw.isEmpty) {
      return null;
    }
    return jsonEncode(raw);
  }

  Future<bool> _flushReactionToggle(
    SocialRepository repository,
    ImOutboxEntry entry,
  ) async {
    Map<String, dynamic> payload;
    try {
      payload = jsonDecode(entry.payloadJson) as Map<String, dynamic>;
    } catch (_) {
      await repository.markOutboxFailed(
        clientOpId: entry.clientOpId,
        error: 'invalid_payload_json',
      );
      return false;
    }
    final messageId = (payload['message_id'] as String?)?.trim() ?? '';
    final emoji = (payload['emoji'] as String?)?.trim() ?? '';
    if (messageId.isEmpty || emoji.isEmpty) {
      await repository.markOutboxFailed(
        clientOpId: entry.clientOpId,
        error: 'invalid_payload: reaction requires message_id+emoji',
      );
      return false;
    }

    await repository.markOutboxInflight(entry.clientOpId);
    try {
      final result = await repository.toggleReaction(
        conversationId: entry.conversationId,
        messageId: messageId,
        emoji: emoji,
        clientOpId: entry.clientOpId,
      );
      await repository.updateMessageReactions(
        conversationId: entry.conversationId,
        messageId: messageId,
        reactions: result.reactions,
      );
      await repository.markOutboxAcked(entry.clientOpId);
      onConversationDirty?.call(entry.conversationId);
      return true;
    } catch (error) {
      await repository.markOutboxFailed(
        clientOpId: entry.clientOpId,
        error: error.toString(),
      );
      // Open chat (if any) reloads cache to drop optimistic toggle.
      onConversationDirty?.call(entry.conversationId);
      return false;
    }
  }
}

final imOutboxWorkerProvider = Provider<ImOutboxWorker>((ref) {
  final worker = ImOutboxWorker(ref);
  ref.onDispose(worker.dispose);
  return worker;
});

/// Watch this (e.g. from conversations root) to start the outbox worker.
final imOutboxBootstrapProvider = Provider<void>((ref) {
  unawaited(ref.watch(imOutboxWorkerProvider).ensureStarted());
});
