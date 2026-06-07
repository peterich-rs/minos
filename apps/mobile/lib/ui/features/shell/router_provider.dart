import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'package:minos/ui/features/shell/router.dart';

/// Provides the singleton [GoRouter] instance for the app.
///
/// The router is created once and kept alive for the lifetime of the app.
/// It internally listens to auth/connection state changes via
/// [_RouterRefreshNotifier] and re-evaluates redirects automatically.
final routerProvider = Provider<GoRouter>((ref) {
  final router = createAppRouter(ref);
  ref.onDispose(router.dispose);
  return router;
});
