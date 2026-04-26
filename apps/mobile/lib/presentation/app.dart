import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/root_route_decision.dart';
import 'package:minos/presentation/pages/pairing_page.dart';
import 'package:minos/presentation/pages/thread_list_page.dart';

/// Root of the Minos app. Provides the Shad theme and routes between
/// [PairingPage] and [ThreadListPage] based on the latest [ConnectionState].
class MinosApp extends StatelessWidget {
  const MinosApp({super.key});

  @override
  Widget build(BuildContext context) {
    return ShadApp(
      title: 'Minos',
      themeMode: ThemeMode.system,
      theme: ShadThemeData(
        brightness: Brightness.light,
        colorScheme: const ShadZincColorScheme.light(),
      ),
      darkTheme: ShadThemeData(
        brightness: Brightness.dark,
        colorScheme: const ShadZincColorScheme.dark(),
      ),
      // Passing a builder activates the toaster/sonner wrapping that
      // [ShadToaster.of] requires.
      builder: (context, child) => child ?? const SizedBox.shrink(),
      home: const _Router(),
    );
  }
}

class _Router extends ConsumerWidget {
  const _Router();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(connectionStateProvider);
    final hasPersistedPairing = ref.watch(hasPersistedPairingProvider);
    final route = decideRootRoute(
      connectionState: state.asData?.value,
      hasPersistedPairing: hasPersistedPairing.asData?.value ?? false,
    );

    return switch (route) {
      RootRoute.threads => const ThreadListPage(),
      RootRoute.pairing => const PairingPage(),
    };
  }
}
