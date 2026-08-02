import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

/// Slack/Buzz-style full-width row shell (Desktop `MessageChrome` parity).
///
/// Always left-aligned — no mine/others bubble split on the collaboration timeline.
class ConversationMessageChrome extends StatelessWidget {
  const ConversationMessageChrome({
    super.key,
    required this.avatar,
    required this.body,
    this.header,
    this.groupedWithPrevious = false,
    this.mentionsMe = false,
    this.onLongPress,
  });

  final Widget avatar;
  final Widget? header;
  final Widget body;
  final bool groupedWithPrevious;
  final bool mentionsMe;
  final VoidCallback? onLongPress;

  @override
  Widget build(BuildContext context) {
    final content = Material(
      color: Colors.transparent,
      child: InkWell(
        onLongPress: onLongPress,
        borderRadius: MinosRadii.mdAll,
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 150),
          curve: Curves.easeOut,
          decoration: BoxDecoration(
            borderRadius: MinosRadii.mdAll,
            color: mentionsMe
                ? const Color(0xFFF59E0B).withValues(alpha: 0.08)
                : Colors.transparent,
            border: mentionsMe
                ? const Border(
                    left: BorderSide(color: Color(0xFFF59E0B), width: 3),
                  )
                : null,
          ),
          padding: EdgeInsets.fromLTRB(
            mentionsMe ? MinosSpacing.sm : MinosSpacing.sm + 2,
            groupedWithPrevious ? MinosSpacing.xs : MinosSpacing.sm,
            MinosSpacing.sm + 2,
            MinosSpacing.sm,
          ),
          child: Row(
            crossAxisAlignment: groupedWithPrevious
                ? CrossAxisAlignment.center
                : CrossAxisAlignment.start,
            children: <Widget>[
              avatar,
              const SizedBox(width: MinosSpacing.sm + 2),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: <Widget>[
                    if (header != null) ...<Widget>[
                      header!,
                      const SizedBox(height: MinosSpacing.xxs),
                    ],
                    body,
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );

    return Padding(
      padding: EdgeInsets.only(
        left: MinosSpacing.sm,
        right: MinosSpacing.sm,
        top: groupedWithPrevious ? 0 : MinosSpacing.xxs,
        bottom: MinosSpacing.xxs,
      ),
      child: content,
    );
  }
}

/// Fixed-width avatar gutter matching Desktop `MessageAvatarGutter` (w-9 ≈ 36).
class ConversationAvatarGutter extends StatelessWidget {
  const ConversationAvatarGutter({super.key, required this.child});

  final Widget child;

  static const double width = 36;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: width,
      child: Align(alignment: Alignment.topCenter, child: child),
    );
  }
}

/// Author + timestamp baseline row (Desktop `MessageHeaderRow` parity).
class ConversationMessageHeader extends StatelessWidget {
  const ConversationMessageHeader({
    super.key,
    required this.authorLabel,
    required this.timeLabel,
    this.isAgent = false,
    this.sessionShort,
    this.deliveryLabel,
    this.deliveryIsError = false,
    this.onAuthorTap,
  });

  final String authorLabel;
  final String timeLabel;
  final bool isAgent;
  final String? sessionShort;
  final String? deliveryLabel;
  final bool deliveryIsError;
  final VoidCallback? onAuthorTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final authorStyle = theme.textTheme.labelLarge?.copyWith(
      color: colors.textPrimary,
      fontWeight: FontWeight.w700,
      height: 1.15,
      letterSpacing: -0.1,
    );

    Widget author = Text(
      authorLabel,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: authorStyle?.copyWith(
        decoration: onAuthorTap != null ? TextDecoration.underline : null,
        decorationColor: colors.textPrimary.withValues(alpha: 0.35),
      ),
    );
    if (onAuthorTap != null) {
      author = GestureDetector(
        onTap: onAuthorTap,
        behavior: HitTestBehavior.opaque,
        child: author,
      );
    }

    return Wrap(
      crossAxisAlignment: WrapCrossAlignment.center,
      spacing: MinosSpacing.xs + 2,
      runSpacing: MinosSpacing.xxs,
      children: <Widget>[
        if (isAgent)
          Icon(Icons.smart_toy_outlined, size: 14, color: colors.accent),
        author,
        if (sessionShort != null && sessionShort!.isNotEmpty)
          Text(
            '#$sessionShort',
            style: theme.textTheme.labelSmall?.copyWith(
              color: colors.textTertiary,
              fontFamily: 'monospace',
              fontWeight: FontWeight.w500,
            ),
          ),
        if (timeLabel.isNotEmpty)
          Text(
            timeLabel,
            style: theme.textTheme.labelSmall?.copyWith(
              color: colors.textTertiary,
              fontFeatures: const <FontFeature>[FontFeature.tabularFigures()],
            ),
          ),
        if (deliveryLabel != null)
          Text(
            deliveryLabel!,
            style: theme.textTheme.labelSmall?.copyWith(
              color: deliveryIsError ? colors.danger : colors.textTertiary,
              fontWeight: FontWeight.w600,
            ),
          ),
        if (isAgent)
          DecoratedBox(
            decoration: BoxDecoration(
              color: colors.accent.withValues(alpha: 0.12),
              borderRadius: MinosRadii.pillAll,
            ),
            child: Padding(
              padding: const EdgeInsets.symmetric(
                horizontal: MinosSpacing.xs + 2,
                vertical: 1,
              ),
              child: Text(
                'Agent',
                style: theme.textTheme.labelSmall?.copyWith(
                  color: colors.accent,
                  fontWeight: FontWeight.w700,
                ),
              ),
            ),
          ),
      ],
    );
  }
}
