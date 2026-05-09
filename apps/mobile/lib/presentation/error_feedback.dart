import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/src/rust/api/minos.dart';

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
  try {
    emitLog(
      level: LogLevel.error,
      target: 'minos_mobile::flutter::$target',
      message: detail == source || source.isEmpty
          ? message
          : '$message (source: $source)',
    );
  } catch (_) {
    // Toast delivery should still work even if the logging bridge fails.
  }

  ShadToaster.maybeOf(context)?.show(
    ShadToast.destructive(
      title: Text(title),
      description: detail.isEmpty ? null : Text(detail),
    ),
  );
}
