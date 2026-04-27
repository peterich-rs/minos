import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/domain/active_session.dart';
import 'package:minos/presentation/widgets/chat/input_bar.dart';
import 'package:minos/src/rust/api/minos.dart' show AgentName;

Widget _wrap(Widget child) {
  return ShadApp(
    home: Scaffold(body: child),
  );
}

void main() {
  testWidgets('Send disabled when text is empty (Idle session)', (
    tester,
  ) async {
    var sent = false;
    await tester.pumpWidget(
      _wrap(
        InputBar(
          session: const SessionIdle(),
          onSend: (_) => sent = true,
          onStop: () {},
        ),
      ),
    );

    expect(find.text('Send'), findsOneWidget);
    expect(find.text('Stop'), findsNothing);

    final btn = tester.widget<ShadButton>(
      find.ancestor(of: find.text('Send'), matching: find.byType(ShadButton)),
    );
    expect(btn.enabled, isFalse);
    expect(btn.onPressed, isNull);
    expect(sent, isFalse);
  });

  testWidgets(
    'Send becomes enabled with non-empty trimmed text in Idle session',
    (tester) async {
      String? captured;
      await tester.pumpWidget(
        _wrap(
          InputBar(
            session: const SessionIdle(),
            onSend: (t) => captured = t,
            onStop: () {},
          ),
        ),
      );

      await tester.enterText(find.byType(EditableText),'hi');
      await tester.pump();

      final btn = tester.widget<ShadButton>(
        find.ancestor(of: find.text('Send'), matching: find.byType(ShadButton)),
      );
      expect(btn.enabled, isTrue);

      await tester.tap(find.text('Send'));
      await tester.pump();
      expect(captured, 'hi');
    },
  );

  testWidgets('Streaming session shows Stop instead of Send', (tester) async {
    var stopped = false;
    await tester.pumpWidget(
      _wrap(
        InputBar(
          session: const SessionStreaming(
            threadId: 't1',
            agent: AgentName.codex,
          ),
          onSend: (_) {},
          onStop: () => stopped = true,
        ),
      ),
    );

    expect(find.text('Stop'), findsOneWidget);
    expect(find.text('Send'), findsNothing);

    await tester.tap(find.text('Stop'));
    await tester.pump();
    expect(stopped, isTrue);
  });

  testWidgets('Starting session also shows Stop', (tester) async {
    await tester.pumpWidget(
      _wrap(
        InputBar(
          session: const SessionStarting(
            agent: AgentName.codex,
            prompt: 'hi',
          ),
          onSend: (_) {},
          onStop: () {},
        ),
      ),
    );
    expect(find.text('Stop'), findsOneWidget);
  });

  testWidgets('Send disabled when text exceeds 8000 chars', (tester) async {
    await tester.pumpWidget(
      _wrap(
        InputBar(
          session: const SessionIdle(),
          onSend: (_) {},
          onStop: () {},
        ),
      ),
    );

    final huge = 'a' * 8001;
    await tester.enterText(find.byType(EditableText),huge);
    await tester.pump();

    final btn = tester.widget<ShadButton>(
      find.ancestor(of: find.text('Send'), matching: find.byType(ShadButton)),
    );
    expect(btn.enabled, isFalse);
  });

  testWidgets('AwaitingInput allows Send', (tester) async {
    await tester.pumpWidget(
      _wrap(
        InputBar(
          session: const SessionAwaitingInput(
            threadId: 't1',
            agent: AgentName.codex,
          ),
          onSend: (_) {},
          onStop: () {},
        ),
      ),
    );

    await tester.enterText(find.byType(EditableText),'follow up');
    await tester.pump();

    final btn = tester.widget<ShadButton>(
      find.ancestor(of: find.text('Send'), matching: find.byType(ShadButton)),
    );
    expect(btn.enabled, isTrue);
  });
}
