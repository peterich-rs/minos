import 'dart:async';
import 'dart:collection';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/src/rust/api/minos.dart';

/// Cap on records kept in the Dart-side mirror. Matches the Rust ring
/// buffer so the panel never out-grows what the core side will replay.
const int kLogRecordRingCapacity = 500;

/// Append-only mirror of the Rust-side log capture, exposed to widgets as
/// a `List<LogRecord>` ordered oldest → newest.
///
/// On first read the notifier seeds itself with `recentLogRecords()` (the
/// snapshot the Rust ring buffer already holds), then subscribes to
/// `subscribeLogRecords()` for the live tail. The combined view survives
/// page navigations because the provider is `keepAlive`.
class LogRecords extends Notifier<List<LogRecord>> {
  static final _provider = NotifierProvider<LogRecords, List<LogRecord>>(
    LogRecords.new,
  );

  /// Riverpod handle. Widgets do `ref.watch(LogRecords.provider)` to read
  /// the current list.
  static NotifierProvider<LogRecords, List<LogRecord>> get provider =>
      _provider;

  StreamSubscription<LogRecord>? _subscription;
  final Queue<LogRecord> _buffer = Queue<LogRecord>();

  @override
  List<LogRecord> build() {
    ref.onDispose(_disposeSubscription);
    _seed();
    _subscribe();
    return List.unmodifiable(_buffer);
  }

  void _seed() {
    _buffer
      ..clear()
      ..addAll(recentLogRecords());
  }

  void _subscribe() {
    _subscription = subscribeLogRecords().listen(
      _append,
      onError: (Object _) {
        // Stream errors are non-fatal — Rust will still capture into xlog
        // and the next subscribe attempt picks up the live tail again.
      },
    );
  }

  void _append(LogRecord record) {
    if (_buffer.length >= kLogRecordRingCapacity) {
      _buffer.removeFirst();
    }
    _buffer.addLast(record);
    state = List.unmodifiable(_buffer);
  }

  void _disposeSubscription() {
    _subscription?.cancel();
    _subscription = null;
  }

  /// Drop every cached record. Useful for the "clear" affordance in the
  /// debug panel; the next emitted record refills from the tail forward.
  void clear() {
    _buffer.clear();
    state = const <LogRecord>[];
  }
}
