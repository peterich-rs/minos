import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

/// Large-title header for top-level shell tabs (iOS-style, single column).
class MinosPageHeader extends StatelessWidget {
  const MinosPageHeader({
    super.key,
    required this.title,
    this.subtitle,
    this.trailing,
  });

  final String title;
  final String? subtitle;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);

    return Padding(
      padding: const EdgeInsets.fromLTRB(
        MinosSpacing.pageX,
        MinosSpacing.md,
        MinosSpacing.md,
        MinosSpacing.sm,
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: <Widget>[
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(title, style: theme.textTheme.headlineLarge),
                if (subtitle != null) ...<Widget>[
                  const SizedBox(height: MinosSpacing.xs),
                  Text(
                    subtitle!,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: colors.textSecondary,
                    ),
                  ),
                ],
              ],
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}
