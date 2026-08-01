import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/root_route_decision.dart';
import 'package:minos/src/rust/api/minos.dart' show AgentName, ConversationKind;
import 'package:minos/ui/features/agents/views/agent_start_page.dart';
import 'package:minos/ui/features/agents/views/agents_hub_page.dart';
import 'package:minos/ui/features/auth/views/login_page.dart';
import 'package:minos/ui/features/chat/views/thread_view_page.dart';
import 'package:minos/ui/features/debug/views/log_viewer_page.dart';
import 'package:minos/ui/features/projects/views/project_detail_page.dart';
import 'package:minos/ui/features/shell/views/app_shell_page.dart';
import 'package:minos/ui/features/social/views/group_members_page.dart';
import 'package:minos/ui/features/social/views/social_chat_page.dart';
import 'package:minos/ui/features/social/views/social_hub_page.dart';

/// Route path constants for the app.
abstract final class AppRoutes {
  static const String splash = '/splash';
  static const String login = '/login';
  static const String shell = '/';
  static const String thread = '/thread/:sessionId';
  static const String newThread = '/thread/new';
  static const String agentStart = '/agent-start';
  static const String agentProfile = '/agent-profile/:profileId';
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

      switch (route) {
        case RootRoute.splash:
          if (currentPath != AppRoutes.splash) return AppRoutes.splash;
        case RootRoute.login:
          if (currentPath != AppRoutes.login) return AppRoutes.login;
        case RootRoute.projectList:
        case RootRoute.projectListOffline:
          if (currentPath == AppRoutes.splash ||
              currentPath == AppRoutes.login) {
            return AppRoutes.shell;
          }
      }
      return null;
    },
    routes: <RouteBase>[
      GoRoute(
        path: AppRoutes.splash,
        builder: (context, state) => const _SplashScreen(),
      ),
      GoRoute(
        path: AppRoutes.login,
        builder: (context, state) => const LoginPage(),
      ),
      GoRoute(
        path: AppRoutes.shell,
        builder: (context, state) => const AppShellPage(),
      ),
      GoRoute(
        path: AppRoutes.thread,
        builder: (context, state) {
          final sessionId = state.pathParameters['sessionId']!;
          final extra = state.extra as ThreadRouteExtra?;
          return ThreadViewPage(
            sessionId: sessionId,
            agent: extra?.agent,
            agentProfileId: extra?.agentProfileId,
          );
        },
      ),
      GoRoute(
        path: AppRoutes.newThread,
        builder: (context, state) {
          final extra = state.extra as ThreadRouteExtra?;
          return ThreadViewPage(agentProfileId: extra?.agentProfileId);
        },
      ),
      GoRoute(
        path: AppRoutes.agentStart,
        builder: (context, state) => const AgentStartPage(),
      ),
      GoRoute(
        path: AppRoutes.agentProfile,
        builder: (context, state) {
          final profileId = state.pathParameters['profileId']!;
          return AgentProfilePage(profileId: profileId);
        },
      ),
      GoRoute(
        path: AppRoutes.logViewer,
        builder: (context, state) => const LogViewerPage(),
      ),
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
      GoRoute(
        path: AppRoutes.socialHub,
        builder: (context, state) => const SocialHubPage(),
      ),
      GoRoute(
        path: AppRoutes.socialChat,
        builder: (context, state) {
          final conversationId = state.pathParameters['conversationId']!;
          final extra = state.extra as SocialChatRouteExtra?;
          return SocialChatPage(
            conversationId: conversationId,
            title: extra?.title ?? '',
            kind: extra?.kind,
          );
        },
      ),
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

class ThreadRouteExtra {
  const ThreadRouteExtra({this.agent, this.agentProfileId});

  final AgentName? agent;
  final String? agentProfileId;
}

class SocialChatRouteExtra {
  const SocialChatRouteExtra({required this.title, required this.kind});

  final String title;
  final ConversationKind kind;
}

class GroupMembersRouteExtra {
  const GroupMembersRouteExtra({required this.title});

  final String title;
}

class ProjectDetailRouteExtra {
  const ProjectDetailRouteExtra({required this.projectName});

  final String projectName;
}

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

class _SplashScreen extends StatelessWidget {
  const _SplashScreen();

  @override
  Widget build(BuildContext context) {
    return const Scaffold(body: Center(child: CircularProgressIndicator()));
  }
}
