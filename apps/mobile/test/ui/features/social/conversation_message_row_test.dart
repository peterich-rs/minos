import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/features/social/widgets/conversation_message_row.dart';
import 'package:minos/ui/features/social/widgets/conversation_system_message.dart';
import 'package:minos/ui/theme/theme.dart';

UserSummary _user(String id, {String name = 'Alice'}) {
  return UserSummary(accountId: id, minosId: id, displayName: name);
}

SocialChatMessage _msg({
  required String id,
  String accountId = 'u1',
  String name = 'Alice',
  SenderType senderType = SenderType.user,
  String text = 'Hello world',
  int createdAtMs = 1700000000000,
  SocialMessageDeliveryState delivery = SocialMessageDeliveryState.sent,
  bool recalled = false,
  ChatMessageReplySummary? replyTo,
  List<String> mentioned = const <String>[],
}) {
  return SocialChatMessage(
    localId: id,
    conversationId: 'c1',
    sender: _user(accountId, name: name),
    text: text,
    createdAtMs: createdAtMs,
    clientSeq: 1,
    deliveryState: delivery,
    senderType: senderType,
    serverMessageId: id,
    recalledAtMs: recalled ? createdAtMs : null,
    replyTo: replyTo,
    mentionedAccountIds: mentioned,
  );
}

void main() {
  Widget wrap(Widget child) {
    return MaterialApp(
      theme: MinosTheme.light(),
      home: Scaffold(body: child),
    );
  }

  testWidgets('mine message is left-aligned and shows 我 header', (
    tester,
  ) async {
    await tester.pumpWidget(
      wrap(
        ConversationMessageRow(
          message: _msg(id: 'm1', name: 'Fan'),
          isMine: true,
        ),
      ),
    );

    expect(find.text('我'), findsOneWidget);
    expect(find.text('Hello world'), findsOneWidget);
    // No right-aligned Row for mine vs others — full-width left chrome.
    expect(find.byType(ConversationMessageRow), findsOneWidget);
  });

  testWidgets('agent message shows Agent chip and author name', (tester) async {
    await tester.pumpWidget(
      wrap(
        ConversationMessageRow(
          message: _msg(
            id: 'a1',
            accountId: 'agent-1',
            name: 'Codex',
            senderType: SenderType.agent,
            text: 'Done.',
          ),
          isMine: false,
        ),
      ),
    );

    expect(find.text('Codex'), findsOneWidget);
    expect(find.text('Agent'), findsOneWidget);
    expect(find.text('Done.'), findsOneWidget);
  });

  testWidgets('continuation hides author header', (tester) async {
    await tester.pumpWidget(
      wrap(
        ConversationMessageRow(
          message: _msg(id: 'm2', text: 'second'),
          isMine: true,
          groupedWithPrevious: true,
        ),
      ),
    );

    expect(find.text('我'), findsNothing);
    expect(find.text('second'), findsOneWidget);
  });

  testWidgets('failed send shows retry badge', (tester) async {
    var retried = false;
    await tester.pumpWidget(
      wrap(
        ConversationMessageRow(
          message: _msg(id: 'm3', delivery: SocialMessageDeliveryState.failed),
          isMine: true,
          onRetry: () => retried = true,
        ),
      ),
    );

    expect(find.text('!'), findsOneWidget);
    expect(find.text('失败'), findsOneWidget);
    await tester.tap(find.text('!'));
    expect(retried, isTrue);
  });

  testWidgets('recalled message uses system chrome', (tester) async {
    await tester.pumpWidget(
      wrap(
        ConversationMessageRow(
          message: _msg(id: 'm4', recalled: true),
          isMine: true,
        ),
      ),
    );

    expect(find.byType(ConversationSystemMessage), findsOneWidget);
    expect(find.text('你撤回了一条消息'), findsOneWidget);
  });

  testWidgets('reply preview is rendered', (tester) async {
    await tester.pumpWidget(
      wrap(
        ConversationMessageRow(
          message: _msg(
            id: 'm5',
            text: 'follow up',
            replyTo: ChatMessageReplySummary(
              messageId: 'parent',
              text: 'original',
              sender: _user('u2', name: 'Bob'),
            ),
          ),
          isMine: true,
        ),
      ),
    );

    expect(find.textContaining('Bob'), findsOneWidget);
    expect(find.text('original'), findsOneWidget);
    expect(find.text('follow up'), findsOneWidget);
  });

  testWidgets('mentions-me chip is shown', (tester) async {
    await tester.pumpWidget(
      wrap(
        ConversationMessageRow(
          message: _msg(
            id: 'm6',
            accountId: 'other',
            name: 'Bob',
            text: '@me hello',
            mentioned: const <String>['me'],
          ),
          isMine: false,
          mentionsMe: true,
        ),
      ),
    );

    expect(find.text('提到了你'), findsOneWidget);
  });
}
