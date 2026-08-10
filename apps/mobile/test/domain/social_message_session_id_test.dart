import 'package:flutter_test/flutter_test.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  group('SocialChatMessage.agentSessionIdFromMessageId', () {
    test('parses agent-result canonical ids', () {
      const message = SocialChatMessage(
        localId: 'local-1',
        conversationId: 'conv-1',
        sender: MessageSender.bot(
          botId: 'agent-1',
          displayName: 'Grok',
          runtimeAgent: 'grok',
          name: 'grok',
        ),
        text: 'hi',
        createdAtMs: 1,
        clientSeq: 1,
        deliveryState: SocialMessageDeliveryState.sent,
        senderType: SenderType.agent,
        serverMessageId: 'agent-result:conv-1:sess-abc:turn-9',
      );
      expect(message.agentSessionIdFromMessageId, 'sess-abc');
    });

    test('returns null for plain hub uuids', () {
      const message = SocialChatMessage(
        localId: 'local-2',
        conversationId: 'conv-1',
        sender: MessageSender.bot(
          botId: 'agent-1',
          displayName: 'Grok',
          runtimeAgent: 'grok',
          name: 'grok',
        ),
        text: 'hi',
        createdAtMs: 1,
        clientSeq: 1,
        deliveryState: SocialMessageDeliveryState.sent,
        senderType: SenderType.agent,
        serverMessageId: 'a1b2c3d4-e5f6-7890-abcd-ef1234567890',
      );
      expect(message.agentSessionIdFromMessageId, isNull);
    });
  });
}
