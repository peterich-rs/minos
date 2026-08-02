import 'package:minos/src/rust/api/minos.dart';

enum SocialMessageDeliveryState { sending, sent, failed }

class SocialChatMessage {
  const SocialChatMessage({
    required this.localId,
    required this.conversationId,
    required this.sender,
    required this.text,
    required this.createdAtMs,
    required this.clientSeq,
    required this.deliveryState,
    this.senderType = SenderType.user,
    this.serverMessageId,
    this.serverOrderKey,
    this.replyTo,
    this.recalledAtMs,
    this.mentionedAccountIds = const <String>[],
  });

  final String localId;
  final String conversationId;
  final UserSummary sender;
  final String text;
  final int createdAtMs;
  final int clientSeq;
  final SocialMessageDeliveryState deliveryState;
  final SenderType senderType;
  final String? serverMessageId;
  final int? serverOrderKey;
  final ChatMessageReplySummary? replyTo;
  final int? recalledAtMs;
  final List<String> mentionedAccountIds;

  bool get isRecalled => recalledAtMs != null;

  bool get canReply =>
      deliveryState == SocialMessageDeliveryState.sent &&
      !isRecalled &&
      serverMessageId != null;

  bool get canRecall =>
      deliveryState == SocialMessageDeliveryState.sent &&
      !isRecalled &&
      serverMessageId != null;

  /// Best-effort session id for agent bubbles.
  ///
  /// Dual-written Desktop results use
  /// `agent-result:{conversationId}:{sessionId}:{turnId}`. Hub-native agent
  /// messages may lack this shape — callers should fall back to session list.
  String? get agentSessionIdFromMessageId {
    final id = (serverMessageId ?? localId).trim();
    if (!id.startsWith('agent-result:')) return null;
    final parts = id.split(':');
    // agent-result + conversation + session + durable (+ optional extra)
    if (parts.length < 4) return null;
    final sessionId = parts[2].trim();
    return sessionId.isEmpty ? null : sessionId;
  }

  SocialChatMessage copyWith({
    String? localId,
    String? conversationId,
    UserSummary? sender,
    String? text,
    int? createdAtMs,
    int? clientSeq,
    SocialMessageDeliveryState? deliveryState,
    SenderType? senderType,
    Object? serverMessageId = _socialMessageUnset,
    Object? serverOrderKey = _socialMessageUnset,
    Object? replyTo = _socialMessageUnset,
    Object? recalledAtMs = _socialMessageUnset,
    List<String>? mentionedAccountIds,
  }) {
    return SocialChatMessage(
      localId: localId ?? this.localId,
      conversationId: conversationId ?? this.conversationId,
      sender: sender ?? this.sender,
      text: text ?? this.text,
      createdAtMs: createdAtMs ?? this.createdAtMs,
      clientSeq: clientSeq ?? this.clientSeq,
      deliveryState: deliveryState ?? this.deliveryState,
      senderType: senderType ?? this.senderType,
      serverMessageId: identical(serverMessageId, _socialMessageUnset)
          ? this.serverMessageId
          : serverMessageId as String?,
      serverOrderKey: identical(serverOrderKey, _socialMessageUnset)
          ? this.serverOrderKey
          : serverOrderKey as int?,
      replyTo: identical(replyTo, _socialMessageUnset)
          ? this.replyTo
          : replyTo as ChatMessageReplySummary?,
      recalledAtMs: identical(recalledAtMs, _socialMessageUnset)
          ? this.recalledAtMs
          : recalledAtMs as int?,
      mentionedAccountIds: mentionedAccountIds ?? this.mentionedAccountIds,
    );
  }
}

const Object _socialMessageUnset = Object();
