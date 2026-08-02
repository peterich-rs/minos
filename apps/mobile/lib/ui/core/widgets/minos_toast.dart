import 'package:flutter/material.dart';
import 'package:minos/ui/theme/theme.dart';

/// Floating toast / snack feedback using Minos tokens (replaces ShadToaster).
void showMinosToast(
  BuildContext context, {
  required String title,
  String? description,
  bool destructive = false,
}) {
  if (!context.mounted) return;
  final messenger = ScaffoldMessenger.maybeOf(context);
  if (messenger == null) return;

  final colors = context.minosColors;
  final detail = description?.trim() ?? '';
  final text = detail.isEmpty ? title : '$title · $detail';

  messenger
    ..hideCurrentSnackBar()
    ..showSnackBar(
      SnackBar(
        content: Text(
          text,
          style: Theme.of(context).textTheme.bodyMedium?.copyWith(
            color: destructive ? colors.textOnAccent : colors.textPrimary,
          ),
        ),
        backgroundColor: destructive ? colors.danger : colors.surfaceElevated,
        behavior: SnackBarBehavior.floating,
      ),
    );
}
