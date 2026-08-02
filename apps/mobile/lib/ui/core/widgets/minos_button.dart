import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

enum MinosButtonVariant { primary, outline, ghost, destructive }

/// Minos-styled button replacing shadcn [ShadButton] variants.
class MinosButton extends StatelessWidget {
  const MinosButton({
    super.key,
    required this.onPressed,
    required this.child,
    this.variant = MinosButtonVariant.primary,
    this.expanded = false,
  });

  const MinosButton.outline({
    super.key,
    required this.onPressed,
    required this.child,
    this.expanded = false,
  }) : variant = MinosButtonVariant.outline;

  const MinosButton.ghost({
    super.key,
    required this.onPressed,
    required this.child,
    this.expanded = false,
  }) : variant = MinosButtonVariant.ghost;

  const MinosButton.destructive({
    super.key,
    required this.onPressed,
    required this.child,
    this.expanded = false,
  }) : variant = MinosButtonVariant.destructive;

  final VoidCallback? onPressed;
  final Widget child;
  final MinosButtonVariant variant;
  final bool expanded;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final button = switch (variant) {
      MinosButtonVariant.primary => FilledButton(
        onPressed: onPressed,
        child: child,
      ),
      MinosButtonVariant.outline => OutlinedButton(
        onPressed: onPressed,
        child: child,
      ),
      MinosButtonVariant.ghost => TextButton(
        onPressed: onPressed,
        child: child,
      ),
      MinosButtonVariant.destructive => FilledButton(
        onPressed: onPressed,
        style: FilledButton.styleFrom(
          backgroundColor: colors.danger,
          foregroundColor: colors.textOnAccent,
        ),
        child: child,
      ),
    };

    // Parent must offer a finite max width (Column / Expanded / sized box).
    if (!expanded) return button;
    return SizedBox(width: double.infinity, child: button);
  }
}
