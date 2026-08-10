/// Best-effort optimistic mention extract for pending local rows.
///
/// Hub `extract_participant_mentions` remains SSOT after ack. This only seeds
/// cache/UI so reload before HTTP upsert does not drop bot mentions.
library;

/// Collect `@token` name parts from body (alphanumeric / `-` / `_` / optional `#short`).
List<String> collectMentionTokens(String text) {
  final tokens = <String>[];
  final bytes = text.codeUnits;
  var index = 0;
  while (index < bytes.length) {
    if (bytes[index] != 0x40 /* @ */ ) {
      index += 1;
      continue;
    }
    if (index > 0) {
      final prev = bytes[index - 1];
      final isAlnum =
          (prev >= 0x30 && prev <= 0x39) ||
          (prev >= 0x41 && prev <= 0x5a) ||
          (prev >= 0x61 && prev <= 0x7a);
      if (isAlnum) {
        index += 1;
        continue;
      }
    }
    final start = index + 1;
    var end = start;
    while (end < bytes.length) {
      final c = bytes[end];
      final ok =
          (c >= 0x30 && c <= 0x39) ||
          (c >= 0x41 && c <= 0x5a) ||
          (c >= 0x61 && c <= 0x7a) ||
          c == 0x2d /* - */ ||
          c == 0x5f /* _ */ ||
          c == 0x23 /* # */;
      if (!ok) break;
      end += 1;
    }
    if (end > start) {
      tokens.add(text.substring(start, end));
      index = end;
      continue;
    }
    index += 1;
  }
  return tokens;
}

String mentionNamePart(String token) {
  final hash = token.indexOf('#');
  if (hash <= 0) return token;
  return token.substring(0, hash);
}

/// Participant-facing human row for optimistic extract.
class MentionHumanRef {
  const MentionHumanRef({required this.accountId, required this.minosId});

  final String accountId;
  final String minosId;
}

/// Participant-facing agent row for optimistic extract.
class MentionAgentRef {
  const MentionAgentRef({
    required this.agentId,
    required this.runtimeAgent,
    required this.name,
  });

  final String agentId;
  final String runtimeAgent;
  final String name;
}

class OptimisticMentions {
  const OptimisticMentions({
    this.accountIds = const <String>[],
    this.agentIds = const <String>[],
  });

  final List<String> accountIds;
  final List<String> agentIds;
}

/// Resolve body @tokens against current participants (membership-first).
/// Order is body appearance order; self account is skipped.
OptimisticMentions extractOptimisticMentions({
  required String text,
  required String? selfAccountId,
  required List<MentionHumanRef> humans,
  required List<MentionAgentRef> agents,
}) {
  final byMinos = <String, String>{
    for (final h in humans) h.minosId: h.accountId,
  };
  final accountIds = <String>[];
  final agentIds = <String>[];
  final seenAccounts = <String>{};
  final seenAgents = <String>{};
  final self = selfAccountId?.trim() ?? '';

  for (final token in collectMentionTokens(text)) {
    final name = mentionNamePart(token);
    if (name.isEmpty) continue;
    final accountId = byMinos[name];
    if (accountId != null) {
      if (accountId != self && seenAccounts.add(accountId)) {
        accountIds.add(accountId);
      }
      continue;
    }
    final lower = name.toLowerCase();
    MentionAgentRef? match;
    for (final a in agents) {
      if (a.agentId.toLowerCase() == lower ||
          a.runtimeAgent.toLowerCase() == lower ||
          a.name.toLowerCase() == lower) {
        match = a;
        break;
      }
    }
    if (match != null && seenAgents.add(match.agentId)) {
      agentIds.add(match.agentId);
    }
  }

  return OptimisticMentions(accountIds: accountIds, agentIds: agentIds);
}
