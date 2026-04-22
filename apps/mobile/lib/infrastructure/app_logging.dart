import 'dart:io';

import 'package:xlog/xlog.dart';

// Kept alive for the app's lifetime so mars-xlog's appender thread is not
// collected. We never read the handle back — it exists purely as a top-level
// GC root — so the `unused_element` lint fires; that is by design.
// ignore: unused_element
MarsXlogLogger? _logger;

// Future-gated init so two concurrent callers return the same in-flight
// operation instead of racing across the `await Directory(...).create()`
// suspension point and each opening their own MarsXlogLogger.
Future<void>? _initFuture;

/// Open the Dart-side mars-xlog writer at `<logDir>` with name prefix
/// `mobile-flutter`. The Rust-side writer (opened via [MinosCore.init]) uses
/// name prefix `mobile-rust` and shares the same directory — per spec §9.4,
/// the (namePrefix, logDir) pair is the single-writer key so the two do not
/// collide.
///
/// Idempotent AND concurrency-safe: calling twice returns the same future.
/// Tests that need a fresh writer should use a separate directory.
Future<void> initDartLogger({required String logDir}) =>
    _initFuture ??= _openLogger(logDir);

Future<void> _openLogger(String logDir) async {
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
