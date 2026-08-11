/// Best-effort optimistic mention extract for pending local rows + outbox wire.
///
/// Hub validates structured mentions after ack (body never invents targets).
/// This seeds cache/UI and builds AppendMessage `mentions` payloads so reload
/// before HTTP upsert does not drop bot mentions and delivery has explicit
/// targets.
library;

/// One `@token` occurrence in body text (token without leading `@`).
class MentionTokenSpan {
  const MentionTokenSpan({
    required this.token,
    required this.start,
    required this.length,
  });

  /// Name part after `@` (may include `#short`).
  final String token;

  /// UTF-16 code-unit index of `@` in the body.
  final int start;

  /// UTF-16 code-unit length covering `@` + token.
  final int length;
}

/// Collect `@token` spans from body (alphanumeric / `-` / `_` / optional `#short`).
///
/// [start]/[length] cover the full `@token` (including `@`), matching desktop
/// wire spans for AppendMessage.
List<MentionTokenSpan> collectMentionTokenSpans(String text) {
  final spans = <MentionTokenSpan>[];
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
    final atStart = index;
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
      spans.add(
        MentionTokenSpan(
          token: text.substring(start, end),
          start: atStart,
          length: end - atStart,
        ),
      );
      index = end;
      continue;
    }
    index += 1;
  }
  return spans;
}

/// Collect `@token` name parts from body (without leading `@`).
List<String> collectMentionTokens(String text) {
  return collectMentionTokenSpans(
    text,
  ).map((s) => s.token).toList(growable: false);
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
    this.structuredMentions = const <Map<String, Object?>>[],
  });

  final List<String> accountIds;
  final List<String> agentIds;

  /// Wire [MentionTarget] objects for AppendMessage / outbox payload.
  ///
  /// Shape:
  /// - `{"kind":"bot","bot_id":"...","start":0,"length":6}`
  /// - `{"kind":"account","account_id":"...","start":0,"length":5}`
  final List<Map<String, Object?>> structuredMentions;
}

/// Resolve body @tokens against current participants (membership-first).
/// Order is body appearance order; self account is skipped.
///
/// Produces both id lists (local cache) and structured wire mentions (outbox).
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
  final structured = <Map<String, Object?>>[];
  final seenAccounts = <String>{};
  final seenAgents = <String>{};
  final self = selfAccountId?.trim() ?? '';

  for (final span in collectMentionTokenSpans(text)) {
    final name = mentionNamePart(span.token);
    if (name.isEmpty) continue;
    final accountId = byMinos[name];
    if (accountId != null) {
      if (accountId != self && seenAccounts.add(accountId)) {
        accountIds.add(accountId);
        structured.add(<String, Object?>{
          'kind': 'account',
          'account_id': accountId,
          'start': span.start,
          'length': span.length,
        });
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
      structured.add(<String, Object?>{
        'kind': 'bot',
        'bot_id': match.agentId,
        'start': span.start,
        'length': span.length,
      });
    }
  }

  return OptimisticMentions(
    accountIds: accountIds,
    agentIds: agentIds,
    structuredMentions: structured,
  );
}
