import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

/// Day divider pill for the collaboration timeline (Desktop day-divider parity).
class ConversationDayDivider extends StatelessWidget {
  const ConversationDayDivider({super.key, required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        MinosSpacing.lg,
        MinosSpacing.md,
        MinosSpacing.lg,
        MinosSpacing.sm,
      ),
      child: Row(
        children: <Widget>[
          Expanded(
            child: Divider(
              height: 1,
              thickness: 0.5,
              color: colors.borderSubtle,
            ),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: MinosSpacing.sm),
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: colors.surfaceMuted,
                borderRadius: MinosRadii.pillAll,
              ),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: MinosSpacing.sm + 2,
                  vertical: MinosSpacing.xs,
                ),
                child: Text(
                  label,
                  style: theme.textTheme.labelSmall?.copyWith(
                    color: colors.textSecondary,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
            ),
          ),
          Expanded(
            child: Divider(
              height: 1,
              thickness: 0.5,
              color: colors.borderSubtle,
            ),
          ),
        ],
      ),
    );
  }
}
