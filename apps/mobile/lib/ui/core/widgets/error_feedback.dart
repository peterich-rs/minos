import 'package:flutter/material.dart';
import 'package:minos/application/flutter_log.dart';
import 'package:minos/ui/core/widgets/minos_toast.dart';

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

  showMinosToast(
    context,
    title: title,
    description: detail.isEmpty ? null : detail,
    destructive: true,
  );
}
