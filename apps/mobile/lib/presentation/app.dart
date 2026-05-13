import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/project_providers.dart';
import 'package:minos/application/router_provider.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/application/thread_events_provider.dart';
import 'package:minos/application/thread_list_provider.dart';
import 'package:minos/src/rust/api/minos.dart' as core;
import 'package:shadcn_ui/shadcn_ui.dart';

/// Root of the Minos app. Provides the Shad theme, uses [GoRouter] for
/// declarative routing between splash / login / shell surfaces based on
/// the joint state of auth, connection, and pairing providers, and bridges
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
    ref.listen<AsyncValue<core.ConnectionState>>(connectionStateProvider, (
      previous,
      next,
    ) {
      final previousState = previous?.asData?.value;
      final currentState = next.asData?.value;
      if (currentState is! core.ConnectionState_Connected ||
          previousState is core.ConnectionState_Connected) {
        return;
      }

      ref
        ..invalidate(projectListProvider)
        ..invalidate(threadListProvider)
        ..invalidate(threadEventsProvider)
        ..invalidate(conversationsProvider)
        ..invalidate(friendsProvider)
        ..invalidate(friendRequestsProvider)
        ..invalidate(socialProfileProvider)
        ..invalidate(pairedMacsProvider)
        ..invalidate(activeMacProvider)
        ..invalidate(runtimeAgentDescriptorsProvider)
        ..invalidate(hostSkillsProvider)
        ..invalidate(agentProfilesControllerProvider);
    });

    final router = ref.watch(routerProvider);

    return ShadApp.router(
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
      routerConfig: router,
    );
  }
}
