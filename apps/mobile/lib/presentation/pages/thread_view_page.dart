import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/preferred_agent_provider.dart';
import 'package:minos/application/thread_events_provider.dart';
import 'package:minos/domain/agent_profile.dart';
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
  const ThreadViewPage({
    super.key,
    this.threadId,
    this.agent,
    this.agentProfileId,
  });

  /// Pre-existing thread to load. Null = new chat.
  final String? threadId;

  /// Agent the thread was started with. Set when navigating from the thread
  /// list (we already have it on the [ThreadSummary]); the title falls back
  /// to it whenever the global active session is bound to a *different*
  /// thread, so we never label a historical thread with the live session's
  /// agent.
  final AgentName? agent;
  final String? agentProfileId;

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
    // When the user navigated into a specific thread, that always wins —
    // otherwise tapping an older thread would show the most-recently-started
    // session's events because the active-session controller is global.
    // The session-derived branch is only meant for the "new chat" path,
    // where widget.threadId is null and we need start_agent to mint one.
    if (widget.threadId != null) return widget.threadId;
    return _sessionThreadId(session);
  }

  static String? _sessionThreadId(ActiveSession session) {
    return switch (session) {
      SessionStreaming(threadId: final t) => t,
      SessionAwaitingInput(threadId: final t) => t,
      SessionStopped(threadId: final t) => t,
      SessionError(threadId: final t?) => t,
      _ => null,
    };
  }

  /// Returns the global session if it is currently bound to the thread the
  /// page is rendering, otherwise [SessionIdle]. The whole page (title,
  /// subtitle, input bar, send/start decision) reads off this view-scoped
  /// value so historical threads never inherit the live session's "回复中"
  /// badge or "停止" button while the agent is busy on a different thread.
  ActiveSession _viewSession(ActiveSession session) {
    if (widget.threadId == null) return session;
    return _sessionThreadId(session) == widget.threadId
        ? session
        : const SessionIdle();
  }

  String _enqueueOptimisticMessage(String text) {
    // Snapshot the event count at enqueue time so the bubble can be slotted
    // back into the timeline at the right chronological position even if a
    // codex reply lands before our own MessageStarted{user} echo.
    final anchor =
        ref
            .read(
              threadEventsProvider(
                _resolvedThreadId(ref.read(activeSessionControllerProvider)) ??
                    '',
              ),
            )
            .asData
            ?.value
            .length ??
        0;
    final message = _OptimisticUserMessage(
      id: 'optimistic-${_nextOptimisticMessageId++}',
      text: text,
      status: _OptimisticMessageStatus.pending,
      anchorEventCount: anchor,
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

  /// Daemon RPC ack means the message is durable; clear the spinner now
  /// instead of waiting for a `MessageStarted{role:user}` echo (which the
  /// upstream pipeline does not always deliver in a timely fashion). The
  /// optimistic entry stays in the list as a `confirmed` row until either
  /// the real `MessageStarted{user}` event consumes it, or the user
  /// navigates away.
  void _markOptimisticMessageConfirmed(String id) {
    _clearOptimisticTimer(id);
    _updateOptimisticMessage(
      id,
      (current) => current.copyWith(status: _OptimisticMessageStatus.confirmed),
    );
  }

  void _consumeConfirmedUserMessages(
    String threadId,
    List<_UserMessageEcho> userMessages,
  ) {
    if (_trackedThreadId != threadId) {
      _trackedThreadId = threadId;
    }

    var didRemove = false;
    while (true) {
      final index = _optimisticMessages.indexWhere(
        (message) =>
            message.status == _OptimisticMessageStatus.confirmed &&
            _hasEchoForOptimistic(message, userMessages),
      );
      if (index == -1) break;
      final id = _optimisticMessages[index].id;
      _clearOptimisticTimer(id);
      _optimisticMessages.removeAt(index);
      didRemove = true;
    }
    if (didRemove && mounted) {
      setState(() {});
    }
  }

  void _handleThreadMetrics(
    String threadId,
    int eventCount,
    List<_UserMessageEcho> userMessages,
  ) {
    _consumeConfirmedUserMessages(threadId, userMessages);
    _maybeAutoScroll(eventCount + _optimisticMessages.length);
  }

  Future<void> _dispatchMessage(String text, ActiveSession viewSession) async {
    final optimisticId = _enqueueOptimisticMessage(text);
    final controller = ref.read(activeSessionControllerProvider.notifier);
    final targetThreadId = widget.threadId ?? _sessionThreadId(viewSession);
    final selectedProfile = _dispatchProfile(targetThreadId);
    if (selectedProfile?.hostDeviceId case final hostId?) {
      await ref.read(activeMacProvider.notifier).setActive(hostId);
    }
    MinosError? error;
    if (targetThreadId == null) {
      error = await controller.startAndSend(
        agent:
            selectedProfile?.runtimeAgent ?? ref.read(preferredAgentProvider),
        prompt: text,
      );
    } else {
      error = await controller.sendToThread(
        threadId: targetThreadId,
        agent:
            selectedProfile?.runtimeAgent ??
            widget.agent ??
            _sessionAgent(viewSession) ??
            ref.read(preferredAgentProvider),
        text: text,
      );
    }

    if (!mounted) return;
    if (error != null) {
      _markOptimisticMessageFailed(optimisticId);
    } else {
      _markOptimisticMessageConfirmed(optimisticId);
      final startedThreadId = _sessionThreadId(
        ref.read(activeSessionControllerProvider),
      );
      if (selectedProfile != null && startedThreadId != null) {
        await ref
            .read(agentProfilesControllerProvider.notifier)
            .bindThreadToProfile(
              threadId: startedThreadId,
              profileId: selectedProfile.id,
            );
      }
    }
  }

  AgentProfile? _dispatchProfile(String? threadId) {
    final workspace = ref.read(agentProfilesControllerProvider).asData?.value;
    if (workspace == null) return null;
    if (widget.agentProfileId != null) {
      return workspace.profileById(widget.agentProfileId!);
    }
    if (threadId != null) {
      return workspace.profileForThread(threadId);
    }
    return workspace.preferredProfile;
  }

  void _onSend(String text, ActiveSession viewSession) {
    unawaited(_dispatchMessage(text, viewSession));
  }

  @override
  Widget build(BuildContext context) {
    final session = ref.watch(activeSessionControllerProvider);
    final viewSession = _viewSession(session);
    final threadId = _resolvedThreadId(session);
    final selectedProfile = widget.agentProfileId != null
        ? ref
              .watch(agentProfilesControllerProvider)
              .asData
              ?.value
              .profileById(widget.agentProfileId!)
        : (threadId == null
              ? ref.watch(preferredAgentProfileProvider)
              : ref.watch(threadBoundAgentProfileProvider(threadId)));

    final body = threadId == null && _optimisticMessages.isEmpty
        ? _NewChatEmptyState()
        : threadId == null
        ? _LoadingThreadState(optimisticUserMessages: _optimisticMessages)
        : _ThreadEventStream(
            threadId: threadId,
            optimisticUserMessages: _optimisticMessages,
            scroll: _scroll,
            stickToBottom: _stickToBottom,
            showLiveAssistantState:
                viewSession is SessionStreaming ||
                viewSession is SessionStarting,
            onMetricsChanged: (eventCount, userMessages) =>
                _handleThreadMetrics(threadId, eventCount, userMessages),
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
    final shadTheme = ShadTheme.of(context);
    final scaffoldBg = shadTheme.colorScheme.background;
    final liveAgent = _sessionAgent(viewSession);
    final titleAgent = liveAgent ?? widget.agent;
    final subtitle = _sessionSubtitle(
      viewSession,
      selectedProfile: selectedProfile,
    );

    return Scaffold(
      backgroundColor: scaffoldBg,
      appBar: AppBar(
        backgroundColor: shadTheme.colorScheme.background,
        surfaceTintColor: Colors.transparent,
        scrolledUnderElevation: 0,
        elevation: 0,
        shape: Border(
          bottom: BorderSide(color: shadTheme.colorScheme.border, width: 1),
        ),
        titleSpacing: 0,
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Text(
              threadId == null
                  ? (selectedProfile?.name ?? '新对话')
                  : (selectedProfile?.name ??
                        (titleAgent == null ? '会话' : _agentLabel(titleAgent))),
              style: theme.textTheme.titleMedium?.copyWith(
                fontWeight: FontWeight.w600,
                color: shadTheme.colorScheme.foreground,
              ),
            ),
            if (subtitle != null)
              Text(subtitle, style: shadTheme.textTheme.muted),
          ],
        ),
      ),
      body: SafeArea(
        bottom: false,
        child: Column(
          children: <Widget>[
            Expanded(child: body),
            InputBar(
              session: viewSession,
              onSend: (t) => _onSend(t, viewSession),
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

String? _sessionSubtitle(
  ActiveSession session, {
  AgentProfile? selectedProfile,
}) {
  final profileLabel = selectedProfile == null
      ? null
      : '${selectedProfile.model} · ${_reasoningLabel(selectedProfile.reasoningEffort)}';
  return switch (session) {
    SessionIdle() => profileLabel,
    SessionStarting(:final agent) =>
      '${profileLabel ?? _agentLabel(agent)} 启动中…',
    SessionStreaming(:final agent) =>
      '${profileLabel ?? _agentLabel(agent)} 回复中',
    SessionAwaitingInput(:final agent) =>
      '${profileLabel ?? _agentLabel(agent)} 等待输入',
    SessionStopped() => '已停止',
    SessionError() => '出错',
  };
}

String _reasoningLabel(AgentReasoningEffort effort) {
  return switch (effort) {
    AgentReasoningEffort.low => 'Low',
    AgentReasoningEffort.medium => 'Medium',
    AgentReasoningEffort.high => 'High',
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
    final shadTheme = ShadTheme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Icon(
              LucideIcons.messageCircle,
              size: 44,
              color: shadTheme.colorScheme.mutedForeground,
            ),
            const SizedBox(height: 12),
            Text(
              '开始新对话',
              style: theme.textTheme.titleMedium?.copyWith(
                fontWeight: FontWeight.w600,
                color: shadTheme.colorScheme.foreground,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              '在下方输入消息，Agent 会立刻接管。',
              style: shadTheme.textTheme.muted,
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
    required this.showLiveAssistantState,
    required this.onMetricsChanged,
    required this.unreadBelow,
    required this.onJumpToBottom,
  });

  final String threadId;
  final List<_OptimisticUserMessage> optimisticUserMessages;
  final ScrollController scroll;
  final bool stickToBottom;
  final bool showLiveAssistantState;
  final void Function(int eventCount, List<_UserMessageEcho> userMessages)
  onMetricsChanged;
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
        WidgetsBinding.instance.addPostFrameCallback((_) {
          onMetricsChanged(events.length, _extractUserMessages(events));
        });
        if (events.isEmpty && optimisticUserMessages.isEmpty) {
          return const Center(child: Text('暂无消息'));
        }
        final groups = _GroupedEvents.from(
          events,
          optimistic: optimisticUserMessages,
          showLiveAssistantState: showLiveAssistantState,
        );
        final items = groups.items;
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

  List<_UserMessageEcho> _extractUserMessages(List<UiEventMessage> events) =>
      _extractUserMessageEchoes(events);
}

/// Quiet placeholder shown while the initial `readThread` future resolves
/// or while a brand-new chat is waiting for `start_agent` to mint a thread
/// id. Optimistic bubbles render in their natural top-to-bottom order; we
/// deliberately drop the centered `CircularProgressIndicator` because the
/// thread provider is now `keepAlive: true` and re-entry usually returns a
/// cached event list within a frame, so a big spinner looked jarring.
class _LoadingThreadState extends StatelessWidget {
  const _LoadingThreadState({required this.optimisticUserMessages});

  final List<_OptimisticUserMessage> optimisticUserMessages;

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.symmetric(vertical: 8),
      children: <Widget>[
        for (final message in optimisticUserMessages)
          MessageBubble(
            isUser: true,
            markdownContent: message.text,
            deliveryState: switch (message.status) {
              _OptimisticMessageStatus.sending => MessageDeliveryState.sending,
              _OptimisticMessageStatus.failed => MessageDeliveryState.failed,
              _ => MessageDeliveryState.none,
            },
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
///
/// Optimistic user messages are interleaved by their `anchorEventCount`
/// (events.length at enqueue): each optimistic renders just before the
/// first event whose index >= its anchor. Anything anchored past the end
/// of the events list trails. This keeps a just-sent user bubble above an
/// assistant reply that streams in before our own `MessageStarted{user}`
/// echo arrives.
class _GroupedEvents {
  _GroupedEvents._(this.items);
  final List<Widget> items;

  factory _GroupedEvents.from(
    List<UiEventMessage> events, {
    List<_OptimisticUserMessage> optimistic = const [],
    bool showLiveAssistantState = false,
  }) {
    final widgets = <Widget>[];
    // Per-message buffers, keyed by message_id.
    final textByMsg = <String, StringBuffer>{};
    final reasoningByMsg = <String, StringBuffer>{};
    final reasoningStatusByMsg = <String, MessageBubbleStatusLine>{};
    final toolStatusByMsg = <String, MessageBubbleStatusLine>{};
    final completedMsgs = <String>{};
    final roleByMsg = <String, MessageRole>{};
    // Tool calls: maintain insertion order so we can render their cards
    // inline. Mutated when ToolCallCompleted lands.
    final toolCalls = <String, _ToolCallEntry>{};
    String? lastAssistantMessageId;

    final messageStartIndex = <String, int>{};
    for (var eventIndex = 0; eventIndex < events.length; eventIndex++) {
      final e = events[eventIndex];
      switch (e) {
        case UiEventMessage_MessageStarted(:final messageId, :final role):
          messageStartIndex.putIfAbsent(messageId, () => eventIndex);
          roleByMsg[messageId] = role;
          textByMsg.putIfAbsent(messageId, () => StringBuffer());
          if (role == MessageRole.assistant) {
            lastAssistantMessageId = messageId;
          }
        case UiEventMessage_TextDelta(:final messageId, :final text):
          textByMsg.putIfAbsent(messageId, () => StringBuffer()).write(text);
        case UiEventMessage_ReasoningDelta(:final messageId, :final text):
          reasoningByMsg
              .putIfAbsent(messageId, () => StringBuffer())
              .write(text);
          final preview = _statusPreview(text);
          if (preview != null) {
            reasoningStatusByMsg[messageId] = MessageBubbleStatusLine(
              icon: Icons.psychology_outlined,
              label: '思考中 · $preview',
              tone: MessageBubbleStatusTone.info,
            );
          }
        case UiEventMessage_MessageCompleted(:final messageId):
          completedMsgs.add(messageId);
        case UiEventMessage_ToolCallPlaced(
          :final messageId,
          :final toolCallId,
          :final name,
          :final argsJson,
        ):
          toolCalls[toolCallId] = _ToolCallEntry(
            messageId: messageId,
            name: name,
            args: argsJson,
          );
          toolStatusByMsg[messageId] = MessageBubbleStatusLine(
            icon: Icons.build_outlined,
            label: '调用工具 · $name',
            tone: MessageBubbleStatusTone.info,
          );
        case UiEventMessage_ToolCallCompleted(
          :final toolCallId,
          :final output,
          :final isError,
        ):
          final existing = toolCalls[toolCallId];
          if (existing != null) {
            existing.output = output;
            existing.isError = isError;
            if (existing.messageId.isNotEmpty) {
              toolStatusByMsg[existing.messageId] = MessageBubbleStatusLine(
                icon: isError
                    ? Icons.error_outline
                    : Icons.check_circle_outline,
                label: isError
                    ? '工具失败 · ${existing.name}'
                    : '工具完成 · ${existing.name}',
                tone: isError
                    ? MessageBubbleStatusTone.error
                    : MessageBubbleStatusTone.success,
              );
            }
          } else {
            // Out-of-order: synthesise a placeholder entry.
            toolCalls[toolCallId] = _ToolCallEntry(
              messageId: lastAssistantMessageId ?? '',
              name: '(unknown)',
              args: '{}',
              output: output,
              isError: isError,
            );
            if (lastAssistantMessageId != null) {
              toolStatusByMsg[lastAssistantMessageId] = MessageBubbleStatusLine(
                icon: isError
                    ? Icons.error_outline
                    : Icons.check_circle_outline,
                label: isError ? '工具失败 · 未知工具' : '工具完成 · 未知工具',
                tone: isError
                    ? MessageBubbleStatusTone.error
                    : MessageBubbleStatusTone.success,
              );
            }
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

    final userEchoes = _extractUserMessageEchoes(
      events,
      roleByMsg: roleByMsg,
      textByMsg: textByMsg,
      messageStartIndex: messageStartIndex,
    );

    // Optimistic anchors: render each optimistic bubble before the first
    // event whose index >= its anchor. Sort defensively in case enqueue
    // order ever diverges from anchor monotonicity.
    final pendingOptimistic =
        optimistic
            .where(
              (message) =>
                  message.status != _OptimisticMessageStatus.confirmed ||
                  !_hasEchoForOptimistic(message, userEchoes),
            )
            .toList()
          ..sort((a, b) => a.anchorEventCount.compareTo(b.anchorEventCount));

    Widget optimisticWidget(_OptimisticUserMessage m) => MessageBubble(
      isUser: true,
      markdownContent: m.text,
      deliveryState: switch (m.status) {
        _OptimisticMessageStatus.sending => MessageDeliveryState.sending,
        _OptimisticMessageStatus.failed => MessageDeliveryState.failed,
        _ => MessageDeliveryState.none,
      },
    );

    // Render bubbles in role-ordered insertion order. We deliberately walk
    // the events again to preserve message ordering relative to
    // ThreadClosed / Error markers that were already appended above.
    final renderedMessages = <String>{};
    final renderedToolCalls = <String>{};
    final ordered = <Widget>[];
    var optimisticIdx = 0;
    final liveAssistantMessageId = showLiveAssistantState
        ? lastAssistantMessageId
        : null;
    for (var i = 0; i < events.length; i++) {
      while (optimisticIdx < pendingOptimistic.length &&
          pendingOptimistic[optimisticIdx].anchorEventCount <= i) {
        ordered.add(optimisticWidget(pendingOptimistic[optimisticIdx]));
        optimisticIdx++;
      }
      final e = events[i];
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
        final isLiveAssistantMessage =
            role == MessageRole.assistant &&
            liveAssistantMessageId == msgId &&
            !isComplete;
        if (role == MessageRole.user) {
          // Live fan-out can transiently deliver MessageStarted{user}
          // before the matching TextDelta. Skip that empty interim row so
          // the optimistic bubble doesn't collapse into an air bubble.
          final echo = _UserMessageEcho(
            eventIndex: messageStartIndex[msgId] ?? i,
            text: text,
          );
          if (text.trim().isNotEmpty &&
              !_optimisticSuppressesUserEcho(echo, optimistic)) {
            ordered.add(
              MessageBubble(
                isUser: true,
                markdownContent: text,
                isStreaming: false,
              ),
            );
          }
        } else {
          final statusLines = <MessageBubbleStatusLine>[
            if (isLiveAssistantMessage && reasoningStatusByMsg[msgId] != null)
              reasoningStatusByMsg[msgId]!,
            if (isLiveAssistantMessage && toolStatusByMsg[msgId] != null)
              toolStatusByMsg[msgId]!,
          ];
          ordered.add(
            StreamingText(
              messageId: msgId,
              accumulatedText: text,
              showCursor: isLiveAssistantMessage,
              statusLines: statusLines,
            ),
          );
        }
        if (reasoning.isNotEmpty && !isLiveAssistantMessage) {
          ordered.add(
            ReasoningSection(messageId: msgId, reasoningText: reasoning),
          );
        }
      }

      if (tcId != null && !renderedToolCalls.contains(tcId)) {
        renderedToolCalls.add(tcId);
        final entry = toolCalls[tcId]!;
        final hideDetailedCard =
            showLiveAssistantState &&
            liveAssistantMessageId != null &&
            entry.messageId == liveAssistantMessageId &&
            !completedMsgs.contains(entry.messageId);
        if (!hideDetailedCard) {
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
    }

    // Anything anchored past the end of the events list (no event arrived
    // yet) trails the rendered bubbles.
    while (optimisticIdx < pendingOptimistic.length) {
      ordered.add(optimisticWidget(pendingOptimistic[optimisticIdx]));
      optimisticIdx++;
    }

    // Append the trailing markers (ThreadClosed / Error) that we already
    // captured in `widgets`. Order: bubbles → markers.
    return _GroupedEvents._([...ordered, ...widgets]);
  }
}

class _UserMessageEcho {
  const _UserMessageEcho({required this.eventIndex, required this.text});

  final int eventIndex;
  final String text;

  String get normalizedText => _normalizeMessageText(text);
}

List<_UserMessageEcho> _extractUserMessageEchoes(
  List<UiEventMessage> events, {
  Map<String, MessageRole>? roleByMsg,
  Map<String, StringBuffer>? textByMsg,
  Map<String, int>? messageStartIndex,
}) {
  final roles = roleByMsg ?? <String, MessageRole>{};
  final texts = textByMsg ?? <String, StringBuffer>{};
  final starts = messageStartIndex ?? <String, int>{};

  if (roleByMsg == null || textByMsg == null || messageStartIndex == null) {
    for (var i = 0; i < events.length; i++) {
      switch (events[i]) {
        case UiEventMessage_MessageStarted(:final messageId, :final role):
          starts.putIfAbsent(messageId, () => i);
          roles[messageId] = role;
          texts.putIfAbsent(messageId, () => StringBuffer());
        case UiEventMessage_TextDelta(:final messageId, :final text):
          texts.putIfAbsent(messageId, () => StringBuffer()).write(text);
        default:
          break;
      }
    }
  }

  final echoes = <_UserMessageEcho>[];
  for (final entry in roles.entries) {
    if (entry.value != MessageRole.user) continue;
    final text = texts[entry.key]?.toString() ?? '';
    if (text.trim().isEmpty) continue;
    echoes.add(
      _UserMessageEcho(eventIndex: starts[entry.key] ?? 0, text: text),
    );
  }
  echoes.sort((a, b) => a.eventIndex.compareTo(b.eventIndex));
  return echoes;
}

bool _hasEchoForOptimistic(
  _OptimisticUserMessage message,
  List<_UserMessageEcho> echoes,
) {
  final normalized = _normalizeMessageText(message.text);
  if (normalized.isEmpty) return false;
  return echoes.any(
    (echo) =>
        echo.eventIndex >= message.anchorEventCount &&
        echo.normalizedText == normalized,
  );
}

bool _optimisticSuppressesUserEcho(
  _UserMessageEcho echo,
  List<_OptimisticUserMessage> optimistic,
) {
  return optimistic.any(
    (message) =>
        message.status != _OptimisticMessageStatus.confirmed &&
        echo.eventIndex >= message.anchorEventCount &&
        _normalizeMessageText(message.text) == echo.normalizedText,
  );
}

String _normalizeMessageText(String text) {
  return text.trim().split(RegExp(r'\s+')).where((s) => s.isNotEmpty).join(' ');
}

class _ToolCallEntry {
  _ToolCallEntry({
    required this.messageId,
    required this.name,
    required this.args,
    this.output,
    this.isError = false,
  });
  final String messageId;
  final String name;
  final String args;
  String? output;
  bool isError;
}

String? _statusPreview(String raw) {
  final collapsed = raw.replaceAll(RegExp(r'\s+'), ' ').trim();
  if (collapsed.isEmpty) return null;
  const maxChars = 48;
  if (collapsed.length <= maxChars) return collapsed;
  return '${collapsed.substring(0, maxChars - 1)}…';
}

enum _OptimisticMessageStatus { pending, sending, confirmed, failed }

class _OptimisticUserMessage {
  const _OptimisticUserMessage({
    required this.id,
    required this.text,
    required this.status,
    required this.anchorEventCount,
  });

  final String id;
  final String text;
  final _OptimisticMessageStatus status;

  /// Snapshot of `events.length` at enqueue time. Used at render time to
  /// place the optimistic bubble after the events that already existed and
  /// before any events that arrived afterwards (e.g. an assistant
  /// `MessageStarted` that beat our own `MessageStarted{user}` echo).
  final int anchorEventCount;

  _OptimisticUserMessage copyWith({_OptimisticMessageStatus? status}) {
    return _OptimisticUserMessage(
      id: id,
      text: text,
      status: status ?? this.status,
      anchorEventCount: anchorEventCount,
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
