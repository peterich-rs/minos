import 'package:minos/src/rust/api/minos.dart';

/// Sort conversations by last activity descending (last message time).
///
/// Tie-break (semantic, not UUID noise): title ascending, then conversationId
/// ascending for stability when titles also match.
List<ConversationSummary> sortConversationsByLastActive(
  List<ConversationSummary> items,
) {
  final sorted = List<ConversationSummary>.of(items)
    ..sort((a, b) {
      final byTime = b.lastMessageAtMs.toInt().compareTo(
        a.lastMessageAtMs.toInt(),
      );
      if (byTime != 0) return byTime;
      final byTitle = a.title.compareTo(b.title);
      if (byTitle != 0) return byTitle;
      return a.conversationId.compareTo(b.conversationId);
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
