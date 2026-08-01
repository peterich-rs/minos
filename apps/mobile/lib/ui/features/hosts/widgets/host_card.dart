import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:minos/ui/core/widgets/minos_status_dot.dart';
import 'package:minos/ui/core/widgets/minos_surface.dart';
import 'package:minos/ui/theme/theme.dart';

/// Card for one linked host (online / offline / selected).
class HostCard extends StatelessWidget {
  const HostCard({
    super.key,
    required this.displayName,
    required this.online,
    required this.selected,
    required this.onTap,
    this.subtitle,
  });

  final String displayName;
  final bool online;
  final bool selected;
  final VoidCallback onTap;
  final String? subtitle;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final name = displayName.trim().isEmpty ? 'Mac' : displayName.trim();
    final initial = name.isNotEmpty ? name[0].toUpperCase() : 'M';

    return MinosSurface(
      highlighted: selected,
      bordered: true,
      onTap: onTap,
      padding: const EdgeInsets.all(MinosSpacing.lg),
      child: Row(
        children: <Widget>[
          Container(
            width: 48,
            height: 48,
            decoration: BoxDecoration(
              color: online ? colors.accentSoft : colors.surfaceMuted,
              borderRadius: MinosRadii.smAll,
            ),
            alignment: Alignment.center,
            child: Text(
              initial,
              style: theme.textTheme.titleLarge?.copyWith(
                color: online ? colors.accent : colors.textTertiary,
              ),
            ),
          ),
          const SizedBox(width: MinosSpacing.md),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.titleMedium,
                ),
                const SizedBox(height: MinosSpacing.xs),
                Row(
                  children: <Widget>[
                    MinosStatusDot(
                      presence: online
                          ? MinosPresence.online
                          : MinosPresence.offline,
                    ),
                    const SizedBox(width: MinosSpacing.xs),
                    Flexible(
                      child: Text(
                        subtitle ?? (online ? '设备在线 · 可路由' : '设备离线'),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.bodySmall?.copyWith(
                          color: online ? colors.success : colors.textSecondary,
                        ),
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
          const SizedBox(width: MinosSpacing.sm),
          if (selected)
            Icon(
              CupertinoIcons.checkmark_circle_fill,
              color: colors.accent,
              size: 22,
            )
          else
            Icon(
              CupertinoIcons.chevron_right,
              size: 16,
              color: colors.textTertiary,
            ),
        ],
      ),
    );
  }
}
