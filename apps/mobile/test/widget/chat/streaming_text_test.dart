import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/presentation/widgets/chat/message_bubble.dart';
import 'package:minos/presentation/widgets/chat/streaming_text.dart';

void main() {
  testWidgets('shows thinking placeholder when accumulated text empty', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: StreamingText(
            messageId: 'm1',
            accumulatedText: '',
            isComplete: false,
          ),
        ),
      ),
    );

    expect(find.byType(MessageBubble), findsOneWidget);
    // Italicised placeholder content is encoded as Markdown italics; the
    // important assertion is that the underlying bubble received non-empty
    // markdown so a thinking glyph is on screen.
    final bubble = tester.widget<MessageBubble>(find.byType(MessageBubble));
    expect(bubble.markdownContent.contains('thinking'), isTrue);
    expect(bubble.isStreaming, isTrue);
  });

  testWidgets('forwards accumulated text into the bubble', (tester) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: StreamingText(
            messageId: 'm1',
            accumulatedText: 'Hel',
            isComplete: false,
          ),
        ),
      ),
    );

    var bubble = tester.widget<MessageBubble>(find.byType(MessageBubble));
    expect(bubble.markdownContent, 'Hel');
    expect(bubble.isStreaming, isTrue);

    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: StreamingText(
            messageId: 'm1',
            accumulatedText: 'Hello',
            isComplete: false,
          ),
        ),
      ),
    );

    bubble = tester.widget<MessageBubble>(find.byType(MessageBubble));
    expect(bubble.markdownContent, 'Hello');
  });

  testWidgets('isComplete=true clears the streaming cursor', (tester) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: StreamingText(
            messageId: 'm1',
            accumulatedText: 'Hello',
            isComplete: true,
          ),
        ),
      ),
    );

    final bubble = tester.widget<MessageBubble>(find.byType(MessageBubble));
    expect(bubble.isStreaming, isFalse);
  });
}
