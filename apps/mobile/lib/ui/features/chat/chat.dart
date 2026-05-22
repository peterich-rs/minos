/// Feature: Chat (Agent Thread View)
///
/// Real-time chat surface for agent threads. Renders translated
/// UiEventMessage streams as message bubbles, tool-call cards, and
/// reasoning sections with a sticky composer.
///
/// View Models:
///   - [ActiveSessionController] (application/active_session_provider.dart)
///   - [ThreadEvents] (application/thread_events_provider.dart)
///
/// Views:
///   - [ThreadViewPage]
///   - [InputBar], [MessageBubble], [StreamingText], [ToolCallCard],
///     [ReasoningSection]
library;

export 'package:minos/application/active_session_provider.dart';
export 'package:minos/application/thread_events_provider.dart';
export 'package:minos/ui/features/chat/views/thread_view_page.dart';
export 'package:minos/ui/features/chat/widgets/input_bar.dart';
export 'package:minos/ui/features/chat/widgets/message_bubble.dart';
export 'package:minos/ui/features/chat/widgets/message_meta_row.dart';
export 'package:minos/ui/features/chat/widgets/reasoning_section.dart';
export 'package:minos/ui/features/chat/widgets/streaming_text.dart';
export 'package:minos/ui/features/chat/widgets/tool_call_card.dart';
