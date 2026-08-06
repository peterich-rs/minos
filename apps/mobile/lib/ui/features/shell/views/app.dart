import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/im_outbox_worker.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/project_providers.dart';
import 'package:minos/application/runtime_actions.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/application/thread_events_provider.dart';
import 'package:minos/application/thread_list_provider.dart';
import 'package:minos/src/rust/api/minos.dart' as core;
import 'package:minos/ui/features/shell/router_provider.dart';
import 'package:minos/ui/theme/theme.dart';

/// Root of the Minos app.
///
/// Golden-path surfaces use [MinosTheme] tokens only (no shadcn).
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
    ref.read(runtimeActionsProvider).notifyAppLifecycle(state);
  }

  @override
  Widget build(BuildContext context) {
    // Side-effect listen is the Riverpod-supported pattern (not initState).
    // fireImmediately is false, so the first AsyncValue does not run here
    // during mount — only later Connected transitions invalidate caches.
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

    // C6.2: App-root outbox worker bootstrap (cold start without Messages tab).
    ref.watch(imOutboxBootstrapProvider);

    final router = ref.watch(routerProvider);

    return MaterialApp.router(
      title: 'Minos',
      debugShowCheckedModeBanner: false,
      themeMode: ThemeMode.system,
      theme: MinosTheme.light(),
      darkTheme: MinosTheme.dark(),
      routerConfig: router,
    );
  }
}
