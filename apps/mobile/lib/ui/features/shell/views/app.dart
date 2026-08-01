import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/project_providers.dart';
import 'package:minos/application/runtime_actions.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/application/thread_events_provider.dart';
import 'package:minos/application/thread_list_provider.dart';
import 'package:minos/src/rust/api/minos.dart' as core;
import 'package:minos/ui/features/shell/router_provider.dart';
import 'package:minos/ui/theme/theme.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

/// Root of the Minos app.
///
/// Golden-path surfaces use [MinosTheme] tokens. A residual [ShadTheme] is
/// layered for legacy screens (agent editor, social) that still read
/// `ShadTheme.of` until they are migrated or removed.
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

    return MaterialApp.router(
      title: 'Minos',
      debugShowCheckedModeBanner: false,
      themeMode: ThemeMode.system,
      theme: MinosTheme.light(),
      darkTheme: MinosTheme.dark(),
      routerConfig: router,
      builder: (context, child) {
        final brightness = Theme.of(context).brightness;
        final shadData = brightness == Brightness.dark
            ? ShadThemeData(
                brightness: Brightness.dark,
                colorScheme: const ShadZincColorScheme.dark(),
              )
            : ShadThemeData(
                brightness: Brightness.light,
                colorScheme: const ShadZincColorScheme.light(),
              );
        // ShadToaster supports residual toast call sites (error_feedback).
        return ShadTheme(
          data: shadData,
          child: ShadToaster(child: child ?? const SizedBox.shrink()),
        );
      },
    );
  }
}
