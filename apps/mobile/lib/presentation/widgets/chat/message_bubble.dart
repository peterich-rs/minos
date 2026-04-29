import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';

enum MessageDeliveryState { none, sending, failed }

/// One chat bubble. iMessage-style:
///   - user (right): iOS systemBlue, white text
///   - assistant (left): subtle gray surface, theme on-surface text
///
/// Bubbles use an asymmetric corner radius (small corner on the speaker
/// side) so the row reads as belonging to the avatar rail. When
/// [isStreaming] is true a small blinking cursor renders below the body.
class MessageBubble extends StatelessWidget {
  const MessageBubble({
    super.key,
    required this.isUser,
    required this.markdownContent,
    this.isStreaming = false,
    this.deliveryState = MessageDeliveryState.none,
  });

  final bool isUser;
  final String markdownContent;
  final bool isStreaming;
  final MessageDeliveryState deliveryState;

  static const Color _userBg = Color(0xFF007AFF);
  static const Color _userFg = Colors.white;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final isDark = theme.brightness == Brightness.dark;

    final bg = isUser
        ? _userBg
        : (isDark ? const Color(0xFF2C2C2E) : const Color(0xFFEDEDEF));
    final fg = isUser ? _userFg : scheme.onSurface;

    final radius = BorderRadius.only(
      topLeft: const Radius.circular(18),
      topRight: const Radius.circular(18),
      bottomLeft: Radius.circular(isUser ? 18 : 4),
      bottomRight: Radius.circular(isUser ? 4 : 18),
    );

    return Align(
      alignment: isUser ? Alignment.centerRight : Alignment.centerLeft,
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: MediaQuery.of(context).size.width * 0.78,
        ),
        child: Container(
          margin: EdgeInsets.fromLTRB(isUser ? 48 : 12, 3, isUser ? 12 : 48, 3),
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
          decoration: BoxDecoration(color: bg, borderRadius: radius),
          child: Column(
            crossAxisAlignment: isUser
                ? CrossAxisAlignment.end
                : CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              MarkdownBody(
                data: markdownContent,
                selectable: true,
                styleSheet: MarkdownStyleSheet.fromTheme(theme).copyWith(
                  p: theme.textTheme.bodyMedium?.copyWith(
                    color: fg,
                    height: 1.4,
                  ),
                  code: theme.textTheme.bodySmall?.copyWith(
                    fontFamily: 'monospace',
                    color: fg,
                    backgroundColor: isUser
                        ? Colors.white.withValues(alpha: 0.18)
                        : (isDark
                              ? const Color(0xFF1C1C1E)
                              : const Color(0xFFE0E0E5)),
                  ),
                  a: TextStyle(
                    color: isUser ? Colors.white : scheme.primary,
                    decoration: TextDecoration.underline,
                  ),
                ),
              ),
              if (isStreaming)
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: _StreamingCursor(color: fg),
                ),
              if (isUser && deliveryState != MessageDeliveryState.none)
                Padding(
                  padding: const EdgeInsets.only(top: 6),
                  child: _DeliveryStateIndicator(
                    state: deliveryState,
                    color: _userFg.withValues(alpha: 0.9),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _DeliveryStateIndicator extends StatelessWidget {
  const _DeliveryStateIndicator({required this.state, required this.color});

  final MessageDeliveryState state;
  final Color color;

  @override
  Widget build(BuildContext context) {
    return switch (state) {
      MessageDeliveryState.sending => SizedBox(
        width: 14,
        height: 14,
        child: CircularProgressIndicator(
          strokeWidth: 1.8,
          valueColor: AlwaysStoppedAnimation<Color>(color),
        ),
      ),
      MessageDeliveryState.failed => Icon(
        Icons.error_outline,
        size: 14,
        color: color,
      ),
      MessageDeliveryState.none => const SizedBox.shrink(),
    };
  }
}

class _StreamingCursor extends StatefulWidget {
  const _StreamingCursor({required this.color});
  final Color color;

  @override
  State<_StreamingCursor> createState() => _StreamingCursorState();
}

class _StreamingCursorState extends State<_StreamingCursor>
    with SingleTickerProviderStateMixin {
  late final AnimationController _ctl = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 700),
  )..repeat(reverse: true);

  @override
  void dispose() {
    _ctl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return FadeTransition(
      opacity: _ctl,
      child: Container(
        width: 6,
        height: 12,
        decoration: BoxDecoration(
          color: widget.color,
          borderRadius: BorderRadius.circular(2),
        ),
      ),
    );
  }
}
