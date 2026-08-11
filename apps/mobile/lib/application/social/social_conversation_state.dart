import 'package:freezed_annotation/freezed_annotation.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/domain/social_message_order.dart';

part 'social_conversation_state.freezed.dart';

/// Timeline window + meta for one conversation (TimelineSync).
///
/// Immutable freezed state so Riverpod equality / copyWith stay correct under
/// complex concurrent updates (load older, snapshot, inbound, outbox).
@freezed
abstract class SocialConversationState with _$SocialConversationState {
  const SocialConversationState._();

  const factory SocialConversationState({
    String? myAccountId,
    @Default(<SocialChatMessage>[]) List<SocialChatMessage> messages,
    int? minLoadedSeq,
    int? maxLoadedSeq,
    @Default(false) bool hasOlder,
    @Default(false) bool loadingOlder,
    @Default(true) bool isLoading,
    Object? error,
  }) = _SocialConversationState;

  /// Cold-open state before the first hydrate completes.
  factory SocialConversationState.initial() => const SocialConversationState();

  /// Replace messages and recompute seq window from durable rows only.
  SocialConversationState withMessages(List<SocialChatMessage> next) {
    return copyWith(
      messages: next,
      minLoadedSeq: minLoadedSeqOf(next),
      maxLoadedSeq: maxLoadedSeqOf(next),
    );
  }
}
