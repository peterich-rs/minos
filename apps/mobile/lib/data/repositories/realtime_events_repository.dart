import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Account realtime control-plane events (snapshot / presence / notices).
///
/// Not a product agent-transcript or session-compose surface.
final realtimeEventsRepositoryProvider = Provider<RealtimeEventsRepository>((
  ref,
) {
  return RealtimeEventsRepository(ref.watch(minosCoreServiceProvider));
});

class RealtimeEventsRepository {
  const RealtimeEventsRepository(this._core);

  final MinosCoreProtocol _core;

  Stream<UiEventFrame> get uiEvents => _core.uiEvents;
}
