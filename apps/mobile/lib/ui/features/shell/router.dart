import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/root_route_decision.dart';
import 'package:minos/src/rust/api/minos.dart' show ConversationKind;
import 'package:minos/ui/features/auth/views/login_page.dart';
import 'package:minos/ui/features/debug/views/log_viewer_page.dart';
import 'package:minos/ui/features/shell/views/app_shell_page.dart';
import 'package:minos/ui/features/social/views/group_members_page.dart';
import 'package:minos/ui/features/social/views/social_chat_page.dart';

/// Route path constants for the IM-first app shell.
abstract final class AppRoutes {
  static const String splash = '/splash';
  static const String login = '/login';
  static const String shell = '/';
  static const String logViewer = '/log-viewer';
  static const String socialHub = '/social';
  static const String socialChat = '/social/chat/:conversationId';
  static const String groupMembers = '/social/chat/:conversationId/members';
}

/// Creates the [GoRouter] instance wired to Riverpod for auth-based redirects.
///
/// Redirect reads only the synchronous [authControllerProvider]. Both
/// [RootRoute.projectList] and [RootRoute.projectListOffline] map to the
/// same shell path, so async connection/pairing providers are not needed
/// here (offline chrome lives inside the shell).
GoRouter createAppRouter(Ref ref) {
  final routerNotifier = _RouterRefreshNotifier(ref);

  return GoRouter(
    initialLocation: AppRoutes.splash,
    refreshListenable: routerNotifier,
    redirect: (context, state) {
      final authState = ref.read(authControllerProvider);
      final route = decideRootRoute(
        authState: authState,
        connectionState: null,
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
        path: AppRoutes.logViewer,
        builder: (context, state) => const LogViewerPage(),
      ),
      // Legacy deep link: conversation inbox is now the shell Messages tab.
      GoRoute(
        path: AppRoutes.socialHub,
        redirect: (context, state) => AppRoutes.shell,
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

class SocialChatRouteExtra {
  const SocialChatRouteExtra({required this.title, required this.kind});

  final String title;
  final ConversationKind kind;
}

class GroupMembersRouteExtra {
  const GroupMembersRouteExtra({required this.title});

  final String title;
}

class _RouterRefreshNotifier extends ChangeNotifier {
  _RouterRefreshNotifier(this._ref) {
    // Auth alone drives root redirects (splash ↔ login ↔ shell).
    _subscription = _ref.listen(
      authControllerProvider,
      (_, _) => notifyListeners(),
    );
  }

  final Ref _ref;
  late final ProviderSubscription<dynamic> _subscription;

  @override
  void dispose() {
    _subscription.close();
    super.dispose();
  }
}

class _SplashScreen extends StatelessWidget {
  const _SplashScreen();

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Theme.of(context).scaffoldBackgroundColor,
      body: const Center(child: CircularProgressIndicator()),
    );
  }
}
