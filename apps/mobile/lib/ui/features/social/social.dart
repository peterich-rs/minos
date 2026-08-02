/// Feature: Social (People & Conversations)
///
/// Friend management, direct messaging, and group conversations.
/// Collaboration IM uses Slack/Buzz full-width rows (aligned with Desktop).
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
export 'package:minos/ui/features/social/views/group_members_page.dart';
export 'package:minos/ui/features/social/views/social_chat_page.dart';
export 'package:minos/ui/features/social/views/social_hub_page.dart';
export 'package:minos/ui/features/social/widgets/social_management_sections.dart';
