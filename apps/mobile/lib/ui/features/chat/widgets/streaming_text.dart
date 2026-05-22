import 'package:flutter/material.dart';

import 'message_bubble.dart';

/// Renders an in-flight assistant message as a [MessageBubble] whose
/// markdown content grows in place as `accumulatedText` accumulates.
///
/// The widget itself is dumb — it owns no event-listening logic. The
/// parent (e.g. `ThreadViewPage`) is responsible for grouping
/// `UiEventMessage_TextDelta`s by `messageId` and concatenating them
/// before passing them in, and for flipping `isComplete=true` when the
/// matching `UiEventMessage_MessageCompleted` arrives.
///
/// When [accumulatedText] is empty, a "thinking…" placeholder is shown
/// so the bubble has visible content while the first delta is in
/// flight; the [isComplete] flag still controls whether the streaming
/// cursor renders.
class StreamingText extends StatelessWidget {
  const StreamingText({
    super.key,
    required this.messageId,
    required this.accumulatedText,
    required this.showCursor,
    this.statusLines = const <MessageBubbleStatusLine>[],
  });

  /// The `message_id` from the bridging `UiEventMessage`s. Carried for
  /// debugging / future per-message actions; the widget itself doesn't
  /// rely on it for rendering.
  final String messageId;

  /// Concatenated text from every `UiEventMessage_TextDelta` for this
  /// message, in arrival order.
  final String accumulatedText;

  /// True while this message owns the live cursor.
  final bool showCursor;

  /// Compact activity rows rendered below the message body while the turn is
  /// still active.
  final List<MessageBubbleStatusLine> statusLines;

  @override
  Widget build(BuildContext context) {
    final hasText = accumulatedText.isNotEmpty;
    return MessageBubble(
      isUser: false,
      markdownContent: hasText ? accumulatedText : '_处理中…_',
      isStreaming: showCursor,
      statusLines: statusLines,
    );
  }
}
