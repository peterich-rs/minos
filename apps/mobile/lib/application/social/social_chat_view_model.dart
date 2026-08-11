import 'package:minos/application/social/social_conversation_notifier.dart';
import 'package:minos/application/social/social_conversation_state.dart';
import 'package:minos/application/social/social_ui_state.dart';
import 'package:minos/domain/social_message.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'social_chat_view_model.g.dart';

/// Feature-level ViewModel for the open conversation surface.
///
/// UI should prefer this aggregation over watching multiple fine-grained
/// providers: timeline state, reply draft target, and intentful actions.
@riverpod
class SocialChatViewModel extends _$SocialChatViewModel {
  @override
  SocialConversationState build(String conversationId) {
    return ref.watch(socialConversationProvider(conversationId));
  }

  SocialConversation get _timeline =>
      ref.read(socialConversationProvider(conversationId).notifier);

  SocialChatMessage? get replyTarget =>
      ref.read(socialReplyMessageProvider(conversationId));

  Future<void> send(String text) async {
    final reply = replyTarget;
    await _timeline.sendMessage(text, replyToMessage: reply);
    ref.read(socialReplyDraftProvider(conversationId).notifier).clear();
  }

  Future<void> retry(String localId) => _timeline.retryMessage(localId);

  Future<void> recall(String localId) => _timeline.recallMessage(localId);

  Future<void> toggleReaction({
    required String messageId,
    required String emoji,
  }) {
    return _timeline.toggleReaction(messageId: messageId, emoji: emoji);
  }

  Future<void> loadOlder() => _timeline.loadOlder();

  Future<void> refresh() => _timeline.refresh();

  void selectReply(String localId) {
    ref.read(socialReplyDraftProvider(conversationId).notifier).select(localId);
  }

  void clearReply() {
    ref.read(socialReplyDraftProvider(conversationId).notifier).clear();
  }
}
