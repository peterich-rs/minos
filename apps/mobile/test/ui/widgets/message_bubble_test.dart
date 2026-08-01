import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:minos/ui/features/chat/widgets/message_bubble.dart';
import 'package:minos/ui/theme/theme.dart';

void main() {
  Widget wrap(Widget child) {
    return MaterialApp(
      theme: MinosTheme.light(),
      home: Scaffold(body: child),
    );
  }

  testWidgets('user bubble renders markdown content', (tester) async {
    await tester.pumpWidget(
      wrap(
        const MessageBubble(isUser: true, markdownContent: 'Hello from phone'),
      ),
    );

    expect(find.text('Hello from phone'), findsOneWidget);
  });

  testWidgets('assistant bubble shows streaming cursor', (tester) async {
    await tester.pumpWidget(
      wrap(
        const MessageBubble(
          isUser: false,
          markdownContent: 'Thinking…',
          isStreaming: true,
        ),
      ),
    );

    expect(find.text('Thinking…'), findsOneWidget);
    expect(
      find.byKey(const ValueKey<String>('streaming-cursor')),
      findsOneWidget,
    );
  });
}
