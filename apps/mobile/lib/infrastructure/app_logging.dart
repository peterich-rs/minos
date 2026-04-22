import 'dart:io';

import 'package:xlog/xlog.dart';

// Keep the logger alive for the app's lifetime; dropping it would stop the
// mars-xlog appender.
MarsXlogLogger? _logger;

/// Open the Dart-side mars-xlog writer at `<logDir>` with name prefix
/// `mobile-flutter`. The Rust-side writer (opened via [MinosCore.init]) uses
/// name prefix `mobile-rust` and shares the same directory — per spec §9.4,
/// the (namePrefix, logDir) pair is the single-writer key so the two do not
/// collide.
///
/// Idempotent: calling twice is a no-op. Tests that need a fresh writer
/// should use a separate directory.
Future<void> initDartLogger({required String logDir}) async {
  if (_logger != null) return;

  final cacheDir = '$logDir/cache';
  await Directory(cacheDir).create(recursive: true);

  _logger = MarsXlogLogger.open(
    MarsXlogConfig(
      logDir: logDir,
      cacheDir: cacheDir,
      namePrefix: 'mobile-flutter',
      appenderMode: MarsXlogAppenderMode.async,
      compressMode: MarsXlogCompressMode.zstd,
    ),
    level: MarsXlogLevel.info,
  );
}
