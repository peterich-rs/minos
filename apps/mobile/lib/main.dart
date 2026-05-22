import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/infrastructure/app_paths.dart';
import 'package:minos/infrastructure/minos_core.dart';
import 'package:minos/ui/features/shell/views/app.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final logDir = await logDirectory();
  final core = await MinosCore.init(selfName: 'iPhone', logDir: logDir);
  runApp(
    ProviderScope(
      overrides: [minosCoreServiceProvider.overrideWithValue(core)],
      child: const MinosApp(),
    ),
  );
}
