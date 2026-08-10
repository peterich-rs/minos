import 'package:flutter_test/flutter_test.dart';
import 'package:minos/domain/mention_extract.dart';

void main() {
  group('extractOptimisticMentions', () {
    test('preserves agent appearance order against roster', () {
      final mentions = extractOptimisticMentions(
        text: '@claude @codex count off',
        selfAccountId: 'me',
        humans: const [],
        agents: const [
          MentionAgentRef(
            agentId: 'id-codex',
            runtimeAgent: 'codex',
            name: 'Codex',
          ),
          MentionAgentRef(
            agentId: 'id-claude',
            runtimeAgent: 'claude',
            name: 'Claude',
          ),
        ],
      );
      expect(mentions.agentIds, <String>['id-claude', 'id-codex']);
      expect(mentions.structuredMentions, <Map<String, Object?>>[
        <String, Object?>{
          'kind': 'bot',
          'bot_id': 'id-claude',
          'start': 0,
          'length': 7, // @claude
        },
        <String, Object?>{
          'kind': 'bot',
          'bot_id': 'id-codex',
          'start': 8,
          'length': 6, // @codex
        },
      ]);
    });

    test('skips self human and unknown tokens', () {
      final mentions = extractOptimisticMentions(
        text: '@alice @bob @gemini hi',
        selfAccountId: 'a1',
        humans: const [
          MentionHumanRef(accountId: 'a1', minosId: 'alice'),
          MentionHumanRef(accountId: 'b1', minosId: 'bob'),
        ],
        agents: const [
          MentionAgentRef(
            agentId: 'id-codex',
            runtimeAgent: 'codex',
            name: 'Codex',
          ),
        ],
      );
      expect(mentions.accountIds, <String>['b1']);
      expect(mentions.agentIds, isEmpty);
      expect(mentions.structuredMentions, <Map<String, Object?>>[
        <String, Object?>{
          'kind': 'account',
          'account_id': 'b1',
          'start': 7,
          'length': 4, // @bob
        },
      ]);
    });

    test('collectMentionTokenSpans covers @token with start/length', () {
      final spans = collectMentionTokenSpans('@bob hi @claude');
      expect(spans.length, 2);
      expect(spans[0].token, 'bob');
      expect(spans[0].start, 0);
      expect(spans[0].length, 4);
      expect(spans[1].token, 'claude');
      expect(spans[1].start, 8);
      expect(spans[1].length, 7);
    });
  });
}
