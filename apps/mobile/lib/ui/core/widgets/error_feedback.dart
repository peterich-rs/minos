import 'package:flutter/material.dart';
import 'package:minos/application/flutter_log.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

void showLoggedErrorToast(
  BuildContext context, {
  required String target,
  required String title,
  required Object error,
  String? description,
}) {
  final detail = (description ?? error.toString()).trim();
  final source = error.toString().trim();
  final message = detail.isEmpty ? title : '$title: $detail';
  logFlutterError(
    target,
    detail == source || source.isEmpty ? message : '$message (source: $source)',
  );

  final toaster = ShadToaster.maybeOf(context);
  if (toaster != null) {
    toaster.show(
      ShadToast.destructive(
        title: Text(title),
        description: detail.isEmpty ? null : Text(detail),
      ),
    );
    return;
  }

  if (!context.mounted) return;
  final messenger = ScaffoldMessenger.maybeOf(context);
  messenger?.showSnackBar(
    SnackBar(content: Text(detail.isEmpty ? title : '$title · $detail')),
  );
}
