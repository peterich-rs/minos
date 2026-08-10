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
    });
  });
}
