import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/display_payload_preview.dart';
import 'package:minos/application/flutter_log.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/preferred_agent_provider.dart';
import 'package:minos/application/thread_commands.dart';
import 'package:minos/application/thread_events_provider.dart';
import 'package:minos/application/thread_view_state.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/agent_question_sheet.dart';
import 'package:minos/ui/core/widgets/approval_sheet.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/features/chat/widgets/input_bar.dart';
import 'package:minos/ui/features/chat/widgets/message_bubble.dart';
import 'package:minos/ui/features/chat/widgets/reasoning_section.dart';
import 'package:minos/ui/features/chat/widgets/streaming_text.dart';
import 'package:minos/ui/features/chat/widgets/tool_call_card.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

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
/// list renders empty and the first `onSend` dispatches via
/// `sendChatMessage`. Once the controller transitions to
/// `SessionStreaming` we follow that thread id for events.
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
  static const Duration _keyboardRevealDelay = Duration(milliseconds: 120);
  static const Duration _keyboardRevealSettleDelay = Duration(
    milliseconds: 260,
  );

  final ScrollController _scroll = ScrollController();
  StreamSubscription<UiEventFrame>? _approvalSub;
  late final String _viewStateId;
  double _lastKeyboardInsetBottom = 0;

  @override
  void initState() {
    super.initState();
    _viewStateId = widget.threadId ?? 'draft-${identityHashCode(this)}';
    _scroll.addListener(_onScroll);
    _listenForApprovalEvents();
  }

  @override
  void dispose() {
    unawaited(_approvalSub?.cancel());
    _scroll
      ..removeListener(_onScroll)
      ..dispose();
    super.dispose();
  }

  /// Subscribe to the live UI event stream and intercept approval-related
  /// events for the current thread. Approval requests trigger the bottom
  /// sheet; approval timeouts dismiss it and show a toast.
  void _listenForApprovalEvents() {
    final threadCommands = ref.read(threadCommandsProvider);
    _approvalSub = threadCommands.uiEvents.listen((frame) {
      if (!mounted) return;
      final session = ref.read(activeSessionControllerProvider);
      final threadId = _resolvedThreadId(session);
      if (threadId == null || frame.threadId != threadId) return;

      final event = frame.ui;
      if (event is UiEventMessage_Raw) {
        _handleRawApprovalEvent(event, threadId);
      }
    });
  }

  void _handleRawApprovalEvent(UiEventMessage_Raw event, String threadId) {
    if (event.kind == 'approval/request' || event.kind == 'approval_request') {
      unawaited(_onApprovalRequest(event.payloadJson, threadId));
    } else if (event.kind == 'approval/timeout' ||
        event.kind == 'approval_timeout') {
      _onApprovalTimeout(event.payloadJson);
    } else if (event.kind == 'opencode/question.asked') {
      unawaited(_onAgentQuestionRequest(event.payloadJson, threadId));
    } else if (event.kind == 'opencode/question.replied' ||
        event.kind == 'opencode/question.rejected') {
      _onAgentQuestionResolved();
    }
  }

  Future<void> _onApprovalRequest(String payloadJson, String threadId) async {
    final threadViewState = ref.read(
      threadViewStateControllerProvider(_viewStateId),
    );
    if (threadViewState.approvalSheetVisible) return;

    final Map<String, dynamic> json;
    try {
      json = jsonDecode(payloadJson) as Map<String, dynamic>;
    } catch (error, stackTrace) {
      logFlutterWarn(
        'thread_view',
        'approval request decode failed threadId=$threadId',
        error: error,
        stackTrace: stackTrace,
      );
      return;
    }

    final request = ApprovalRequestData.fromJson(json);
    logFlutterInfo(
      'thread_view',
      'approval request received threadId=$threadId requestId=${request.requestId} method=${request.method}',
    );
    ref
        .read(threadViewStateControllerProvider(_viewStateId).notifier)
        .setApprovalSheetVisible(true);

    final decision = await showApprovalSheet(context, request: request);
    ref
        .read(threadViewStateControllerProvider(_viewStateId).notifier)
        .setApprovalSheetVisible(false);

    if (decision == null) {
      logFlutterInfo(
        'thread_view',
        'approval request dismissed threadId=$threadId requestId=${request.requestId}',
      );
      // Timeout or dismissed without decision — host auto-declines
      return;
    }

    final decisionPayload = _buildDecisionPayload(request.method, decision);

    try {
      await ref
          .read(threadCommandsProvider)
          .sendApprovalDecision(
            requestId: request.requestId,
            threadId: request.threadId,
            decision: decisionPayload,
          );
    } catch (error) {
      if (mounted) {
        showLoggedErrorToast(
          context,
          target: 'thread_view',
          title: '发送审批决定失败',
          error: error,
        );
      }
    }
  }

  void _onApprovalTimeout(String payloadJson) {
    // Dismiss the approval sheet if it's currently showing
    final controller = ref.read(
      threadViewStateControllerProvider(_viewStateId).notifier,
    );
    if (ref
            .read(threadViewStateControllerProvider(_viewStateId))
            .approvalSheetVisible &&
        mounted) {
      Navigator.of(context).pop(null);
      controller.setApprovalSheetVisible(false);
    }

    // Show a toast indicating the approval timed out
    if (mounted) {
      final Map<String, dynamic> json;
      try {
        json = jsonDecode(payloadJson) as Map<String, dynamic>;
      } catch (_) {
        logFlutterWarn('thread_view', 'approval timeout payload decode failed');
        _showThreadInfo(context, '审批已超时，自动拒绝');
        return;
      }
      final reason = json['reason'] as String? ?? 'timeout';
      logFlutterInfo('thread_view', 'approval timed out reason=$reason');
      final message = reason == 'disconnected' ? '连接断开，审批已自动拒绝' : '审批已超时，自动拒绝';
      _showThreadInfo(context, message);
    }
  }

  Future<void> _onAgentQuestionRequest(
    String payloadJson,
    String threadId,
  ) async {
    final threadViewState = ref.read(
      threadViewStateControllerProvider(_viewStateId),
    );
    if (threadViewState.approvalSheetVisible) return;

    final Map<String, dynamic> json;
    try {
      json = jsonDecode(payloadJson) as Map<String, dynamic>;
    } catch (error, stackTrace) {
      logFlutterWarn(
        'thread_view',
        'agent question decode failed threadId=$threadId',
        error: error,
        stackTrace: stackTrace,
      );
      return;
    }

    final request = AgentQuestionRequestData.fromJson(json);
    if (request.requestId.isEmpty || request.questions.isEmpty) {
      logFlutterWarn(
        'thread_view',
        'agent question payload missing request id or questions threadId=$threadId',
      );
      return;
    }

    ref
        .read(threadViewStateControllerProvider(_viewStateId).notifier)
        .setApprovalSheetVisible(true);
    final answers = await showAgentQuestionSheet(context, request: request);
    if (!mounted) return;
    ref
        .read(threadViewStateControllerProvider(_viewStateId).notifier)
        .setApprovalSheetVisible(false);

    if (answers == null) return;

    try {
      await ref
          .read(threadCommandsProvider)
          .respondOpencodeQuestion(
            sessionId: threadId,
            questionId: request.requestId,
            answers: answers,
          );
    } catch (error) {
      if (mounted) {
        showLoggedErrorToast(
          context,
          target: 'thread_view',
          title: '发送问题答案失败',
          error: error,
        );
      }
    }
  }

  void _onAgentQuestionResolved() {
    final controller = ref.read(
      threadViewStateControllerProvider(_viewStateId).notifier,
    );
    if (ref
            .read(threadViewStateControllerProvider(_viewStateId))
            .approvalSheetVisible &&
        mounted) {
      Navigator.of(context).pop(null);
      controller.setApprovalSheetVisible(false);
    }
  }

  Map<String, dynamic> _buildDecisionPayload(
    String method,
    ApprovalDecision decision,
  ) {
    final accept = decision == ApprovalDecision.accept;
    if (method.contains('command_execution') ||
        method.contains('exec_command')) {
      return {'decision': accept ? 'accept' : 'decline'};
    } else if (method.contains('file_change') ||
        method.contains('apply_patch')) {
      return {'decision': accept ? 'accept' : 'decline'};
    } else if (method.contains('permissions')) {
      if (accept) {
        return <String, dynamic>{
          'granted': <String, dynamic>{
            'profile': 'default',
            'scope': 'session',
          },
        };
      } else {
        return <String, dynamic>{'denied': <String, dynamic>{}};
      }
    }
    return {'decision': accept ? 'accept' : 'decline'};
  }

  void _onScroll() {
    if (!_scroll.hasClients) return;
    final pos = _scroll.position;
    final distanceFromBottom = pos.maxScrollExtent - pos.pixels;
    ref
        .read(threadViewStateControllerProvider(_viewStateId).notifier)
        .updateScrollState(
          distanceFromBottom: distanceFromBottom,
          stickyThreshold: _stickyThreshold,
        );
  }

  String? _resolvedThreadId(ActiveSession session) {
    // When the user navigated into a specific thread, that always wins —
    // otherwise tapping an older thread would show the most-recently-started
    // session's events because the active-session controller is global.
    // The session-derived branch is only meant for the "new chat" path,
    // where widget.threadId is null and we need sendChatMessage to mint one.
    if (widget.threadId != null) return widget.threadId;
    return _sessionThreadId(session);
  }

  static String? _sessionThreadId(ActiveSession session) {
    return switch (session) {
      SessionStreaming(threadId: final t) => t,
      SessionAwaitingInput(threadId: final t) => t,
      SessionSuspended(threadId: final t) => t,
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
    return ref
        .read(threadViewStateControllerProvider(_viewStateId).notifier)
        .enqueueOptimisticMessage(text: text, anchorEventCount: anchor);
  }

  void _markOptimisticMessageFailed(String id) {
    ref
        .read(threadViewStateControllerProvider(_viewStateId).notifier)
        .markOptimisticMessageFailed(id);
  }

  /// Daemon RPC ack means the message is durable; clear the spinner now
  /// instead of waiting for a `MessageStarted{role:user}` echo (which the
  /// upstream pipeline does not always deliver in a timely fashion). The
  /// optimistic entry stays in the list as a `confirmed` row until either
  /// the real `MessageStarted{user}` event consumes it, or the user
  /// navigates away.
  void _markOptimisticMessageConfirmed(String id) {
    ref
        .read(threadViewStateControllerProvider(_viewStateId).notifier)
        .markOptimisticMessageConfirmed(id);
  }

  void _handleThreadMetrics(
    String threadId,
    int eventCount,
    List<ThreadUserMessageEcho> userMessages,
  ) {
    final shouldAutoScroll = ref
        .read(threadViewStateControllerProvider(_viewStateId).notifier)
        .handleThreadMetrics(
          eventCount: eventCount,
          userMessages: userMessages,
        );
    if (!shouldAutoScroll) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_scroll.hasClients) return;
      _scroll.jumpTo(_scroll.position.maxScrollExtent);
    });
  }

  void _syncKeyboardInset(double keyboardInsetBottom, {required bool stick}) {
    if ((_lastKeyboardInsetBottom - keyboardInsetBottom).abs() < 0.5) return;
    _lastKeyboardInsetBottom = keyboardInsetBottom;
    _revealBottomMessages(stick: stick);
  }

  void _revealBottomMessages({required bool stick}) {
    if (!stick) return;

    void jumpToBottom() {
      if (!mounted || !_scroll.hasClients) return;
      _scroll.jumpTo(_scroll.position.maxScrollExtent);
    }

    WidgetsBinding.instance.addPostFrameCallback((_) => jumpToBottom());
    unawaited(Future<void>.delayed(_keyboardRevealDelay, jumpToBottom));
    unawaited(Future<void>.delayed(_keyboardRevealSettleDelay, jumpToBottom));
  }

  Future<void> _dispatchMessage(String text, ActiveSession viewSession) async {
    final controller = ref.read(activeSessionControllerProvider.notifier);
    final targetThreadId = widget.threadId ?? _sessionThreadId(viewSession);
    final selectedProfile = _dispatchProfile(targetThreadId);
    // AgentProfile selection: for new chats (no thread yet), use the
    // preferred agent from the profile or global preference. For existing
    // threads, fall back through profile → widget.agent → session agent →
    // global preference.
    final dispatchAgent =
        selectedProfile?.runtimeAgent ??
        widget.agent ??
        _sessionAgent(viewSession) ??
        ref.read(preferredAgentProvider);
    if (dispatchAgent == null) {
      _showThreadInfo(context, '请先选择一个 Agent');
      return;
    }

    final optimisticId = _enqueueOptimisticMessage(text);
    if (selectedProfile?.hostDeviceId case final hostId?) {
      await ref.read(activeMacProvider.notifier).setActive(hostId);
    }

    // Unified send path: all messages go through sendChatMessage (via
    // sendUserMessage on the wire). The server handles session creation
    // when sessionId is empty and state-based dispatch (turn/start vs
    // turn/steer) when a session already exists.
    final sessionId = targetThreadId ?? '';
    final error = await controller.send(
      agent: dispatchAgent,
      text: text,
      dispatch: () => ref
          .read(threadCommandsProvider)
          .sendUserMessage(sessionId: sessionId, text: text),
    );

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
    final profiles = ref.read(agentProfilesControllerProvider).asData?.value;
    if (profiles == null) return null;
    if (widget.agentProfileId != null) {
      return profiles.profileById(widget.agentProfileId!);
    }
    if (threadId != null) {
      return profiles.profileForThread(threadId);
    }
    return profiles.preferredProfile;
  }

  void _onSend(String text, ActiveSession viewSession) {
    unawaited(_dispatchMessage(text, viewSession));
  }

  @override
  Widget build(BuildContext context) {
    final session = ref.watch(activeSessionControllerProvider);
    final threadViewState = ref.watch(
      threadViewStateControllerProvider(_viewStateId),
    );
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

    final body = threadId == null && threadViewState.optimisticMessages.isEmpty
        ? _NewChatEmptyState()
        : threadId == null
        ? _LoadingThreadState(
            optimisticUserMessages: threadViewState.optimisticMessages,
          )
        : _ThreadEventStream(
            threadId: threadId,
            optimisticUserMessages: threadViewState.optimisticMessages,
            scroll: _scroll,
            stickToBottom: threadViewState.stickToBottom,
            showLiveAssistantState:
                viewSession is SessionStreaming ||
                viewSession is SessionSending,
            onMetricsChanged: (eventCount, userMessages) =>
                _handleThreadMetrics(threadId, eventCount, userMessages),
            unreadBelow: threadViewState.unreadBelow,
            onJumpToBottom: () {
              if (!_scroll.hasClients) return;
              unawaited(
                _scroll.animateTo(
                  _scroll.position.maxScrollExtent,
                  duration: const Duration(milliseconds: 200),
                  curve: Curves.easeOut,
                ),
              );
              ref
                  .read(
                    threadViewStateControllerProvider(_viewStateId).notifier,
                  )
                  .jumpToBottom();
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
    final keyboardInsetBottom = MediaQuery.of(context).viewInsets.bottom;
    _syncKeyboardInset(
      keyboardInsetBottom,
      stick: threadViewState.stickToBottom,
    );

    return Scaffold(
      resizeToAvoidBottomInset: false,
      backgroundColor: scaffoldBg,
      appBar: AppBar(
        backgroundColor: shadTheme.colorScheme.background,
        surfaceTintColor: Colors.transparent,
        scrolledUnderElevation: 0,
        elevation: 0,
        shape: Border(
          bottom: BorderSide(color: shadTheme.colorScheme.border, width: 1),
        ),
        centerTitle: true,
        titleSpacing: 0,
        title: SizedBox(
          width: double.infinity,
          child: Column(
            crossAxisAlignment: .center,
            mainAxisSize: .min,
            children: <Widget>[
              Text(
                threadId == null
                    ? (selectedProfile?.name ?? '新对话')
                    : (selectedProfile?.name ??
                          (titleAgent == null
                              ? '会话'
                              : _agentLabel(titleAgent))),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.center,
                style: theme.textTheme.titleMedium?.copyWith(
                  fontWeight: FontWeight.w600,
                  color: shadTheme.colorScheme.foreground,
                ),
              ),
              if (subtitle != null)
                Text(
                  subtitle,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  textAlign: TextAlign.center,
                  style: shadTheme.textTheme.muted,
                ),
            ],
          ),
        ),
      ),
      body: SafeArea(
        bottom: false,
        child: Column(
          children: <Widget>[
            Expanded(child: body),
            AnimatedPadding(
              duration: const Duration(milliseconds: 220),
              curve: Curves.easeOutCubic,
              padding: EdgeInsets.only(bottom: keyboardInsetBottom),
              child: InputBar(
                session: viewSession,
                onSend: (t) => _onSend(t, viewSession),
                onStop: () =>
                    ref.read(activeSessionControllerProvider.notifier).stop(),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

AgentName? _sessionAgent(ActiveSession session) {
  return switch (session) {
    SessionSending(:final agent) => agent,
    SessionStreaming(:final agent) => agent,
    SessionAwaitingInput(:final agent) => agent,
    SessionSuspended(:final agent) => agent,
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
    SessionSending(:final agent) =>
      '${profileLabel ?? _agentLabel(agent)} 发送中…',
    SessionStreaming(:final agent) =>
      '${profileLabel ?? _agentLabel(agent)} 回复中',
    SessionAwaitingInput(:final agent) =>
      '${profileLabel ?? _agentLabel(agent)} 等待输入',
    SessionSuspended() => '已暂停',
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
        padding: const .all(24),
        child: Column(
          mainAxisSize: .min,
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
                fontWeight: .w600,
                color: shadTheme.colorScheme.foreground,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              '在下方输入消息，Agent 会立刻接管。',
              style: shadTheme.textTheme.muted,
              textAlign: .center,
            ),
          ],
        ),
      ),
    );
  }
}

void _showThreadInfo(BuildContext context, String title) {
  ShadToaster.maybeOf(context)?.show(ShadToast(title: Text(title)));
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
  final List<ThreadOptimisticUserMessage> optimisticUserMessages;
  final ScrollController scroll;
  final bool stickToBottom;
  final bool showLiveAssistantState;
  final void Function(int eventCount, List<ThreadUserMessageEcho> userMessages)
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
              padding: const .symmetric(vertical: 8),
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

  List<ThreadUserMessageEcho> _extractUserMessages(
    List<UiEventMessage> events,
  ) => _extractUserMessageEchoes(events);
}

/// Quiet placeholder shown while the initial `readThread` future resolves
/// or while a brand-new chat is waiting for `sendChatMessage` to mint a thread
/// id. Optimistic bubbles render in their natural top-to-bottom order; we
/// deliberately drop the centered `CircularProgressIndicator` because the
/// thread provider is now `keepAlive: true` and re-entry usually returns a
/// cached event list within a frame, so a big spinner looked jarring.
class _LoadingThreadState extends StatelessWidget {
  const _LoadingThreadState({required this.optimisticUserMessages});

  final List<ThreadOptimisticUserMessage> optimisticUserMessages;

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
              ThreadOptimisticMessageStatus.sending =>
                MessageDeliveryState.sending,
              ThreadOptimisticMessageStatus.failed =>
                MessageDeliveryState.failed,
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
///   - TextDelta/TextReplace     → appends to or replaces that buffer
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
    List<ThreadOptimisticUserMessage> optimistic =
        const <ThreadOptimisticUserMessage>[],
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
          textByMsg.putIfAbsent(messageId, StringBuffer.new);
          if (role == MessageRole.assistant) {
            lastAssistantMessageId = messageId;
          }
        case UiEventMessage_TextDelta(:final messageId, :final text):
          textByMsg
              .putIfAbsent(messageId, StringBuffer.new)
              .write(text.renderPreview());
        case UiEventMessage_TextReplace(:final messageId, :final text):
          textByMsg[messageId] = StringBuffer(text.renderPreview());
        case UiEventMessage_ReasoningDelta(:final messageId, :final text):
          reasoningByMsg
              .putIfAbsent(messageId, StringBuffer.new)
              .write(text.renderPreview());
          final preview = _statusPreview(text.renderPreview());
          if (preview != null) {
            reasoningStatusByMsg[messageId] = MessageBubbleStatusLine(
              icon: Icons.psychology_outlined,
              label: '思考中 · $preview',
              tone: MessageBubbleStatusTone.info,
            );
          }
        case UiEventMessage_ReasoningReplace(:final messageId, :final text):
          reasoningByMsg[messageId] = StringBuffer(text.renderPreview());
          final preview = _statusPreview(text.renderPreview());
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
            args: argsJson.renderPreview(),
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
            existing.output = output.renderPreview();
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
              output: output.renderPreview(),
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
            .where((message) => !threadOptimisticHasEcho(message, userEchoes))
            .toList()
          ..sort((a, b) => a.anchorEventCount.compareTo(b.anchorEventCount));

    Widget optimisticWidget(ThreadOptimisticUserMessage m) => MessageBubble(
      isUser: true,
      markdownContent: m.text,
      deliveryState: switch (m.status) {
        ThreadOptimisticMessageStatus.sending => MessageDeliveryState.sending,
        ThreadOptimisticMessageStatus.failed => MessageDeliveryState.failed,
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
        UiEventMessage_TextReplace(:final messageId) => messageId,
        UiEventMessage_ReasoningDelta(:final messageId) => messageId,
        UiEventMessage_ReasoningReplace(:final messageId) => messageId,
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
          if (text.trim().isNotEmpty) {
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

List<ThreadUserMessageEcho> _extractUserMessageEchoes(
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
          texts.putIfAbsent(messageId, StringBuffer.new);
        case UiEventMessage_TextDelta(:final messageId, :final text):
          texts
              .putIfAbsent(messageId, StringBuffer.new)
              .write(text.renderPreview());
        case UiEventMessage_TextReplace(:final messageId, :final text):
          texts[messageId] = StringBuffer(text.renderPreview());
        default:
          break;
      }
    }
  }

  final echoes = <ThreadUserMessageEcho>[];
  for (final entry in roles.entries) {
    if (entry.value != MessageRole.user) continue;
    final text = texts[entry.key]?.toString() ?? '';
    if (text.trim().isEmpty) continue;
    echoes.add(
      ThreadUserMessageEcho(eventIndex: starts[entry.key] ?? 0, text: text),
    );
  }
  echoes.sort((a, b) => a.eventIndex.compareTo(b.eventIndex));
  return echoes;
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
