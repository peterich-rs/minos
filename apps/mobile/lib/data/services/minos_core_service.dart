import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/domain/minos_core_protocol.dart';

final minosCoreServiceProvider = Provider<MinosCoreProtocol>((ref) {
  throw UnimplementedError(
    'minosCoreServiceProvider must be overridden in main() with a concrete '
    'MinosCore instance',
  );
});
