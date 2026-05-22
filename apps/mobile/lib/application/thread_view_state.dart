import 'dart:async';

import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'thread_view_state.g.dart';

@Riverpod(keepAlive: false)
class ThreadViewStateController extends _$ThreadViewStateController {
  @override
  ThreadViewState build(String viewId) {
    ref.onDispose(_disposeTimers);
    return const ThreadViewState();
  }

  static const Duration _sendStatusDelay = Duration(milliseconds: 500);

  final Map<String, Timer> _optimisticTimers = <String, Timer>{};
  int _lastEventCount = 0;
  int _nextOptimisticMessageId = 0;

  void updateScrollState({
    required double distanceFromBottom,
    required double stickyThreshold,
  }) {
    final isAtBottom = distanceFromBottom <= stickyThreshold;
    if (isAtBottom == state.stickToBottom) return;
    state = state.copyWith(
      stickToBottom: isAtBottom,
      unreadBelow: isAtBottom ? 0 : state.unreadBelow,
    );
  }

  void jumpToBottom() {
    if (state.stickToBottom && state.unreadBelow == 0) return;
    state = state.copyWith(stickToBottom: true, unreadBelow: 0);
  }

  void setApprovalSheetVisible(bool visible) {
    if (state.approvalSheetVisible == visible) return;
    state = state.copyWith(approvalSheetVisible: visible);
  }

  String enqueueOptimisticMessage({
    required String text,
    required int anchorEventCount,
  }) {
    final message = ThreadOptimisticUserMessage(
      id: 'optimistic-${_nextOptimisticMessageId++}',
      text: text,
      status: ThreadOptimisticMessageStatus.pending,
      anchorEventCount: anchorEventCount,
    );
    state = state.copyWith(
      optimisticMessages: <ThreadOptimisticUserMessage>[
        ...state.optimisticMessages,
        message,
      ],
    );
    _optimisticTimers[message.id] = Timer(_sendStatusDelay, () {
      updateOptimisticMessage(
        message.id,
        (current) => current.status == ThreadOptimisticMessageStatus.pending
            ? current.copyWith(status: ThreadOptimisticMessageStatus.sending)
            : current,
      );
    });
    return message.id;
  }

  bool handleThreadMetrics({
    required int eventCount,
    required List<ThreadUserMessageEcho> userMessages,
  }) {
    _consumeConfirmedUserMessages(userMessages);

    final totalCount = eventCount + state.optimisticMessages.length;
    if (totalCount == _lastEventCount) return false;

    final delta = totalCount - _lastEventCount;
    _lastEventCount = totalCount;
    if (state.stickToBottom) {
      return true;
    }
    if (delta > 0) {
      state = state.copyWith(unreadBelow: state.unreadBelow + delta);
    }
    return false;
  }

  void markOptimisticMessageFailed(String id) {
    _clearOptimisticTimer(id);
    updateOptimisticMessage(
      id,
      (current) =>
          current.copyWith(status: ThreadOptimisticMessageStatus.failed),
    );
  }

  void markOptimisticMessageConfirmed(String id) {
    _clearOptimisticTimer(id);
    updateOptimisticMessage(
      id,
      (current) =>
          current.copyWith(status: ThreadOptimisticMessageStatus.confirmed),
    );
  }

  void updateOptimisticMessage(
    String id,
    ThreadOptimisticUserMessage Function(ThreadOptimisticUserMessage current)
    transform,
  ) {
    final index = state.optimisticMessages.indexWhere(
      (message) => message.id == id,
    );
    if (index == -1) return;

    final optimisticMessages = List<ThreadOptimisticUserMessage>.of(
      state.optimisticMessages,
    );
    optimisticMessages[index] = transform(optimisticMessages[index]);
    state = state.copyWith(optimisticMessages: optimisticMessages);
  }

  void _clearOptimisticTimer(String id) {
    _optimisticTimers.remove(id)?.cancel();
  }

  void _consumeConfirmedUserMessages(List<ThreadUserMessageEcho> userMessages) {
    final confirmedIds = state.optimisticMessages
        .where(
          (message) =>
              message.status == ThreadOptimisticMessageStatus.confirmed &&
              threadOptimisticHasEcho(message, userMessages),
        )
        .map((message) => message.id)
        .toSet();
    if (confirmedIds.isEmpty) return;

    for (final id in confirmedIds) {
      _clearOptimisticTimer(id);
    }

    state = state.copyWith(
      optimisticMessages: state.optimisticMessages
          .where((message) => !confirmedIds.contains(message.id))
          .toList(growable: false),
    );
  }

  void _disposeTimers() {
    for (final timer in _optimisticTimers.values) {
      timer.cancel();
    }
    _optimisticTimers.clear();
  }
}

class ThreadViewState {
  const ThreadViewState({
    this.stickToBottom = true,
    this.unreadBelow = 0,
    this.optimisticMessages = const <ThreadOptimisticUserMessage>[],
    this.approvalSheetVisible = false,
  });

  final bool stickToBottom;
  final int unreadBelow;
  final List<ThreadOptimisticUserMessage> optimisticMessages;
  final bool approvalSheetVisible;

  ThreadViewState copyWith({
    bool? stickToBottom,
    int? unreadBelow,
    List<ThreadOptimisticUserMessage>? optimisticMessages,
    bool? approvalSheetVisible,
  }) {
    return ThreadViewState(
      stickToBottom: stickToBottom ?? this.stickToBottom,
      unreadBelow: unreadBelow ?? this.unreadBelow,
      optimisticMessages: optimisticMessages ?? this.optimisticMessages,
      approvalSheetVisible: approvalSheetVisible ?? this.approvalSheetVisible,
    );
  }
}

enum ThreadOptimisticMessageStatus { pending, sending, confirmed, failed }

class ThreadOptimisticUserMessage {
  const ThreadOptimisticUserMessage({
    required this.id,
    required this.text,
    required this.status,
    required this.anchorEventCount,
  });

  final String id;
  final String text;
  final ThreadOptimisticMessageStatus status;
  final int anchorEventCount;

  ThreadOptimisticUserMessage copyWith({
    ThreadOptimisticMessageStatus? status,
  }) {
    return ThreadOptimisticUserMessage(
      id: id,
      text: text,
      status: status ?? this.status,
      anchorEventCount: anchorEventCount,
    );
  }
}

class ThreadUserMessageEcho {
  const ThreadUserMessageEcho({required this.eventIndex, required this.text});

  final int eventIndex;
  final String text;

  String get normalizedText => normalizeThreadMessageText(text);
}

bool threadOptimisticHasEcho(
  ThreadOptimisticUserMessage message,
  List<ThreadUserMessageEcho> echoes,
) {
  final normalized = normalizeThreadMessageText(message.text);
  if (normalized.isEmpty) return false;
  return echoes.any(
    (echo) =>
        echo.eventIndex >= message.anchorEventCount &&
        echo.normalizedText == normalized,
  );
}

String normalizeThreadMessageText(String text) {
  return text.trim().split(RegExp(r'\s+')).where((s) => s.isNotEmpty).join(' ');
}
