import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/root_route_decision.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/presentation/pages/login_page.dart';
import 'package:minos/presentation/pages/pairing_page.dart';
import 'package:minos/presentation/pages/thread_list_page.dart';

/// Root of the Minos app. Provides the Shad theme, routes between the
/// splash / login / pairing / threadList surfaces based on the joint
/// state of [authControllerProvider], [connectionStateProvider] and
/// [hasPersistedPairingProvider], and bridges
/// [WidgetsBindingObserver.didChangeAppLifecycleState] into the Rust
/// core's `notifyForegrounded` / `notifyBackgrounded` hooks so the WS
/// reconnect loop respects the OS lifecycle (Phase 11.2).
class MinosApp extends ConsumerStatefulWidget {
  const MinosApp({super.key});

  @override
  ConsumerState<MinosApp> createState() => _MinosAppState();
}

class _MinosAppState extends ConsumerState<MinosApp>
    with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    final core = ref.read(minosCoreProvider);
    switch (state) {
      case AppLifecycleState.resumed:
        core.notifyForegrounded();
      case AppLifecycleState.paused:
      case AppLifecycleState.inactive:
      case AppLifecycleState.detached:
      case AppLifecycleState.hidden:
        core.notifyBackgrounded();
    }
  }

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
      // Surface a leftover refresh-failed error as the initial banner so
      // the user knows WHY they bounced back to login; the LoginPage owns
      // the auto-dismiss timer from there on.
      RootRoute.login => LoginPage(
        errorBanner: switch (authState) {
          AuthRefreshFailed(:final error) => error,
          _ => null,
        },
      ),
      RootRoute.pairing => const PairingPage(),
      RootRoute.threadList => const ThreadListPage(),
      RootRoute.threadListMacOffline => const ThreadListPage(),
    };
  }
}

/// Cold-launch placeholder shown while the auth controller is still
/// reading the cached frame from the Rust watch-channel. The surface
/// itself is incidental — a branded splash can replace it later.
class _SplashScreen extends StatelessWidget {
  const _SplashScreen();

  @override
  Widget build(BuildContext context) {
    return const Scaffold(body: Center(child: CircularProgressIndicator()));
  }
}
