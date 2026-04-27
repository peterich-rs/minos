import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/thread_events_provider.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/presentation/widgets/chat/input_bar.dart';
import 'package:minos/presentation/widgets/chat/message_bubble.dart';
import 'package:minos/presentation/widgets/chat/reasoning_section.dart';
import 'package:minos/presentation/widgets/chat/streaming_text.dart';
import 'package:minos/presentation/widgets/chat/tool_call_card.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Chat surface for a single thread. Renders the translated
/// `UiEventMessage` stream as a sequence of bubbles + tool-call cards +
/// reasoning sections, with a sticky composer at the bottom.
///
/// The page is the integration seam where:
///
///   - `threadEventsProvider(threadId)` supplies the historical +
///     live event stream;
///   - `activeSessionControllerProvider` says whether the user can
///     compose, must wait, or should see a Stop button instead.
///
/// `threadId == null` means the user just landed on a "new chat" — the
/// list renders empty and the first `onSend` calls `start_agent`. Once
/// the controller transitions to `SessionStreaming` we follow that
/// thread id for events.
class ThreadViewPage extends ConsumerStatefulWidget {
  const ThreadViewPage({super.key, this.threadId});

  /// Pre-existing thread to load. Null = new chat.
  final String? threadId;

  @override
  ConsumerState<ThreadViewPage> createState() => _ThreadViewPageState();
}

class _ThreadViewPageState extends ConsumerState<ThreadViewPage> {
  static const double _stickyThreshold = 120;

  final ScrollController _scroll = ScrollController();
  bool _stickToBottom = true;
  int _unreadBelow = 0;
  int _lastEventCount = 0;

  @override
  void initState() {
    super.initState();
    _scroll.addListener(_onScroll);
  }

  @override
  void dispose() {
    _scroll
      ..removeListener(_onScroll)
      ..dispose();
    super.dispose();
  }

  void _onScroll() {
    if (!_scroll.hasClients) return;
    final pos = _scroll.position;
    final distanceFromBottom = pos.maxScrollExtent - pos.pixels;
    final isAtBottom = distanceFromBottom <= _stickyThreshold;
    if (isAtBottom != _stickToBottom) {
      setState(() {
        _stickToBottom = isAtBottom;
        if (isAtBottom) _unreadBelow = 0;
      });
    }
  }

  void _maybeAutoScroll(int eventCount) {
    if (eventCount == _lastEventCount) return;
    final delta = eventCount - _lastEventCount;
    _lastEventCount = eventCount;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_scroll.hasClients) return;
      if (_stickToBottom) {
        _scroll.jumpTo(_scroll.position.maxScrollExtent);
      } else if (delta > 0) {
        setState(() => _unreadBelow += delta);
      }
    });
  }

  String? _resolvedThreadId(ActiveSession session) {
    return switch (session) {
      SessionStreaming(threadId: final t) => t,
      SessionAwaitingInput(threadId: final t) => t,
      SessionStopped(threadId: final t) => t,
      SessionError(threadId: final t?) => t,
      _ => widget.threadId,
    };
  }

  void _onSend(String text, ActiveSession session) {
    final controller = ref.read(activeSessionControllerProvider.notifier);
    if (session is SessionIdle ||
        session is SessionStopped ||
        session is SessionError) {
      controller.start(agent: AgentName.codex, prompt: text);
    } else {
      controller.send(text);
    }
  }

  @override
  Widget build(BuildContext context) {
    final session = ref.watch(activeSessionControllerProvider);
    final threadId = _resolvedThreadId(session);

    final body = threadId == null
        ? _NewChatEmptyState()
        : _ThreadEventStream(
            threadId: threadId,
            scroll: _scroll,
            stickToBottom: _stickToBottom,
            onEventCountChanged: _maybeAutoScroll,
            unreadBelow: _unreadBelow,
            onJumpToBottom: () {
              if (!_scroll.hasClients) return;
              _scroll.animateTo(
                _scroll.position.maxScrollExtent,
                duration: const Duration(milliseconds: 200),
                curve: Curves.easeOut,
              );
              setState(() {
                _stickToBottom = true;
                _unreadBelow = 0;
              });
            },
          );

    return Scaffold(
      appBar: AppBar(title: Text(threadId == null ? 'New chat' : 'Thread')),
      body: SafeArea(
        bottom: false,
        child: Column(
          children: [
            Expanded(child: body),
            InputBar(
              session: session,
              onSend: (t) => _onSend(t, session),
              onStop: () =>
                  ref.read(activeSessionControllerProvider.notifier).stop(),
            ),
          ],
        ),
      ),
    );
  }
}

class _NewChatEmptyState extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              Icons.forum_outlined,
              size: 48,
              color: theme.colorScheme.onSurfaceVariant,
            ),
            const SizedBox(height: 12),
            Text(
              'Start a new conversation',
              style: theme.textTheme.titleMedium?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              'Type a prompt below — the agent will pick it up.',
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
              textAlign: TextAlign.center,
            ),
          ],
        ),
      ),
    );
  }
}

class _ThreadEventStream extends ConsumerWidget {
  const _ThreadEventStream({
    required this.threadId,
    required this.scroll,
    required this.stickToBottom,
    required this.onEventCountChanged,
    required this.unreadBelow,
    required this.onJumpToBottom,
  });

  final String threadId;
  final ScrollController scroll;
  final bool stickToBottom;
  final ValueChanged<int> onEventCountChanged;
  final int unreadBelow;
  final VoidCallback onJumpToBottom;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final eventsAsync = ref.watch(threadEventsProvider(threadId));
    return eventsAsync.when(
      loading: () => const Center(child: CircularProgressIndicator()),
      error: (e, _) => Center(child: Text('Error: $e')),
      data: (events) {
        onEventCountChanged(events.length);
        if (events.isEmpty) {
          return const Center(child: Text('No messages yet'));
        }
        final groups = _GroupedEvents.from(events);
        return Stack(
          children: [
            ListView.builder(
              controller: scroll,
              padding: const EdgeInsets.symmetric(vertical: 8),
              itemCount: groups.items.length,
              itemBuilder: (_, i) => groups.items[i],
            ),
            if (unreadBelow > 0)
              Positioned(
                right: 16,
                bottom: 16,
                child: FloatingActionButton.small(
                  onPressed: onJumpToBottom,
                  child: Badge.count(
                    count: unreadBelow,
                    child: const Icon(Icons.arrow_downward),
                  ),
                ),
              ),
          ],
        );
      },
    );
  }
}

/// Translates a flat ordered list of `UiEventMessage`s into a list of
/// chat widgets. Event-to-widget mapping (per plan §10.6 step 2):
///
///   - MessageStarted{user}      → opens a user MessageBubble buffer
///   - MessageStarted{assistant} → opens a StreamingText buffer
///   - TextDelta                 → appends to that buffer
///   - MessageCompleted          → flips bubble to non-streaming
///   - ReasoningDelta            → accumulates into a ReasoningSection
///   - ToolCallPlaced            → emits a ToolCallCard (in-flight)
///   - ToolCallCompleted         → mutates the matching card to done
///   - ThreadClosed              → renders a divider
///   - Error                     → renders a destructive bubble
///   - ThreadOpened/Title/Raw    → ignored in chat view (metadata)
class _GroupedEvents {
  _GroupedEvents._(this.items);
  final List<Widget> items;

  factory _GroupedEvents.from(List<UiEventMessage> events) {
    final widgets = <Widget>[];
    // Per-message buffers, keyed by message_id.
    final textByMsg = <String, StringBuffer>{};
    final reasoningByMsg = <String, StringBuffer>{};
    final completedMsgs = <String>{};
    final roleByMsg = <String, MessageRole>{};
    // Tool calls: maintain insertion order so we can render their cards
    // inline. Mutated when ToolCallCompleted lands.
    final toolCalls = <String, _ToolCallEntry>{};
    final toolCallOrder = <String>[];

    for (final e in events) {
      switch (e) {
        case UiEventMessage_MessageStarted(:final messageId, :final role):
          roleByMsg[messageId] = role;
          textByMsg.putIfAbsent(messageId, () => StringBuffer());
        case UiEventMessage_TextDelta(:final messageId, :final text):
          textByMsg.putIfAbsent(messageId, () => StringBuffer()).write(text);
        case UiEventMessage_ReasoningDelta(:final messageId, :final text):
          reasoningByMsg
              .putIfAbsent(messageId, () => StringBuffer())
              .write(text);
        case UiEventMessage_MessageCompleted(:final messageId):
          completedMsgs.add(messageId);
        case UiEventMessage_ToolCallPlaced(
          :final toolCallId,
          :final name,
          :final argsJson,
        ):
          toolCalls[toolCallId] = _ToolCallEntry(name: name, args: argsJson);
          toolCallOrder.add(toolCallId);
        case UiEventMessage_ToolCallCompleted(
          :final toolCallId,
          :final output,
          :final isError,
        ):
          final existing = toolCalls[toolCallId];
          if (existing != null) {
            existing.output = output;
            existing.isError = isError;
          } else {
            // Out-of-order: synthesise a placeholder entry.
            toolCalls[toolCallId] = _ToolCallEntry(
              name: '(unknown)',
              args: '{}',
              output: output,
              isError: isError,
            );
            toolCallOrder.add(toolCallId);
          }
        case UiEventMessage_ThreadClosed():
          widgets.add(const _ClosedDivider());
        case UiEventMessage_Error(:final code, :final message):
          widgets.add(_ErrorBubble(code: code, message: message));
        case UiEventMessage_ThreadOpened():
        case UiEventMessage_ThreadTitleUpdated():
        case UiEventMessage_Raw():
          // Metadata — not surfaced as a chat row.
          break;
      }
    }

    // Render bubbles in role-ordered insertion order. We deliberately walk
    // the events again to preserve message ordering relative to
    // ThreadClosed / Error markers that were already appended above.
    final renderedMessages = <String>{};
    final renderedToolCalls = <String>{};
    final ordered = <Widget>[];
    for (final e in events) {
      final String? msgId = switch (e) {
        UiEventMessage_MessageStarted(:final messageId) => messageId,
        UiEventMessage_TextDelta(:final messageId) => messageId,
        UiEventMessage_ReasoningDelta(:final messageId) => messageId,
        UiEventMessage_MessageCompleted(:final messageId) => messageId,
        _ => null,
      };
      final String? tcId = switch (e) {
        UiEventMessage_ToolCallPlaced(:final toolCallId) => toolCallId,
        UiEventMessage_ToolCallCompleted(:final toolCallId) => toolCallId,
        _ => null,
      };

      if (msgId != null && !renderedMessages.contains(msgId)) {
        renderedMessages.add(msgId);
        final role = roleByMsg[msgId] ?? MessageRole.assistant;
        final text = textByMsg[msgId]?.toString() ?? '';
        final reasoning = reasoningByMsg[msgId]?.toString() ?? '';
        final isComplete = completedMsgs.contains(msgId);
        if (role == MessageRole.user) {
          ordered.add(
            MessageBubble(
              isUser: true,
              markdownContent: text,
              isStreaming: false,
            ),
          );
        } else {
          ordered.add(
            StreamingText(
              messageId: msgId,
              accumulatedText: text,
              isComplete: isComplete,
            ),
          );
        }
        if (reasoning.isNotEmpty) {
          ordered.add(
            ReasoningSection(messageId: msgId, reasoningText: reasoning),
          );
        }
      }

      if (tcId != null && !renderedToolCalls.contains(tcId)) {
        renderedToolCalls.add(tcId);
        final entry = toolCalls[tcId]!;
        ordered.add(
          ToolCallCard(
            toolCallId: tcId,
            toolName: entry.name,
            argsJson: entry.args,
            output: entry.output,
            isError: entry.isError,
          ),
        );
      }
    }

    // Append the trailing markers (ThreadClosed / Error) that we already
    // captured in `widgets`. Order: bubbles → markers.
    return _GroupedEvents._([...ordered, ...widgets]);
  }
}

class _ToolCallEntry {
  _ToolCallEntry({
    required this.name,
    required this.args,
    this.output,
    this.isError = false,
  });
  final String name;
  final String args;
  String? output;
  bool isError;
}

class _ClosedDivider extends StatelessWidget {
  const _ClosedDivider();
  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 12, horizontal: 32),
      child: Row(
        children: [
          Expanded(child: Divider(color: theme.colorScheme.outlineVariant)),
          const SizedBox(width: 8),
          Text(
            'session ended',
            style: theme.textTheme.labelSmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
          const SizedBox(width: 8),
          Expanded(child: Divider(color: theme.colorScheme.outlineVariant)),
        ],
      ),
    );
  }
}

class _ErrorBubble extends StatelessWidget {
  const _ErrorBubble({required this.code, required this.message});
  final String code;
  final String message;
  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      child: Container(
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
          color: theme.colorScheme.errorContainer,
          borderRadius: BorderRadius.circular(10),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(
              Icons.error_outline,
              color: theme.colorScheme.onErrorContainer,
            ),
            const SizedBox(width: 8),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    code,
                    style: theme.textTheme.labelMedium?.copyWith(
                      color: theme.colorScheme.onErrorContainer,
                      fontWeight: FontWeight.bold,
                    ),
                  ),
                  Text(
                    message,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.onErrorContainer,
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
