import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

enum MinosPresence { online, offline, warning, unknown }

/// Small presence indicator used on host cards and connection rows.
class MinosStatusDot extends StatelessWidget {
  const MinosStatusDot({super.key, required this.presence, this.size = 8});

  final MinosPresence presence;
  final double size;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final color = switch (presence) {
      MinosPresence.online => colors.success,
      MinosPresence.offline => colors.textTertiary,
      MinosPresence.warning => colors.warning,
      MinosPresence.unknown => colors.textTertiary,
    };
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: color,
        shape: BoxShape.circle,
        boxShadow: presence == MinosPresence.online
            ? <BoxShadow>[
                BoxShadow(
                  color: color.withValues(alpha: 0.45),
                  blurRadius: 4,
                  spreadRadius: 0.5,
                ),
              ]
            : null,
      ),
    );
  }
}
