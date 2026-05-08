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
    this.serverMessageId,
    this.serverOrderKey,
    this.mentionedAccountIds = const <String>[],
  });

  final String localId;
  final String conversationId;
  final UserSummary sender;
  final String text;
  final int createdAtMs;
  final int clientSeq;
  final SocialMessageDeliveryState deliveryState;
  final String? serverMessageId;
  final int? serverOrderKey;
  final List<String> mentionedAccountIds;

  SocialChatMessage copyWith({
    String? localId,
    String? conversationId,
    UserSummary? sender,
    String? text,
    int? createdAtMs,
    int? clientSeq,
    SocialMessageDeliveryState? deliveryState,
    Object? serverMessageId = _socialMessageUnset,
    Object? serverOrderKey = _socialMessageUnset,
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
      serverMessageId: identical(serverMessageId, _socialMessageUnset)
          ? this.serverMessageId
          : serverMessageId as String?,
      serverOrderKey: identical(serverOrderKey, _socialMessageUnset)
          ? this.serverOrderKey
          : serverOrderKey as int?,
      mentionedAccountIds: mentionedAccountIds ?? this.mentionedAccountIds,
    );
  }
}

const Object _socialMessageUnset = Object();
