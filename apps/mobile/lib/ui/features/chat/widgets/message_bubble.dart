import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:minos/ui/theme/theme.dart';

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

/// Chat bubble for the golden-path transcript.
///
/// User messages sit on the right as accent bubbles; assistant messages use a
/// soft full-width transcript rail with a compact bot glyph.
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
    final colors = context.minosColors;

    final bg = isUser ? colors.userBubble : colors.assistantBubble;
    final fg = isUser ? colors.userBubbleForeground : colors.textPrimary;

    final radius = BorderRadius.only(
      topLeft: const Radius.circular(MinosRadii.md),
      topRight: const Radius.circular(MinosRadii.md),
      bottomLeft: Radius.circular(isUser ? MinosRadii.md : MinosRadii.xs),
      bottomRight: Radius.circular(isUser ? MinosRadii.xs : MinosRadii.md),
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
                  : colors.surfaceMuted,
            ),
            a: TextStyle(
              color: isUser ? Colors.white : colors.accent,
              decoration: TextDecoration.underline,
            ),
          ),
        ),
        if (statusLines.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: MinosSpacing.sm),
            child: _MessageStatusLines(lines: statusLines),
          ),
        if (isStreaming)
          Padding(
            padding: EdgeInsets.only(
              top: statusLines.isEmpty ? MinosSpacing.sm : MinosSpacing.sm,
            ),
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
            decoration: BoxDecoration(
              color: bg,
              borderRadius: radius,
              border: Border.all(color: colors.borderSubtle),
            ),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Container(
                  width: 26,
                  height: 26,
                  decoration: BoxDecoration(
                    color: colors.accentSoft,
                    borderRadius: MinosRadii.xsAll,
                  ),
                  alignment: Alignment.center,
                  child: Icon(
                    CupertinoIcons.sparkles,
                    size: 14,
                    color: colors.accent,
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
                    color: colors.textSecondary,
                  ),
                ),
              Flexible(child: bubble),
            ],
          )
        : bubble;

    return Padding(
      padding: EdgeInsets.fromLTRB(isUser ? 48 : 12, 4, isUser ? 12 : 12, 4),
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
    final colors = context.minosColors;
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
                color: _statusColor(colors, lines[index].tone),
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
                      color: _statusColor(colors, lines[index].tone),
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

  static Color _statusColor(MinosColors colors, MessageBubbleStatusTone tone) {
    return switch (tone) {
      MessageBubbleStatusTone.info => colors.accent,
      MessageBubbleStatusTone.success => colors.success,
      MessageBubbleStatusTone.error => colors.danger,
      MessageBubbleStatusTone.neutral => colors.textSecondary,
    };
  }
}
