import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

/// Centered muted system chrome (Desktop `MessageSystemChrome` parity).
class ConversationSystemMessage extends StatelessWidget {
  const ConversationSystemMessage({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: MinosSpacing.xxl,
        vertical: MinosSpacing.sm,
      ),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 360),
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: colors.surfaceMuted,
              borderRadius: MinosRadii.mdAll,
            ),
            child: Padding(
              padding: const EdgeInsets.symmetric(
                horizontal: MinosSpacing.md,
                vertical: MinosSpacing.sm,
              ),
              child: Text(
                text,
                textAlign: TextAlign.center,
                style: theme.textTheme.bodySmall?.copyWith(
                  color: colors.textSecondary,
                  fontWeight: FontWeight.w500,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
