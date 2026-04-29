import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/src/rust/api/minos.dart';

const int kRequestTraceRingCapacity = 200;

/// Append-only mirror of the Rust-side request trace store.
///
/// The Rust stream emits both newly-started requests and later updates for the
/// same request id, so this notifier replaces existing entries in place while
/// preserving insertion order.
class RequestTraceRecords extends Notifier<List<RequestTraceRecord>> {
  static final _provider =
      NotifierProvider<RequestTraceRecords, List<RequestTraceRecord>>(
        RequestTraceRecords.new,
      );

  static NotifierProvider<RequestTraceRecords, List<RequestTraceRecord>>
  get provider => _provider;

  StreamSubscription<RequestTraceRecord>? _subscription;
  final List<RequestTraceRecord> _buffer = <RequestTraceRecord>[];

  @override
  List<RequestTraceRecord> build() {
    ref.onDispose(_disposeSubscription);
    _seed();
    _subscribe();
    return List.unmodifiable(_buffer);
  }

  void _seed() {
    _buffer
      ..clear()
      ..addAll(recentRequestTraces());
  }

  void _subscribe() {
    _subscription = subscribeRequestTraces().listen(
      _upsert,
      onError: (Object _) {
        // Best-effort inspector; a later subscription can resnapshot.
      },
    );
  }

  void _upsert(RequestTraceRecord record) {
    final index = _buffer.indexWhere((entry) => entry.id == record.id);
    if (index >= 0) {
      _buffer[index] = record;
    } else {
      if (_buffer.length >= kRequestTraceRingCapacity) {
        _buffer.removeAt(0);
      }
      _buffer.add(record);
    }
    state = List.unmodifiable(_buffer);
  }

  void clear() {
    clearRequestTraces();
    _buffer.clear();
    state = const <RequestTraceRecord>[];
  }

  void _disposeSubscription() {
    _subscription?.cancel();
    _subscription = null;
  }
}
