import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

enum MessageDeliveryState { none, sending, failed }

enum MessageBubbleStatusTone { neutral, info, success, error }

class MessageBubbleStatusLine {
  const MessageBubbleStatusLine({
    required this.icon,
    required this.label,
    this.tone = MessageBubbleStatusTone.neutral,
  });

  final IconData icon;
  final String label;
  final MessageBubbleStatusTone tone;
}

/// One chat bubble. iMessage-style:
///   - user (right): iOS systemBlue bubble, white text
///   - assistant (left): full-width content rail with optional live status rows
///
/// Bubbles use an asymmetric corner radius (small corner on the speaker
/// side) so the row reads as belonging to the avatar rail. Assistant
/// messages deliberately avoid the narrow bubble treatment so markdown,
/// tool progress, and long-form output read more like a transcript.
class MessageBubble extends StatelessWidget {
  const MessageBubble({
    super.key,
    required this.isUser,
    required this.markdownContent,
    this.isStreaming = false,
    this.deliveryState = MessageDeliveryState.none,
    this.statusLines = const <MessageBubbleStatusLine>[],
  });

  final bool isUser;
  final String markdownContent;
  final bool isStreaming;
  final MessageDeliveryState deliveryState;
  final List<MessageBubbleStatusLine> statusLines;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final shadTheme = ShadTheme.of(context);
    final scheme = theme.colorScheme;

    final bg = isUser
        ? shadTheme.colorScheme.primary
        : shadTheme.colorScheme.card;
    final fg = isUser
        ? shadTheme.colorScheme.primaryForeground
        : shadTheme.colorScheme.foreground;

    final radius = BorderRadius.only(
      topLeft: const Radius.circular(8),
      topRight: const Radius.circular(8),
      bottomLeft: Radius.circular(isUser ? 8 : 3),
      bottomRight: Radius.circular(isUser ? 3 : 8),
    );

    final content = Column(
      crossAxisAlignment: isUser
          ? CrossAxisAlignment.end
          : CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: <Widget>[
        MarkdownBody(
          data: markdownContent,
          selectable: true,
          styleSheet: MarkdownStyleSheet.fromTheme(theme).copyWith(
            p: theme.textTheme.bodyMedium?.copyWith(color: fg, height: 1.48),
            code: theme.textTheme.bodySmall?.copyWith(
              fontFamily: 'monospace',
              color: fg,
              backgroundColor: isUser
                  ? Colors.white.withValues(alpha: 0.18)
                  : shadTheme.colorScheme.muted,
            ),
            a: TextStyle(
              color: isUser ? Colors.white : scheme.primary,
              decoration: TextDecoration.underline,
            ),
          ),
        ),
        if (statusLines.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: _MessageStatusLines(lines: statusLines),
          ),
        if (isStreaming)
          Padding(
            padding: EdgeInsets.only(top: statusLines.isEmpty ? 6 : 8),
            child: _StreamingCursor(color: fg),
          ),
      ],
    );

    final bubble = isUser
        ? ConstrainedBox(
            constraints: BoxConstraints(
              maxWidth: MediaQuery.of(context).size.width * 0.84,
            ),
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
              decoration: BoxDecoration(color: bg, borderRadius: radius),
              child: content,
            ),
          )
        : Container(
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
            decoration: BoxDecoration(color: bg, borderRadius: radius),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Container(
                  width: 26,
                  height: 26,
                  decoration: BoxDecoration(
                    color: shadTheme.colorScheme.secondary,
                    borderRadius: BorderRadius.circular(6),
                  ),
                  alignment: Alignment.center,
                  child: Icon(
                    LucideIcons.bot,
                    size: 15,
                    color: shadTheme.colorScheme.secondaryForeground,
                  ),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Padding(
                    padding: const EdgeInsets.only(top: 1),
                    child: content,
                  ),
                ),
              ],
            ),
          );

    // For a user bubble in a non-`none` delivery state we render the
    // indicator OUTSIDE the bubble, just to its left — the WeChat / iMessage
    // convention. Sandwich is `[indicator] [bubble]` inside a right-aligned
    // Row so the bubble stays flush with the right edge regardless of
    // indicator presence. Assistant bubbles never carry a delivery state.
    final showIndicator = isUser && deliveryState != MessageDeliveryState.none;

    final child = isUser
        ? Row(
            mainAxisAlignment: MainAxisAlignment.end,
            crossAxisAlignment: CrossAxisAlignment.center,
            children: <Widget>[
              if (showIndicator)
                Padding(
                  padding: const EdgeInsets.only(right: 6),
                  child: _DeliveryStateIndicator(
                    state: deliveryState,
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              Flexible(child: bubble),
            ],
          )
        : bubble;

    return Padding(
      padding: EdgeInsets.fromLTRB(isUser ? 48 : 10, 4, isUser ? 12 : 10, 4),
      child: child,
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
        key: const ValueKey<String>('streaming-cursor'),
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

class _MessageStatusLines extends StatelessWidget {
  const _MessageStatusLines({required this.lines});

  final List<MessageBubbleStatusLine> lines;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        for (var index = 0; index < lines.length; index++) ...<Widget>[
          if (index > 0) const SizedBox(height: 4),
          Row(
            children: <Widget>[
              Icon(
                lines[index].icon,
                size: 14,
                color: _statusColor(theme, lines[index].tone),
              ),
              const SizedBox(width: 6),
              Expanded(
                child: AnimatedSwitcher(
                  duration: const Duration(milliseconds: 180),
                  switchInCurve: Curves.easeOut,
                  switchOutCurve: Curves.easeIn,
                  child: Text(
                    lines[index].label,
                    key: ValueKey<String>(lines[index].label),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.labelSmall?.copyWith(
                      color: _statusColor(theme, lines[index].tone),
                      fontWeight: FontWeight.w500,
                      height: 1.2,
                    ),
                  ),
                ),
              ),
            ],
          ),
        ],
      ],
    );
  }

  static Color _statusColor(ThemeData theme, MessageBubbleStatusTone tone) {
    final scheme = theme.colorScheme;
    return switch (tone) {
      MessageBubbleStatusTone.info => scheme.primary,
      MessageBubbleStatusTone.success => const Color(0xFF248A3D),
      MessageBubbleStatusTone.error => scheme.error,
      MessageBubbleStatusTone.neutral => scheme.onSurfaceVariant,
    };
  }
}
