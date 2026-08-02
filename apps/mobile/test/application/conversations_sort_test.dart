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

    test('tie-breaks by conversationId descending', () {
      final items = <ConversationSummary>[
        conversation(id: 'alpha', lastMessageAtMs: 50),
        conversation(id: 'zeta', lastMessageAtMs: 50),
        conversation(id: 'beta', lastMessageAtMs: 50),
      ];

      final sorted = sortConversationsByLastActive(items);

      expect(sorted.map((c) => c.conversationId).toList(), <String>[
        'zeta',
        'beta',
        'alpha',
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
