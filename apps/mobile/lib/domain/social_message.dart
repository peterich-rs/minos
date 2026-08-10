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

    /// Stable idempotency key for Hub insert (= wire client_message_id).
    /// For pending/sending rows this equals [localId].
    this.clientMessageId,
    this.serverOrderKey,
    this.replyTo,
    this.recalledAtMs,
    this.mentionedAccountIds = const <String>[],
    this.mentionedAgentIds = const <String>[],
    this.reactions = const <ReactionGroup>[],
  });

  final String localId;
  final String conversationId;

  /// First-class author card (Account | Bot). Never stuff bot id into account_id.
  final MessageSender sender;
  final String text;
  final int createdAtMs;
  final int clientSeq;
  final SocialMessageDeliveryState deliveryState;
  final SenderType senderType;
  final String? serverMessageId;
  final String? clientMessageId;
  final int? serverOrderKey;
  final ChatMessageReplySummary? replyTo;
  final int? recalledAtMs;
  final List<String> mentionedAccountIds;

  /// Structured bot mentions (`target_kind=agent`) from Hub wire.
  /// Survives hydrate/reload via SQLite cache — not derived from body text.
  final List<String> mentionedAgentIds;

  /// Hub reaction aggregates (viewer-resolved when available).
  final List<ReactionGroup> reactions;

  /// Idempotency key used on send/retry (never invent a new one on retry).
  String get wireClientMessageId =>
      (clientMessageId ?? serverMessageId ?? localId).trim();

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
    MessageSender? sender,
    String? text,
    int? createdAtMs,
    int? clientSeq,
    SocialMessageDeliveryState? deliveryState,
    SenderType? senderType,
    Object? serverMessageId = _socialMessageUnset,
    Object? clientMessageId = _socialMessageUnset,
    Object? serverOrderKey = _socialMessageUnset,
    Object? replyTo = _socialMessageUnset,
    Object? recalledAtMs = _socialMessageUnset,
    List<String>? mentionedAccountIds,
    List<String>? mentionedAgentIds,
    List<ReactionGroup>? reactions,
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
      clientMessageId: identical(clientMessageId, _socialMessageUnset)
          ? this.clientMessageId
          : clientMessageId as String?,
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
      mentionedAgentIds: mentionedAgentIds ?? this.mentionedAgentIds,
      reactions: reactions ?? this.reactions,
    );
  }
}

const Object _socialMessageUnset = Object();
