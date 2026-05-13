/// Data Layer — Repositories
///
/// Repositories consume one or more Services, transform raw API models
/// into clean Domain Models, and handle caching / offline sync / retry.
/// They expose Domain Models to the application (ViewModel) layer.
///
/// Architecture: repositories are the single source of truth for data.
/// ViewModels inject repositories via Riverpod and never call services
/// directly.
library;

export 'package:minos/application/agent_profiles_provider.dart'
    show
        agentProfileStoreProvider,
        agentProfilesControllerProvider,
        AgentProfilesController;
export 'package:minos/application/group_agent_provider.dart'
    show groupAgentBindingsProvider, GroupAgentBindingsController;
export 'package:minos/application/minos_providers.dart';
export 'package:minos/application/project_providers.dart';
export 'package:minos/application/social_providers.dart'
    show
        socialCacheStoreProvider,
        socialProfileProvider,
        friendRequestsProvider,
        friendsProvider,
        conversationsProvider,
        FriendRequestsController,
        FriendsController,
        ConversationsController;
export 'package:minos/application/thread_list_provider.dart';
