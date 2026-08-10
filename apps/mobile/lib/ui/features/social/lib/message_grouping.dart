import 'package:minos/domain/message_sender_ext.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Default continuity window (Slack-like; matches Desktop `MESSAGE_GROUP_WINDOW_MS`).
const int messageGroupWindowMs = 10 * 60 * 1000;

/// Author key for grouping consecutive collaboration rows.
///
/// - Users group by account id (multi-member groups must not collapse across people).
/// - Agents group by bot_id + optional session id.
/// - Recalled rows still keep their author key so neighboring non-recalled
///   messages from the same author can continue grouping around them only when
///   callers decide; [isMessageGroupContinuation] treats recalled as a break
///   so system-style recall chrome does not steal avatar slots awkwardly.
String? messageAuthorKey(SocialChatMessage message) {
  if (message.isRecalled) return null;
  if (message.senderType == SenderType.agent || message.sender.isBot) {
    final session = message.agentSessionIdFromMessageId ?? '';
    final agentId = message.sender.identityId;
    return 'agent:$agentId:$session';
  }
  return message.sender.groupingKey;
}

/// True when [curr] should hide avatar/header as a continuation of [prev].
bool isMessageGroupContinuation(
  SocialChatMessage? prev,
  SocialChatMessage curr, {
  int windowMs = messageGroupWindowMs,
}) {
  if (prev == null) return false;
  final prevKey = messageAuthorKey(prev);
  final currKey = messageAuthorKey(curr);
  if (prevKey == null || currKey == null || prevKey != currKey) {
    return false;
  }

  final prevMs = prev.createdAtMs;
  final currMs = curr.createdAtMs;
  // Without valid timestamps, still collapse consecutive same-author rows.
  if (prevMs <= 0 || currMs <= 0) {
    return true;
  }
  return (currMs - prevMs).abs() <= windowMs;
}

/// Local calendar day key `YYYY-MM-DD`, or null when timestamp missing/invalid.
String? localDayKey(int? ms) {
  if (ms == null || ms <= 0) return null;
  final d = DateTime.fromMillisecondsSinceEpoch(ms, isUtc: false);
  final y = d.year.toString().padLeft(4, '0');
  final m = d.month.toString().padLeft(2, '0');
  final day = d.day.toString().padLeft(2, '0');
  return '$y-$m-$day';
}

/// Whether to insert a day divider before [curr] given the previous message.
bool shouldShowDayDivider(SocialChatMessage? prev, SocialChatMessage curr) {
  final currKey = localDayKey(curr.createdAtMs);
  if (currKey == null) return false;
  if (prev == null) return true;
  final prevKey = localDayKey(prev.createdAtMs);
  if (prevKey == null) return true;
  return prevKey != currKey;
}

/// Human day divider label (localized Chinese, matches Mobile UI language).
String formatDayDividerLabel(int ms, {DateTime? now}) {
  final d = DateTime.fromMillisecondsSinceEpoch(ms, isUtc: false);
  final today = now ?? DateTime.now();
  final yesterday = DateTime(today.year, today.month, today.day - 1);

  if (_isSameDay(d, today)) return '今天';
  if (_isSameDay(d, yesterday)) return '昨天';
  if (d.year == today.year) {
    return '${d.month}月${d.day}日';
  }
  return '${d.year}年${d.month}月${d.day}日';
}

/// Compact clock for message headers (`HH:mm`).
String formatMessageClock(int ms) {
  if (ms <= 0) return '';
  final d = DateTime.fromMillisecondsSinceEpoch(ms, isUtc: false);
  final hh = d.hour.toString().padLeft(2, '0');
  final mm = d.minute.toString().padLeft(2, '0');
  return '$hh:$mm';
}

/// Full locale timestamp for long-press / accessibility.
String formatMessageFullTimestamp(int ms) {
  if (ms <= 0) return '';
  final d = DateTime.fromMillisecondsSinceEpoch(ms, isUtc: false);
  final y = d.year.toString().padLeft(4, '0');
  final m = d.month.toString().padLeft(2, '0');
  final day = d.day.toString().padLeft(2, '0');
  return '$y-$m-$day ${formatMessageClock(ms)}';
}

bool _isSameDay(DateTime lhs, DateTime rhs) {
  return lhs.year == rhs.year && lhs.month == rhs.month && lhs.day == rhs.day;
}
