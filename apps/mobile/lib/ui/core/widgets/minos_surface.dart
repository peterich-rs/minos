import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

/// Soft surface card used for grouped rows and host/session cards.
class MinosSurface extends StatelessWidget {
  const MinosSurface({
    super.key,
    required this.child,
    this.padding,
    this.onTap,
    this.bordered = false,
    this.highlighted = false,
  });

  final Widget child;
  final EdgeInsetsGeometry? padding;
  final VoidCallback? onTap;
  final bool bordered;
  final bool highlighted;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final decoration = BoxDecoration(
      color: highlighted ? colors.accentSoft : colors.surface,
      borderRadius: MinosRadii.mdAll,
      border: bordered || highlighted
          ? Border.all(
              color: highlighted
                  ? colors.accent.withValues(alpha: 0.35)
                  : colors.borderSubtle,
            )
          : null,
    );

    final content = Padding(padding: padding ?? EdgeInsets.zero, child: child);

    if (onTap == null) {
      return DecoratedBox(decoration: decoration, child: content);
    }

    return Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: onTap,
        borderRadius: MinosRadii.mdAll,
        child: Ink(decoration: decoration, child: content),
      ),
    );
  }
}
