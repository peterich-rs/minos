import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/root_route_decision.dart';
import 'package:minos/presentation/pages/pairing_page.dart';
import 'package:minos/presentation/pages/thread_list_page.dart';

/// Root of the Minos app. Provides the Shad theme and routes between
/// the splash / login / pairing / threadList surfaces based on the
/// joint state of [authControllerProvider], [connectionStateProvider]
/// and [hasPersistedPairingProvider].
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
    final authState = ref.watch(authControllerProvider);
    final connection = ref.watch(connectionStateProvider);
    final hasPersistedPairing = ref.watch(hasPersistedPairingProvider);
    final route = decideRootRoute(
      authState: authState,
      connectionState: connection.asData?.value,
      hasPersistedPairing: hasPersistedPairing.asData?.value ?? false,
    );

    return switch (route) {
      RootRoute.splash => const _SplashScreen(),
      // Phase 9 replaces this with the real LoginPage; Phase 8 only
      // gates the routing.
      RootRoute.login => const _SplashScreen(label: 'Login (Phase 9)'),
      RootRoute.pairing => const PairingPage(),
      RootRoute.threadList => const ThreadListPage(),
      RootRoute.threadListMacOffline => const ThreadListPage(),
    };
  }
}

/// Cold-launch placeholder shown while the auth controller is still
/// reading the cached frame from the Rust watch-channel. Phase 9 may
/// swap this for a branded splash; the surface itself is incidental.
class _SplashScreen extends StatelessWidget {
  const _SplashScreen({this.label});
  final String? label;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const CircularProgressIndicator(),
            if (label != null) ...[
              const SizedBox(height: 12),
              Text(label!),
            ],
          ],
        ),
      ),
    );
  }
}
