import 'dart:async';
import 'dart:convert';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/social/social_conversation_notifier.dart';
import 'package:minos/application/social/social_inbox_notifier.dart';
import 'package:minos/data/repositories/realtime_events_repository.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Consume `snapshot_required` UiEvent (Rust realtime) → TimelineSync / InboxSync.
///
/// Conversation topic: only reconcile when [socialConversationProvider] already
/// exists (chat open). Never cold-start autoDispose for a closed chat (would
/// race mark-read / clear-unread). Background: no-op; next open rebuilds.
/// Account topic: full inbox hydrate is OK.
final imSnapshotSyncProvider = Provider<void>((ref) {
  final repo = ref.watch(realtimeEventsRepositoryProvider);
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
