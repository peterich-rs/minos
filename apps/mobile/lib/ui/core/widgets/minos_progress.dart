import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

/// Compact circular progress indicator using Minos accent.
class MinosProgress extends StatelessWidget {
  const MinosProgress({super.key, this.size = 28, this.strokeWidth = 2.5});

  final double size;
  final double strokeWidth;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    return SizedBox(
      width: size,
      height: size,
      child: CircularProgressIndicator(
        strokeWidth: strokeWidth,
        color: colors.accent,
      ),
    );
  }
}
