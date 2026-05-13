/// Feature: Social (People & Conversations)
///
/// Friend management, direct messaging, and group conversations.
/// Includes friend search, friend requests, and the social chat surface.
///
/// View Models:
///   - [SocialConversation] (application/social_providers.dart)
///   - [ConversationsController] (application/social_providers.dart)
///   - [FriendRequestsController] (application/social_providers.dart)
///   - [FriendsController] (application/social_providers.dart)
///
/// Views:
///   - [SocialHubPage]
///   - [SocialChatPage]
///   - [GroupMembersPage]
library;

export 'package:minos/application/social_providers.dart';
export 'package:minos/presentation/pages/group_members_page.dart';
export 'package:minos/presentation/pages/social_chat_page.dart';
export 'package:minos/presentation/pages/social_hub_page.dart';
