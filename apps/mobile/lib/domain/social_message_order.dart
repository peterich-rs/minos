import 'package:minos/domain/social_message.dart';

/// Timeline order for cached social messages (C3 final / decision 5).
///
/// - Durable rows with [SocialChatMessage.serverOrderKey] (Hub `message_seq`):
///   strict ASC by seq. Never compare seq against `created_at_ms`.
/// - Rows without seq are only optimistic (`sending` / `failed`); they sort
///   after durable peers, then by `clientSeq` / `createdAtMs` among themselves.
int compareSocialChatMessages(SocialChatMessage a, SocialChatMessage b) {
  final aKey = a.serverOrderKey;
  final bKey = b.serverOrderKey;
  final aHas = aKey != null;
  final bHas = bKey != null;

  if (aHas && bHas && aKey != bKey) {
    return aKey.compareTo(bKey);
  }

  final aOpt =
      a.deliveryState == SocialMessageDeliveryState.sending ||
      a.deliveryState == SocialMessageDeliveryState.failed;
  final bOpt =
      b.deliveryState == SocialMessageDeliveryState.sending ||
      b.deliveryState == SocialMessageDeliveryState.failed;

  // Optimistic without seq: after durable peers.
  if (aOpt && !bOpt && !aHas) return 1;
  if (bOpt && !aOpt && !bHas) return -1;

  // One side has seq, the other does not: seq-bearing first.
  if (aHas && !bHas) return -1;
  if (bHas && !aHas) return 1;

  final byTime = a.createdAtMs.compareTo(b.createdAtMs);
  if (byTime != 0) return byTime;

  final byClientSeq = a.clientSeq.compareTo(b.clientSeq);
  if (byClientSeq != 0) return byClientSeq;

  return a.localId.compareTo(b.localId);
}

/// Sort a message list with [compareSocialChatMessages] (stable copy).
List<SocialChatMessage> sortSocialChatMessages(
  Iterable<SocialChatMessage> messages,
) {
  final out = List<SocialChatMessage>.of(messages)
    ..sort(compareSocialChatMessages);
  return out;
}

/// Lowest durable Hub seq in [messages], or null.
int? minLoadedSeqOf(Iterable<SocialChatMessage> messages) {
  int? min;
  for (final m in messages) {
    final key = m.serverOrderKey;
    if (key == null) continue;
    if (min == null || key < min) min = key;
  }
  return min;
}

/// Highest durable Hub seq in [messages], or null.
int? maxLoadedSeqOf(Iterable<SocialChatMessage> messages) {
  int? max;
  for (final m in messages) {
    final key = m.serverOrderKey;
    if (key == null) continue;
    if (max == null || key > max) max = key;
  }
  return max;
}
