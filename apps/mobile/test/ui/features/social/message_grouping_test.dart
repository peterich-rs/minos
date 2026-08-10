import 'package:flutter_test/flutter_test.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/features/social/lib/message_grouping.dart';

MessageSender _sender(String id, {SenderType type = SenderType.user}) {
  if (type == SenderType.agent) {
    return MessageSender.bot(botId: id, displayName: id, runtimeAgent: 'codex');
  }
  return MessageSender.account(accountId: id, minosId: id, displayName: id);
}

SocialChatMessage _msg({
  required String id,
  String accountId = 'u1',
  SenderType senderType = SenderType.user,
  int createdAtMs = 0,
  bool recalled = false,
  String? serverMessageId,
  String text = 'hi',
}) {
  return SocialChatMessage(
    localId: id,
    conversationId: 'c1',
    sender: _sender(accountId, type: senderType),
    text: text,
    createdAtMs: createdAtMs,
    clientSeq: 1,
    deliveryState: SocialMessageDeliveryState.sent,
    senderType: senderType,
    serverMessageId: serverMessageId ?? id,
    recalledAtMs: recalled ? createdAtMs : null,
  );
}

void main() {
  group('messageAuthorKey', () {
    test('groups users by account id', () {
      expect(messageAuthorKey(_msg(id: 'a', accountId: 'alice')), 'user:alice');
      expect(
        messageAuthorKey(_msg(id: 'b', accountId: 'bob')),
        isNot(messageAuthorKey(_msg(id: 'a', accountId: 'alice'))),
      );
    });

    test('separates agents by session when present', () {
      final a = _msg(
        id: 'a',
        accountId: 'agent-1',
        senderType: SenderType.agent,
        serverMessageId: 'agent-result:conv:s1:turn1',
      );
      final b = _msg(
        id: 'b',
        accountId: 'agent-1',
        senderType: SenderType.agent,
        serverMessageId: 'agent-result:conv:s2:turn1',
      );
      expect(messageAuthorKey(a), 'agent:agent-1:s1');
      expect(messageAuthorKey(a), isNot(messageAuthorKey(b)));
    });

    test('returns null for recalled', () {
      expect(
        messageAuthorKey(_msg(id: 'r', recalled: true, createdAtMs: 100)),
        isNull,
      );
    });
  });

  group('isMessageGroupContinuation', () {
    const t0 = 1700000000000;

    test('is false without prev', () {
      expect(isMessageGroupContinuation(null, _msg(id: 'a')), isFalse);
    });

    test('collapses consecutive same-author within window', () {
      final prev = _msg(id: 'a', createdAtMs: t0);
      final curr = _msg(id: 'b', createdAtMs: t0 + messageGroupWindowMs - 1);
      expect(isMessageGroupContinuation(prev, curr), isTrue);
    });

    test('breaks after the time window', () {
      final prev = _msg(id: 'a', createdAtMs: t0);
      final curr = _msg(id: 'b', createdAtMs: t0 + messageGroupWindowMs + 1);
      expect(isMessageGroupContinuation(prev, curr), isFalse);
    });

    test('does not group different authors', () {
      final prev = _msg(id: 'a', accountId: 'alice', createdAtMs: t0);
      final curr = _msg(id: 'b', accountId: 'bob', createdAtMs: t0 + 1000);
      expect(isMessageGroupContinuation(prev, curr), isFalse);
    });

    test('groups without timestamps when author matches', () {
      final prev = _msg(id: 'a');
      final curr = _msg(id: 'b');
      expect(isMessageGroupContinuation(prev, curr), isTrue);
    });

    test('does not continue after recalled row', () {
      final prev = _msg(id: 'a', createdAtMs: t0, recalled: true);
      final curr = _msg(id: 'b', createdAtMs: t0 + 1000);
      expect(isMessageGroupContinuation(prev, curr), isFalse);
    });
  });

  group('day dividers', () {
    test('localDayKey formats local calendar day', () {
      final d = DateTime(2024, 6, 3, 15, 0, 0);
      expect(localDayKey(d.millisecondsSinceEpoch), '2024-06-03');
      expect(localDayKey(null), isNull);
      expect(localDayKey(0), isNull);
    });

    test('shouldShowDayDivider on first message with timestamp', () {
      final curr = _msg(
        id: 'a',
        createdAtMs: DateTime.now().millisecondsSinceEpoch,
      );
      expect(shouldShowDayDivider(null, curr), isTrue);
    });

    test('shouldShowDayDivider false same day', () {
      final t = DateTime(2024, 1, 10, 9, 0).millisecondsSinceEpoch;
      final prev = _msg(id: 'a', createdAtMs: t);
      final curr = _msg(
        id: 'b',
        createdAtMs: DateTime(2024, 1, 10, 18, 0).millisecondsSinceEpoch,
      );
      expect(shouldShowDayDivider(prev, curr), isFalse);
    });

    test('shouldShowDayDivider true across days', () {
      final prev = _msg(
        id: 'a',
        createdAtMs: DateTime(2024, 1, 10, 23, 0).millisecondsSinceEpoch,
      );
      final curr = _msg(
        id: 'b',
        createdAtMs: DateTime(2024, 1, 11, 1, 0).millisecondsSinceEpoch,
      );
      expect(shouldShowDayDivider(prev, curr), isTrue);
    });

    test('formatDayDividerLabel returns 今天 for now', () {
      final now = DateTime(2024, 5, 1, 12, 0);
      expect(formatDayDividerLabel(now.millisecondsSinceEpoch, now: now), '今天');
    });

    test('formatDayDividerLabel returns 昨天', () {
      final now = DateTime(2024, 5, 2, 12, 0);
      final y = DateTime(2024, 5, 1, 18, 0);
      expect(formatDayDividerLabel(y.millisecondsSinceEpoch, now: now), '昨天');
    });
  });

  group('clock', () {
    test('formatMessageClock pads hours and minutes', () {
      final ms = DateTime(2024, 1, 1, 9, 5).millisecondsSinceEpoch;
      expect(formatMessageClock(ms), '09:05');
    });
  });
}
