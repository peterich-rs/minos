/// Feature: Social (Conversation IM)
///
/// Collaboration IM: inbox lives in Messages tab; this feature owns the
/// conversation timeline, members, and message chrome (Slack/Buzz full-width).
///
/// View Models:
///   - [SocialConversation] (application/social_providers.dart)
///   - [ConversationsController] (application/social_providers.dart)
///   - [FriendsController] (application/social_providers.dart)
///
/// Views:
///   - [SocialChatPage]
///   - [GroupMembersPage]
library;

export 'package:minos/application/social_providers.dart';
export 'package:minos/ui/features/social/views/group_members_page.dart';
export 'package:minos/ui/features/social/views/social_chat_page.dart';
