import 'dart:async';

import 'package:flutter/cupertino.dart' hide ConnectionState;
import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/preferred_agent_provider.dart';
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
  static const Duration _sendStatusDelay = Duration(milliseconds: 500);

  final ScrollController _scroll = ScrollController();
  final List<_OptimisticUserMessage> _optimisticMessages =
      <_OptimisticUserMessage>[];
  final Map<String, Timer> _optimisticTimers = <String, Timer>{};
  bool _stickToBottom = true;
  int _unreadBelow = 0;
  int _lastEventCount = 0;
  int _nextOptimisticMessageId = 0;
  int _seenUserMessageCount = 0;
  String? _trackedThreadId;

  @override
  void initState() {
    super.initState();
    _scroll.addListener(_onScroll);
  }

  @override
  void dispose() {
    for (final timer in _optimisticTimers.values) {
      timer.cancel();
    }
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

  String _enqueueOptimisticMessage(String text) {
    final message = _OptimisticUserMessage(
      id: 'optimistic-${_nextOptimisticMessageId++}',
      text: text,
      status: _OptimisticMessageStatus.pending,
    );
    setState(() => _optimisticMessages.add(message));
    _optimisticTimers[message.id] = Timer(_sendStatusDelay, () {
      if (!mounted) return;
      _updateOptimisticMessage(
        message.id,
        (current) => current.status == _OptimisticMessageStatus.pending
            ? current.copyWith(status: _OptimisticMessageStatus.sending)
            : current,
      );
    });
    return message.id;
  }

  void _clearOptimisticTimer(String id) {
    _optimisticTimers.remove(id)?.cancel();
  }

  void _updateOptimisticMessage(
    String id,
    _OptimisticUserMessage Function(_OptimisticUserMessage current) transform,
  ) {
    final index = _optimisticMessages.indexWhere((message) => message.id == id);
    if (index == -1) return;
    setState(() {
      _optimisticMessages[index] = transform(_optimisticMessages[index]);
    });
  }

  void _markOptimisticMessageFailed(String id) {
    _clearOptimisticTimer(id);
    _updateOptimisticMessage(
      id,
      (current) => current.copyWith(status: _OptimisticMessageStatus.failed),
    );
  }

  void _consumeConfirmedUserMessages(String threadId, int userMessageCount) {
    if (_trackedThreadId != threadId) {
      _trackedThreadId = threadId;
      _seenUserMessageCount = 0;
    }
    final confirmedDelta = userMessageCount - _seenUserMessageCount;
    if (confirmedDelta <= 0) return;

    var remaining = confirmedDelta;
    var didRemove = false;
    while (remaining > 0) {
      final index = _optimisticMessages.indexWhere(
        (message) => message.status != _OptimisticMessageStatus.failed,
      );
      if (index == -1) break;
      final id = _optimisticMessages[index].id;
      _clearOptimisticTimer(id);
      _optimisticMessages.removeAt(index);
      remaining -= 1;
      didRemove = true;
    }
    _seenUserMessageCount = userMessageCount;
    if (didRemove && mounted) {
      setState(() {});
    }
  }

  void _handleThreadMetrics(
    String threadId,
    int eventCount,
    int userMessageCount,
  ) {
    _consumeConfirmedUserMessages(threadId, userMessageCount);
    _maybeAutoScroll(eventCount + _optimisticMessages.length);
  }

  Future<void> _dispatchMessage(String text, ActiveSession session) async {
    final optimisticId = _enqueueOptimisticMessage(text);
    final controller = ref.read(activeSessionControllerProvider.notifier);
    final shouldStart =
        session is SessionIdle ||
        session is SessionStopped ||
        session is SessionError;
    MinosError? error;
    if (shouldStart) {
      error = await controller.start(
        agent: ref.read(preferredAgentProvider),
        prompt: text,
      );
      error ??= await controller.send(text);
    } else {
      error = await controller.send(text);
    }

    if (!mounted) return;
    if (error != null) {
      _markOptimisticMessageFailed(optimisticId);
    }
  }

  void _onSend(String text, ActiveSession session) {
    unawaited(_dispatchMessage(text, session));
  }

  @override
  Widget build(BuildContext context) {
    final session = ref.watch(activeSessionControllerProvider);
    final threadId = _resolvedThreadId(session);

    final body = threadId == null && _optimisticMessages.isEmpty
        ? _NewChatEmptyState()
        : threadId == null
        ? _LoadingThreadState(optimisticUserMessages: _optimisticMessages)
        : _ThreadEventStream(
            threadId: threadId,
            optimisticUserMessages: _optimisticMessages,
            scroll: _scroll,
            stickToBottom: _stickToBottom,
            onMetricsChanged: (eventCount, userMessageCount) =>
                _handleThreadMetrics(threadId, eventCount, userMessageCount),
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

    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    final scaffoldBg = isDark
        ? const Color(0xFF000000)
        : const Color(0xFFF2F2F7);
    final agent = _sessionAgent(session);
    final subtitle = _sessionSubtitle(session);

    return Scaffold(
      backgroundColor: scaffoldBg,
      appBar: AppBar(
        backgroundColor: theme.colorScheme.surface,
        surfaceTintColor: Colors.transparent,
        scrolledUnderElevation: 0,
        elevation: 0,
        shape: Border(
          bottom: BorderSide(
            color: theme.dividerColor.withValues(alpha: 0.4),
            width: 0.5,
          ),
        ),
        titleSpacing: 0,
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Text(
              threadId == null
                  ? '新对话'
                  : (agent == null ? '会话' : _agentLabel(agent)),
              style: theme.textTheme.titleMedium?.copyWith(
                fontWeight: FontWeight.w600,
              ),
            ),
            if (subtitle != null)
              Text(
                subtitle,
                style: theme.textTheme.labelSmall?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
          ],
        ),
      ),
      body: SafeArea(
        bottom: false,
        child: Column(
          children: <Widget>[
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

AgentName? _sessionAgent(ActiveSession session) {
  return switch (session) {
    SessionStarting(:final agent) => agent,
    SessionStreaming(:final agent) => agent,
    SessionAwaitingInput(:final agent) => agent,
    _ => null,
  };
}

String? _sessionSubtitle(ActiveSession session) {
  return switch (session) {
    SessionIdle() => null,
    SessionStarting(:final agent) => '${_agentLabel(agent)} 启动中…',
    SessionStreaming(:final agent) => '${_agentLabel(agent)} 回复中',
    SessionAwaitingInput(:final agent) => '${_agentLabel(agent)} 等待输入',
    SessionStopped() => '已停止',
    SessionError() => '出错',
  };
}

String _agentLabel(AgentName agent) {
  return switch (agent) {
    AgentName.codex => 'Codex',
    AgentName.claude => 'Claude',
    AgentName.gemini => 'Gemini',
  };
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
          children: <Widget>[
            Icon(
              CupertinoIcons.bubble_left_bubble_right,
              size: 44,
              color: theme.colorScheme.outline,
            ),
            const SizedBox(height: 12),
            Text(
              '开始新对话',
              style: theme.textTheme.titleMedium?.copyWith(
                fontWeight: FontWeight.w600,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              '在下方输入消息，Agent 会立刻接管。',
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
    required this.optimisticUserMessages,
    required this.scroll,
    required this.stickToBottom,
    required this.onMetricsChanged,
    required this.unreadBelow,
    required this.onJumpToBottom,
  });

  final String threadId;
  final List<_OptimisticUserMessage> optimisticUserMessages;
  final ScrollController scroll;
  final bool stickToBottom;
  final void Function(int eventCount, int userMessageCount) onMetricsChanged;
  final int unreadBelow;
  final VoidCallback onJumpToBottom;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final eventsAsync = ref.watch(threadEventsProvider(threadId));
    return eventsAsync.when(
      loading: () =>
          _LoadingThreadState(optimisticUserMessages: optimisticUserMessages),
      error: (e, _) => Center(child: Text('加载失败: $e')),
      data: (events) {
        onMetricsChanged(events.length, _countUserMessages(events));
        if (events.isEmpty && optimisticUserMessages.isEmpty) {
          return const Center(child: Text('暂无消息'));
        }
        final groups = _GroupedEvents.from(events);
        final items = <Widget>[
          ...groups.items,
          ...optimisticUserMessages.map(_optimisticBubble),
        ];
        return Stack(
          children: [
            ListView.builder(
              controller: scroll,
              padding: const EdgeInsets.symmetric(vertical: 8),
              itemCount: items.length,
              itemBuilder: (_, i) => items[i],
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

  int _countUserMessages(List<UiEventMessage> events) {
    return events
        .where(
          (event) =>
              event is UiEventMessage_MessageStarted &&
              event.role == MessageRole.user,
        )
        .length;
  }

  Widget _optimisticBubble(_OptimisticUserMessage message) {
    return MessageBubble(
      isUser: true,
      markdownContent: message.text,
      deliveryState: switch (message.status) {
        _OptimisticMessageStatus.sending => MessageDeliveryState.sending,
        _OptimisticMessageStatus.failed => MessageDeliveryState.failed,
        _ => MessageDeliveryState.none,
      },
    );
  }
}

class _LoadingThreadState extends StatelessWidget {
  const _LoadingThreadState({required this.optimisticUserMessages});

  final List<_OptimisticUserMessage> optimisticUserMessages;

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.symmetric(vertical: 8),
      children: <Widget>[
        ...optimisticUserMessages.map(
          (message) => MessageBubble(
            isUser: true,
            markdownContent: message.text,
            deliveryState: switch (message.status) {
              _OptimisticMessageStatus.sending => MessageDeliveryState.sending,
              _OptimisticMessageStatus.failed => MessageDeliveryState.failed,
              _ => MessageDeliveryState.none,
            },
          ),
        ),
        const Padding(
          padding: EdgeInsets.symmetric(vertical: 24),
          child: Center(child: CircularProgressIndicator()),
        ),
      ],
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

enum _OptimisticMessageStatus { pending, sending, failed }

class _OptimisticUserMessage {
  const _OptimisticUserMessage({
    required this.id,
    required this.text,
    required this.status,
  });

  final String id;
  final String text;
  final _OptimisticMessageStatus status;

  _OptimisticUserMessage copyWith({_OptimisticMessageStatus? status}) {
    return _OptimisticUserMessage(
      id: id,
      text: text,
      status: status ?? this.status,
    );
  }
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
