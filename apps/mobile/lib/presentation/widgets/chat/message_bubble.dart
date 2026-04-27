import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';

/// A single chat bubble. User-side bubbles are right-aligned with the
/// primary container colour; assistant-side bubbles are left-aligned with
/// the surface colour. When [isStreaming] is true, a small blinking
/// cursor renders below the body to signal that more text will arrive.
///
/// The body is rendered as Markdown (using `flutter_markdown_plus`) so
/// the assistant can stream fenced code, lists, and inline formatting.
/// The widget is intentionally stateless except for the cursor's
/// own animation controller.
class MessageBubble extends StatelessWidget {
  const MessageBubble({
    super.key,
    required this.isUser,
    required this.markdownContent,
    this.isStreaming = false,
  });

  final bool isUser;
  final String markdownContent;
  final bool isStreaming;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final bg = isUser
        ? scheme.primaryContainer
        : scheme.surfaceContainerHighest;
    final fg = isUser ? scheme.onPrimaryContainer : scheme.onSurface;
    final align = isUser ? Alignment.centerRight : Alignment.centerLeft;
    final crossAxis = isUser
        ? CrossAxisAlignment.end
        : CrossAxisAlignment.start;

    return Align(
      alignment: align,
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: MediaQuery.of(context).size.width * 0.85,
        ),
        child: Container(
          margin: const EdgeInsets.symmetric(vertical: 4, horizontal: 12),
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
          decoration: BoxDecoration(
            color: bg,
            borderRadius: BorderRadius.circular(14),
          ),
          child: Column(
            crossAxisAlignment: crossAxis,
            mainAxisSize: MainAxisSize.min,
            children: [
              MarkdownBody(
                data: markdownContent,
                selectable: true,
                styleSheet: MarkdownStyleSheet.fromTheme(theme).copyWith(
                  p: theme.textTheme.bodyMedium?.copyWith(color: fg),
                  code: theme.textTheme.bodySmall?.copyWith(
                    fontFamily: 'monospace',
                    color: fg,
                    backgroundColor: scheme.surface,
                  ),
                ),
              ),
              if (isStreaming)
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: _StreamingCursor(color: fg),
                ),
            ],
          ),
        ),
      ),
    );
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
        width: 8,
        height: 14,
        decoration: BoxDecoration(
          color: widget.color,
          borderRadius: BorderRadius.circular(2),
        ),
      ),
    );
  }
}
