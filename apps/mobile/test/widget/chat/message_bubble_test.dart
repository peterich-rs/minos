import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/presentation/widgets/chat/message_bubble.dart';

void main() {
  testWidgets('streaming cursor renders FadeTransition when isStreaming', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: MessageBubble(
            isUser: false,
            markdownContent: 'Hello world',
            isStreaming: true,
          ),
        ),
      ),
    );

    final cursor = find.descendant(
      of: find.byType(MessageBubble),
      matching: find.byType(FadeTransition),
    );
    expect(cursor, findsOneWidget);
  });

  testWidgets('streaming cursor absent when isStreaming=false', (tester) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: MessageBubble(
            isUser: false,
            markdownContent: 'Done.',
            isStreaming: false,
          ),
        ),
      ),
    );

    final cursor = find.descendant(
      of: find.byType(MessageBubble),
      matching: find.byType(FadeTransition),
    );
    expect(cursor, findsNothing);
  });

  testWidgets('user bubble aligns to centerRight', (tester) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: MessageBubble(
            isUser: true,
            markdownContent: 'Hi',
          ),
        ),
      ),
    );

    final align = tester.widget<Align>(find.byType(Align).first);
    expect(align.alignment, Alignment.centerRight);
  });

  testWidgets('assistant bubble aligns to centerLeft', (tester) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: MessageBubble(
            isUser: false,
            markdownContent: 'Reply',
          ),
        ),
      ),
    );

    final align = tester.widget<Align>(find.byType(Align).first);
    expect(align.alignment, Alignment.centerLeft);
  });
}
