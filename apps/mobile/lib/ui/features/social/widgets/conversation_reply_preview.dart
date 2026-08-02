import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

/// Left-border reply chip under a message header (Desktop `ReplyPreview` parity).
class ConversationReplyPreview extends StatelessWidget {
  const ConversationReplyPreview({
    super.key,
    required this.senderName,
    required this.text,
    this.isRecalled = false,
  });

  final String senderName;
  final String text;
  final bool isRecalled;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: colors.surfaceMuted.withValues(alpha: 0.85),
        borderRadius: MinosRadii.smAll,
        border: Border(
          left: BorderSide(
            color: colors.border.withValues(alpha: 0.9),
            width: 2,
          ),
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          MinosSpacing.sm + 2,
          MinosSpacing.xs + 2,
          MinosSpacing.sm,
          MinosSpacing.xs + 2,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Text(
              '↳ $senderName',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: theme.textTheme.labelMedium?.copyWith(
                color: colors.textPrimary,
                fontWeight: FontWeight.w600,
              ),
            ),
            const SizedBox(height: MinosSpacing.xxs),
            Text(
              isRecalled ? '原消息已撤回' : text,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: theme.textTheme.bodySmall?.copyWith(
                color: colors.textSecondary,
                height: 1.3,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
