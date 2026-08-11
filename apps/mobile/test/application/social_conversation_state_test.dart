import 'package:flutter_test/flutter_test.dart';
import 'package:minos/application/social/social_conversation_state.dart';
import 'package:minos/domain/social_message.dart';
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
    SocialMessageDeliveryState delivery = SocialMessageDeliveryState.sent,
  }) {
    return SocialChatMessage(
      localId: id,
      conversationId: 'c1',
      sender: sender(),
      text: id,
      createdAtMs: 0,
      clientSeq: 0,
      deliveryState: delivery,
      serverOrderKey: serverOrderKey,
      serverMessageId: serverOrderKey != null ? id : null,
    );
  }

  group('SocialConversationState', () {
    test('initial is loading with empty timeline', () {
      final state = SocialConversationState.initial();
      expect(state.isLoading, isTrue);
      expect(state.messages, isEmpty);
      expect(state.error, isNull);
      expect(state.minLoadedSeq, isNull);
      expect(state.maxLoadedSeq, isNull);
      expect(state.hasOlder, isFalse);
      expect(state.loadingOlder, isFalse);
    });

    test('withMessages recomputes durable seq window only', () {
      final state = SocialConversationState.initial().withMessages([
        msg(id: 'p', delivery: SocialMessageDeliveryState.sending),
        msg(id: 'a', serverOrderKey: 3),
        msg(id: 'b', serverOrderKey: 9),
      ]);

      expect(state.messages.map((m) => m.localId).toList(), <String>[
        'p',
        'a',
        'b',
      ]);
      expect(state.minLoadedSeq, 3);
      expect(state.maxLoadedSeq, 9);
    });

    test('copyWith can clear error without clobbering messages', () {
      final loaded = SocialConversationState(
        myAccountId: 'acc',
        messages: [msg(id: 'a', serverOrderKey: 1)],
        minLoadedSeq: 1,
        maxLoadedSeq: 1,
        isLoading: false,
        error: 'boom',
      );

      final cleared = loaded.copyWith(error: null);
      expect(cleared.error, isNull);
      expect(cleared.messages, hasLength(1));
      expect(cleared.myAccountId, 'acc');
      expect(cleared.isLoading, isFalse);
    });

    test('freezed equality covers meta fields (messages use identity)', () {
      // SocialChatMessage is not value-equal, so two withMessages builds are
      // not ==. Meta fields still compare via freezed.
      const base = SocialConversationState(
        myAccountId: 'acc',
        minLoadedSeq: 1,
        maxLoadedSeq: 2,
        hasOlder: true,
        isLoading: false,
      );
      final same = base.copyWith();
      final different = base.copyWith(hasOlder: false);
      expect(same, equals(base));
      expect(same.hashCode, equals(base.hashCode));
      expect(different, isNot(equals(base)));
    });
  });
}
