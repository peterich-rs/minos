import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  MessageSender sender() => const MessageSender.account(
        accountId: 'a1',
        minosId: 'alice',
        displayName: 'Alice',
      );

  SocialChatMessage base({
    List<String> mentionedAccountIds = const <String>[],
    List<String> mentionedAgentIds = const <String>[],
  }) {
    return SocialChatMessage(
      localId: 'm1',
      conversationId: 'c1',
      sender: sender(),
      text: 'hi @bot',
      createdAtMs: 1,
      clientSeq: 1,
      deliveryState: SocialMessageDeliveryState.sent,
      serverMessageId: 'srv-1',
      mentionedAccountIds: mentionedAccountIds,
      mentionedAgentIds: mentionedAgentIds,
    );
  }

  group('SocialChatMessage agent mentions', () {
    test('defaults mentionedAgentIds to empty', () {
      final message = base();
      expect(message.mentionedAgentIds, isEmpty);
      expect(message.mentionedAccountIds, isEmpty);
    });

    test('preserves structured agent mentions on construction', () {
      final message = base(
        mentionedAccountIds: const <String>['a1'],
        mentionedAgentIds: const <String>['agent-codex', 'agent-claude'],
      );
      expect(message.mentionedAccountIds, <String>['a1']);
      expect(message.mentionedAgentIds, <String>['agent-codex', 'agent-claude']);
    });

    test('copyWith updates mentionedAgentIds without dropping accounts', () {
      final original = base(
        mentionedAccountIds: const <String>['a1'],
        mentionedAgentIds: const <String>['agent-1'],
      );
      final next = original.copyWith(
        mentionedAgentIds: const <String>['agent-2', 'agent-3'],
      );
      expect(next.mentionedAccountIds, <String>['a1']);
      expect(next.mentionedAgentIds, <String>['agent-2', 'agent-3']);
      expect(original.mentionedAgentIds, <String>['agent-1']);
    });

    test('json list round-trip matches cache column encoding', () {
      // Mirrors social_cache_store mentioned_*_ids_json encode/decode.
      final message = base(
        mentionedAccountIds: const <String>['acct-1'],
        mentionedAgentIds: const <String>['agent-1', 'agent-2'],
      );
      final accountJson = jsonEncode(message.mentionedAccountIds);
      final agentJson = jsonEncode(message.mentionedAgentIds);
      final accounts =
          (jsonDecode(accountJson) as List<dynamic>)
              .map((value) => value as String)
              .toList(growable: false);
      final agents =
          (jsonDecode(agentJson) as List<dynamic>)
              .map((value) => value as String)
              .toList(growable: false);
      final restored = message.copyWith(
        mentionedAccountIds: accounts,
        mentionedAgentIds: agents,
      );
      expect(restored.mentionedAccountIds, message.mentionedAccountIds);
      expect(restored.mentionedAgentIds, message.mentionedAgentIds);
    });
  });
}
