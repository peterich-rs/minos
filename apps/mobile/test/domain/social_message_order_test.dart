import 'package:flutter_test/flutter_test.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/domain/social_message_order.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  MessageSender sender() => const MessageSender.account(
    accountId: 'a',
    minosId: 'm',
    displayName: 'n',
  );

  SocialChatMessage msg({
    required String id,
    int? serverOrderKey,
    int createdAtMs = 0,
    int clientSeq = 0,
    SocialMessageDeliveryState delivery = SocialMessageDeliveryState.sent,
  }) {
    return SocialChatMessage(
      localId: id,
      conversationId: 'c1',
      sender: sender(),
      text: id,
      createdAtMs: createdAtMs,
      clientSeq: clientSeq,
      deliveryState: delivery,
      serverOrderKey: serverOrderKey,
      serverMessageId: serverOrderKey != null ? id : null,
    );
  }

  group('compareSocialChatMessages', () {
    test('orders durable rows by serverOrderKey only', () {
      final a = msg(id: 'a', serverOrderKey: 10, createdAtMs: 999);
      final b = msg(id: 'b', serverOrderKey: 5, createdAtMs: 1);
      final sorted = sortSocialChatMessages([a, b]);
      expect(sorted.map((m) => m.localId).toList(), <String>['b', 'a']);
    });

    test('optimistic without seq sorts after durable', () {
      final durable = msg(id: 'd', serverOrderKey: 2, createdAtMs: 100);
      final pending = msg(
        id: 'p',
        createdAtMs: 1,
        delivery: SocialMessageDeliveryState.sending,
      );
      final sorted = sortSocialChatMessages([pending, durable]);
      expect(sorted.map((m) => m.localId).toList(), <String>['d', 'p']);
    });

    test('does not treat createdAtMs as a pseudo seq', () {
      // Durable seq 2 at old time vs durable seq 1 at new time → still seq ASC.
      final newerTimeOlderSeq = msg(
        id: '1',
        serverOrderKey: 1,
        createdAtMs: 9000,
      );
      final olderTimeNewerSeq = msg(
        id: '2',
        serverOrderKey: 2,
        createdAtMs: 100,
      );
      final sorted = sortSocialChatMessages([
        olderTimeNewerSeq,
        newerTimeOlderSeq,
      ]);
      expect(sorted.map((m) => m.localId).toList(), <String>['1', '2']);
    });
  });

  group('min/maxLoadedSeqOf', () {
    test('ignores optimistic rows without seq', () {
      final messages = [
        msg(id: 'p', delivery: SocialMessageDeliveryState.sending),
        msg(id: 'a', serverOrderKey: 3),
        msg(id: 'b', serverOrderKey: 9),
      ];
      expect(minLoadedSeqOf(messages), 3);
      expect(maxLoadedSeqOf(messages), 9);
    });
  });
}
