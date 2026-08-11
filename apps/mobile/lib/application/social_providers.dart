/// Compatibility barrel for social application providers.
///
/// Prefer importing the focused modules under `application/social/` from new
/// code. This barrel keeps existing call-sites stable after the split.
library;

export 'package:minos/application/social/social_chat_actions.dart';
export 'package:minos/application/social/social_chat_view_model.dart';
export 'package:minos/application/social/social_conversation_notifier.dart';
export 'package:minos/application/social/social_conversation_state.dart';
export 'package:minos/application/social/social_friends_providers.dart';
export 'package:minos/application/social/social_inbox_notifier.dart';
export 'package:minos/application/social/social_realtime_sync.dart';
export 'package:minos/application/social/social_ui_state.dart';
