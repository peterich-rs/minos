import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/social/social_chat_view_model.dart';
import 'package:minos/domain/social_message.dart';

/// Intentful action facade for [SocialChatPage].
///
/// Keeps widgets free of multi-provider orchestration; ViewModel owns state.
final socialChatActionsProvider = Provider.family<SocialChatActions, String>((
  ref,
  conversationId,
) {
  return SocialChatActions(ref, conversationId);
});

class SocialChatActions {
  SocialChatActions(this._ref, this.conversationId);

  final Ref _ref;
  final String conversationId;

  SocialChatViewModel get _vm =>
      _ref.read(socialChatViewModelProvider(conversationId).notifier);

  Future<void> send(String text) => _vm.send(text);

  Future<void> retry(String localId) => _vm.retry(localId);

  Future<void> recall(String localId) => _vm.recall(localId);

  Future<void> toggleReaction({
    required String messageId,
    required String emoji,
  }) {
    return _vm.toggleReaction(messageId: messageId, emoji: emoji);
  }

  Future<void> loadOlder() => _vm.loadOlder();

  Future<void> refresh() => _vm.refresh();

  void selectReply(SocialChatMessage message) =>
      _vm.selectReply(message.localId);

  void clearReply() => _vm.clearReply();
}
