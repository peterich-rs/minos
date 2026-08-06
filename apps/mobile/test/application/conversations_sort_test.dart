import 'package:flutter_test/flutter_test.dart';
import 'package:minos/application/conversations_sort.dart';
import 'package:minos/infrastructure/platform_int64.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  group('sortConversationsByLastActive', () {
    ConversationSummary conversation({
      required String id,
      required int lastMessageAtMs,
      String title = 't',
    }) {
      return ConversationSummary(
        conversationId: id,
        kind: ConversationKind.direct,
        title: title,
        memberCount: 2,
        lastMessageAtMs: platformInt64FromInt(lastMessageAtMs),
        unreadCount: 0,
        unreadMentionCount: 0,
      );
    }

    test('orders by lastMessageAtMs descending', () {
      final items = <ConversationSummary>[
        conversation(id: 'a', lastMessageAtMs: 100),
        conversation(id: 'b', lastMessageAtMs: 300),
        conversation(id: 'c', lastMessageAtMs: 200),
      ];

      final sorted = sortConversationsByLastActive(items);

      expect(sorted.map((c) => c.conversationId).toList(), <String>[
        'b',
        'c',
        'a',
      ]);
    });

    test('tie-breaks by title then conversationId ascending', () {
      final items = <ConversationSummary>[
        conversation(id: 'c-z', lastMessageAtMs: 50, title: 'Zulu'),
        conversation(id: 'c-a', lastMessageAtMs: 50, title: 'Alpha'),
        conversation(id: 'c-b', lastMessageAtMs: 50, title: 'Alpha'),
      ];

      final sorted = sortConversationsByLastActive(items);

      expect(sorted.map((c) => c.conversationId).toList(), <String>[
        'c-a',
        'c-b',
        'c-z',
      ]);
    });

    test('conversationsSortedByLastActive wraps response', () {
      final response = ConversationsResponse(
        conversations: <ConversationSummary>[
          conversation(id: 'old', lastMessageAtMs: 1),
          conversation(id: 'new', lastMessageAtMs: 9),
        ],
      );

      final sorted = conversationsSortedByLastActive(response);

      expect(sorted.conversations.first.conversationId, 'new');
      expect(sorted.conversations.last.conversationId, 'old');
    });
  });
}
