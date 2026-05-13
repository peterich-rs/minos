import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/root_route_decision.dart';
import 'package:minos/presentation/pages/agent_start_page.dart';
import 'package:minos/presentation/pages/agents_hub_page.dart';
import 'package:minos/presentation/pages/app_shell_page.dart';
import 'package:minos/presentation/pages/group_members_page.dart';
import 'package:minos/presentation/pages/log_viewer_page.dart';
import 'package:minos/presentation/pages/login_page.dart';
import 'package:minos/presentation/pages/pairing_page.dart';
import 'package:minos/presentation/pages/project_detail_page.dart';
import 'package:minos/presentation/pages/social_chat_page.dart';
import 'package:minos/presentation/pages/social_hub_page.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/src/rust/api/minos.dart' show AgentName, ConversationKind;

/// Route path constants for the app.
abstract final class AppRoutes {
  static const String splash = '/splash';
  static const String login = '/login';
  static const String shell = '/';
  static const String thread = '/thread/:threadId';
  static const String newThread = '/thread/new';
  static const String agentStart = '/agent-start';
  static const String agentProfile = '/agent-profile/:profileId';
  static const String pairing = '/pairing';
  static const String logViewer = '/log-viewer';
  static const String socialHub = '/social';
  static const String socialChat = '/social/chat/:conversationId';
  static const String groupMembers = '/social/chat/:conversationId/members';
}

/// Creates the [GoRouter] instance wired to Riverpod for auth-based redirects.
///
/// The router watches [authControllerProvider], [connectionStateProvider], and
/// [hasPersistedPairingProvider] to decide the root redirect (splash → login →
/// shell) using the same [decideRootRoute] pure function the old `_Router`
/// widget used.
GoRouter createAppRouter(Ref ref) {
  // Listenable that fires whenever the auth/connection state changes,
  // triggering GoRouter's redirect evaluation.
  final routerNotifier = _RouterRefreshNotifier(ref);

  return GoRouter(
    initialLocation: AppRoutes.splash,
    refreshListenable: routerNotifier,
    redirect: (context, state) {
      final authState = ref.read(authControllerProvider);
      final connection = ref.read(connectionStateProvider);
      final hasPersistedPairing = ref.read(hasPersistedPairingProvider);
      final route = decideRootRoute(
        authState: authState,
        connectionState: connection.asData?.value,
        hasPersistedPairing: hasPersistedPairing.asData?.value ?? false,
      );

      final currentPath = state.uri.path;

      // Determine where the user should be based on auth state.
      switch (route) {
        case RootRoute.splash:
          if (currentPath != AppRoutes.splash) return AppRoutes.splash;
        case RootRoute.login:
          if (currentPath != AppRoutes.login) return AppRoutes.login;
        case RootRoute.projectList:
        case RootRoute.projectListOffline:
          // If user is authenticated but on splash/login, redirect to shell.
          if (currentPath == AppRoutes.splash ||
              currentPath == AppRoutes.login) {
            return AppRoutes.shell;
          }
      }
      return null; // No redirect needed.
    },
    routes: <RouteBase>[
      // Splash
      GoRoute(
        path: AppRoutes.splash,
        builder: (context, state) => const _SplashScreen(),
      ),

      // Login
      GoRoute(
        path: AppRoutes.login,
        builder: (context, state) {
          // The LoginPage reads the auth state directly from the provider
          // to surface any refresh-failed error banner.
          return const LoginPage();
        },
      ),

      // Main shell with bottom navigation
      GoRoute(
        path: AppRoutes.shell,
        builder: (context, state) => const AppShellPage(),
      ),

      // Thread view (existing thread)
      GoRoute(
        path: AppRoutes.thread,
        builder: (context, state) {
          final threadId = state.pathParameters['threadId']!;
          final extra = state.extra as ThreadRouteExtra?;
          return ThreadViewPage(
            threadId: threadId,
            agent: extra?.agent,
            agentProfileId: extra?.agentProfileId,
          );
        },
      ),

      // New thread (no threadId yet)
      GoRoute(
        path: AppRoutes.newThread,
        builder: (context, state) {
          final extra = state.extra as ThreadRouteExtra?;
          return ThreadViewPage(agentProfileId: extra?.agentProfileId);
        },
      ),

      // Agent start page (pick agent + workspace before starting)
      GoRoute(
        path: AppRoutes.agentStart,
        builder: (context, state) => const AgentStartPage(),
      ),

      // Agent profile detail
      GoRoute(
        path: AppRoutes.agentProfile,
        builder: (context, state) {
          final profileId = state.pathParameters['profileId']!;
          return AgentProfilePage(profileId: profileId);
        },
      ),

      // Pairing
      GoRoute(
        path: AppRoutes.pairing,
        builder: (context, state) => const PairingPage(),
      ),

      // Log viewer
      GoRoute(
        path: AppRoutes.logViewer,
        builder: (context, state) => const LogViewerPage(),
      ),

      // Project detail
      GoRoute(
        path: '/project/:projectId',
        builder: (context, state) {
          final projectId = state.pathParameters['projectId']!;
          final extra = state.extra as ProjectDetailRouteExtra?;
          return ProjectDetailPage(
            projectId: projectId,
            projectName: extra?.projectName ?? '',
          );
        },
      ),

      // Social hub
      GoRoute(
        path: AppRoutes.socialHub,
        builder: (context, state) => const SocialHubPage(),
      ),

      // Social chat
      GoRoute(
        path: AppRoutes.socialChat,
        builder: (context, state) {
          final conversationId = state.pathParameters['conversationId']!;
          final extra = state.extra as SocialChatRouteExtra?;
          return SocialChatPage(
            conversationId: conversationId,
            title: extra?.title ?? '',
            kind: extra?.kind ?? ConversationKind.direct,
          );
        },
      ),

      // Group members
      GoRoute(
        path: AppRoutes.groupMembers,
        builder: (context, state) {
          final conversationId = state.pathParameters['conversationId']!;
          final extra = state.extra as GroupMembersRouteExtra?;
          return GroupMembersPage(
            conversationId: conversationId,
            title: extra?.title ?? '群成员',
          );
        },
      ),
    ],
  );
}

/// Extra data passed to the thread route.
class ThreadRouteExtra {
  const ThreadRouteExtra({this.agent, this.agentProfileId});

  final AgentName? agent;
  final String? agentProfileId;
}

/// Extra data passed to the social chat route.
class SocialChatRouteExtra {
  const SocialChatRouteExtra({required this.title, required this.kind});

  final String title;
  final ConversationKind kind;
}

/// Extra data passed to the group members route.
class GroupMembersRouteExtra {
  const GroupMembersRouteExtra({required this.title});

  final String title;
}

/// Extra data passed to the project detail route.
class ProjectDetailRouteExtra {
  const ProjectDetailRouteExtra({required this.projectName});

  final String projectName;
}

/// A [ChangeNotifier] that listens to auth/connection provider changes and
/// notifies GoRouter to re-evaluate its redirect logic.
class _RouterRefreshNotifier extends ChangeNotifier {
  _RouterRefreshNotifier(this._ref) {
    _subscriptions = [
      _ref.listen(authControllerProvider, (_, _) => notifyListeners()),
      _ref.listen(connectionStateProvider, (_, _) => notifyListeners()),
      _ref.listen(hasPersistedPairingProvider, (_, _) => notifyListeners()),
    ];
  }

  final Ref _ref;
  late final List<ProviderSubscription<dynamic>> _subscriptions;

  @override
  void dispose() {
    for (final sub in _subscriptions) {
      sub.close();
    }
    super.dispose();
  }
}

/// Cold-launch placeholder shown while the auth controller is still
/// reading the cached frame from the Rust watch-channel.
class _SplashScreen extends StatelessWidget {
  const _SplashScreen();

  @override
  Widget build(BuildContext context) {
    return const Scaffold(body: Center(child: CircularProgressIndicator()));
  }
}
