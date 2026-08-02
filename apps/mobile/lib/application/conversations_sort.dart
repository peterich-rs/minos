import 'package:minos/src/rust/api/minos.dart';

/// Sort conversations by last activity descending (last message time).
///
/// Tie-break: [ConversationSummary.conversationId] descending for stability.
List<ConversationSummary> sortConversationsByLastActive(
  List<ConversationSummary> items,
) {
  final sorted = List<ConversationSummary>.of(items)
    ..sort((a, b) {
      final byTime = b.lastMessageAtMs.toInt().compareTo(
        a.lastMessageAtMs.toInt(),
      );
      if (byTime != 0) return byTime;
      return b.conversationId.compareTo(a.conversationId);
    });
  return sorted;
}

/// Return a [ConversationsResponse] with conversations sorted by last activity.
ConversationsResponse conversationsSortedByLastActive(
  ConversationsResponse response,
) {
  return ConversationsResponse(
    conversations: sortConversationsByLastActive(response.conversations),
  );
}
