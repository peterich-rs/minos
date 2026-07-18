import 'package:minos/application/display_payload_preview.dart';
import 'package:minos/src/rust/api/minos.dart';

/// One row in the chat transcript, ordered by first-appearance event index.
///
/// Gemini/Grok ACP (and other agents) may attach reasoning, tool calls, and
/// assistant text to the same `message_id` for a turn. The transcript must still
/// follow **output order**: each content kind becomes its own item at the event
/// where it first appeared; later contiguous deltas update that item in place.
/// Intervening tool/text/reasoning events close the previous open segment so a
/// later thought or reply opens a **new** item at the end of the timeline
/// instead of rewriting or concatenating into an earlier row.
sealed class ThreadTimelineItem {
  const ThreadTimelineItem({required this.eventIndex});

  /// Index of the first `UiEventMessage` that opened this row.
  final int eventIndex;
}

final class TimelineUserMessage extends ThreadTimelineItem {
  const TimelineUserMessage({
    required super.eventIndex,
    required this.messageId,
    required this.text,
  });

  final String messageId;
  final String text;
}

final class TimelineAssistantText extends ThreadTimelineItem {
  const TimelineAssistantText({
    required super.eventIndex,
    required this.messageId,
    required this.text,
    required this.showCursor,
  });

  final String messageId;
  final String text;
  final bool showCursor;
}

final class TimelineReasoning extends ThreadTimelineItem {
  const TimelineReasoning({
    required super.eventIndex,
    required this.messageId,
    required this.text,
    required this.isLive,
  });

  final String messageId;
  final String text;
  final bool isLive;
}

final class TimelineToolCall extends ThreadTimelineItem {
  const TimelineToolCall({
    required super.eventIndex,
    required this.messageId,
    required this.toolCallId,
    required this.name,
    required this.argsJson,
    required this.output,
    required this.isError,
  });

  final String messageId;
  final String toolCallId;
  final String name;
  final String argsJson;
  final String? output;
  final bool isError;
}

final class TimelineAssistantPlaceholder extends ThreadTimelineItem {
  const TimelineAssistantPlaceholder({
    required super.eventIndex,
    required this.messageId,
  });

  final String messageId;
}

final class TimelineError extends ThreadTimelineItem {
  const TimelineError({
    required super.eventIndex,
    required this.code,
    required this.message,
  });

  final String code;
  final String message;
}

final class TimelineClosed extends ThreadTimelineItem {
  const TimelineClosed({required super.eventIndex});
}

/// Projects a flat ordered `UiEventMessage` list into timeline rows.
List<ThreadTimelineItem> buildThreadEventTimeline(
  List<UiEventMessage> events, {
  bool showLiveAssistantState = false,
}) {
  final roleByMsg = <String, MessageRole>{};
  final completedMsgs = <String>{};
  final userTextByMsg = <String, StringBuffer>{};
  final messageStartIndex = <String, int>{};
  final toolById = <String, _ToolAccum>{};
  final toolFirstIndex = <String, int>{};
  final textSegments = <_ContentSegment>[];
  final reasoningSegments = <_ContentSegment>[];
  final markers = <ThreadTimelineItem>[];

  String? openTextMessageId;
  String? openReasoningMessageId;
  String? lastAssistantMessageId;

  void closeTextSegment() {
    openTextMessageId = null;
  }

  void closeReasoningSegment() {
    openReasoningMessageId = null;
  }

  void appendSegment({
    required List<_ContentSegment> segments,
    required String? Function() getOpenId,
    required void Function(String?) setOpenId,
    required String messageId,
    required int eventIndex,
    required String text,
  }) {
    if (text.isEmpty) return;
    if (getOpenId() == messageId && segments.isNotEmpty) {
      segments.last.text.write(text);
      return;
    }
    segments.add(
      _ContentSegment(
        messageId: messageId,
        eventIndex: eventIndex,
        text: StringBuffer(text),
      ),
    );
    setOpenId(messageId);
  }

  void replaceSegment({
    required List<_ContentSegment> segments,
    required String? Function() getOpenId,
    required void Function(String?) setOpenId,
    required String messageId,
    required int eventIndex,
    required String text,
  }) {
    if (text.isEmpty) {
      segments.removeWhere((segment) => segment.messageId == messageId);
      if (getOpenId() == messageId) {
        setOpenId(null);
      }
      return;
    }
    if (getOpenId() == messageId && segments.isNotEmpty) {
      segments.last.text
        ..clear()
        ..write(text);
      return;
    }
    segments.add(
      _ContentSegment(
        messageId: messageId,
        eventIndex: eventIndex,
        text: StringBuffer(text),
      ),
    );
    setOpenId(messageId);
  }

  void appendText(String messageId, int eventIndex, String text) {
    appendSegment(
      segments: textSegments,
      getOpenId: () => openTextMessageId,
      setOpenId: (id) => openTextMessageId = id,
      messageId: messageId,
      eventIndex: eventIndex,
      text: text,
    );
  }

  void replaceText(String messageId, int eventIndex, String text) {
    replaceSegment(
      segments: textSegments,
      getOpenId: () => openTextMessageId,
      setOpenId: (id) => openTextMessageId = id,
      messageId: messageId,
      eventIndex: eventIndex,
      text: text,
    );
  }

  void appendReasoning(String messageId, int eventIndex, String text) {
    appendSegment(
      segments: reasoningSegments,
      getOpenId: () => openReasoningMessageId,
      setOpenId: (id) => openReasoningMessageId = id,
      messageId: messageId,
      eventIndex: eventIndex,
      text: text,
    );
  }

  void replaceReasoning(String messageId, int eventIndex, String text) {
    replaceSegment(
      segments: reasoningSegments,
      getOpenId: () => openReasoningMessageId,
      setOpenId: (id) => openReasoningMessageId = id,
      messageId: messageId,
      eventIndex: eventIndex,
      text: text,
    );
  }

  for (var i = 0; i < events.length; i++) {
    final event = events[i];
    switch (event) {
      case UiEventMessage_MessageStarted(:final messageId, :final role):
        messageStartIndex.putIfAbsent(messageId, () => i);
        roleByMsg[messageId] = role;
        if (role == MessageRole.user) {
          userTextByMsg.putIfAbsent(messageId, StringBuffer.new);
        }
        if (role == MessageRole.assistant) {
          lastAssistantMessageId = messageId;
        }
        // A new message boundary always ends contiguous content segments.
        closeTextSegment();
        closeReasoningSegment();
      case UiEventMessage_TextDelta(:final messageId, :final text):
        final preview = text.renderPreview();
        if (preview.isEmpty) break;
        final role = roleByMsg[messageId] ?? MessageRole.assistant;
        if (role == MessageRole.user) {
          userTextByMsg.putIfAbsent(messageId, StringBuffer.new).write(preview);
        } else {
          // Intervening thought/tool closes any open text segment first via
          // those handlers; contiguous assistant deltas stay one bubble.
          appendText(messageId, i, preview);
        }
        closeReasoningSegment();
      case UiEventMessage_TextReplace(:final messageId, :final text):
        final preview = text.renderPreview();
        final role = roleByMsg[messageId] ?? MessageRole.assistant;
        if (role == MessageRole.user) {
          userTextByMsg[messageId] = StringBuffer(preview);
        } else {
          replaceText(messageId, i, preview);
        }
        closeReasoningSegment();
      case UiEventMessage_ReasoningDelta(:final messageId, :final text):
        // Thought after intermediate reply/tool must open at the timeline tail.
        closeTextSegment();
        appendReasoning(messageId, i, text.renderPreview());
      case UiEventMessage_ReasoningReplace(:final messageId, :final text):
        closeTextSegment();
        replaceReasoning(messageId, i, text.renderPreview());
      case UiEventMessage_MessageCompleted(:final messageId):
        completedMsgs.add(messageId);
        closeTextSegment();
        closeReasoningSegment();
      case UiEventMessage_ToolCallPlaced(
        :final messageId,
        :final toolCallId,
        :final name,
        :final argsJson,
      ):
        toolById[toolCallId] = _ToolAccum(
          messageId: messageId,
          name: name,
          argsJson: argsJson.renderPreview(),
        );
        toolFirstIndex.putIfAbsent(toolCallId, () => i);
        closeTextSegment();
        closeReasoningSegment();
      case UiEventMessage_ToolCallCompleted(
        :final toolCallId,
        :final output,
        :final isError,
      ):
        final existing = toolById[toolCallId];
        if (existing != null) {
          existing.output = output.renderPreview();
          existing.isError = isError;
        } else {
          final messageId = lastAssistantMessageId ?? '';
          toolById[toolCallId] = _ToolAccum(
            messageId: messageId,
            name: '(unknown)',
            argsJson: '{}',
            output: output.renderPreview(),
            isError: isError,
          );
          toolFirstIndex.putIfAbsent(toolCallId, () => i);
        }
      case UiEventMessage_Error(:final code, :final message):
        markers.add(TimelineError(eventIndex: i, code: code, message: message));
      case UiEventMessage_ThreadClosed():
        markers.add(TimelineClosed(eventIndex: i));
      case UiEventMessage_ThreadOpened():
      case UiEventMessage_ThreadTitleUpdated():
      case UiEventMessage_SubagentSpawned():
      case UiEventMessage_SubagentStatusUpdated():
      case UiEventMessage_Raw():
        // Subagent tree is rendered via agent-session hierarchy, not the
        // linear message timeline.
        break;
    }
  }

  final liveAssistantMessageId = showLiveAssistantState
      ? lastAssistantMessageId
      : null;

  final rows = <ThreadTimelineItem>[...markers];

  for (final entry in roleByMsg.entries) {
    final messageId = entry.key;
    final role = entry.value;
    if (role != MessageRole.user) continue;
    final text = userTextByMsg[messageId]?.toString() ?? '';
    if (text.trim().isEmpty) continue;
    rows.add(
      TimelineUserMessage(
        eventIndex: messageStartIndex[messageId] ?? 0,
        messageId: messageId,
        text: text,
      ),
    );
  }

  for (var index = 0; index < reasoningSegments.length; index++) {
    final segment = reasoningSegments[index];
    final text = segment.text.toString();
    if (text.isEmpty) continue;
    // Only the still-open reasoning segment is "live". Earlier thought blocks
    // that were interrupted by tools/text are finished chat rows.
    final isOpenTail =
        openReasoningMessageId == segment.messageId &&
        index == reasoningSegments.length - 1;
    final isLive =
        liveAssistantMessageId == segment.messageId &&
        isOpenTail &&
        !completedMsgs.contains(segment.messageId);
    rows.add(
      TimelineReasoning(
        eventIndex: segment.eventIndex,
        messageId: segment.messageId,
        text: text,
        isLive: isLive,
      ),
    );
  }

  for (final entry in toolFirstIndex.entries) {
    final toolCallId = entry.key;
    final tool = toolById[toolCallId];
    if (tool == null) continue;
    rows.add(
      TimelineToolCall(
        eventIndex: entry.value,
        messageId: tool.messageId,
        toolCallId: toolCallId,
        name: tool.name,
        argsJson: tool.argsJson,
        output: tool.output,
        isError: tool.isError,
      ),
    );
  }

  for (var index = 0; index < textSegments.length; index++) {
    final segment = textSegments[index];
    final text = segment.text.toString();
    if (text.isEmpty) continue;
    // Cursor only on the actively open text segment of the live turn. Closed
    // intermediate replies (e.g. early Grok/Gemini agent_message_chunk before
    // tools) are plain chat messages with no cursor.
    final isOpenTail =
        openTextMessageId == segment.messageId &&
        index == textSegments.length - 1;
    final showCursor =
        liveAssistantMessageId == segment.messageId &&
        isOpenTail &&
        !completedMsgs.contains(segment.messageId);
    rows.add(
      TimelineAssistantText(
        eventIndex: segment.eventIndex,
        messageId: segment.messageId,
        text: text,
        showCursor: showCursor,
      ),
    );
  }

  // Live assistant with MessageStarted but no content yet → single placeholder
  // at the MessageStarted index (under the user message only while waiting).
  if (liveAssistantMessageId != null &&
      !completedMsgs.contains(liveAssistantMessageId)) {
    final hasText = textSegments.any(
      (segment) => segment.messageId == liveAssistantMessageId,
    );
    final hasReasoning = reasoningSegments.any(
      (segment) => segment.messageId == liveAssistantMessageId,
    );
    final hasTool = toolById.values.any(
      (tool) => tool.messageId == liveAssistantMessageId,
    );
    if (!hasText && !hasReasoning && !hasTool) {
      rows.add(
        TimelineAssistantPlaceholder(
          eventIndex:
              messageStartIndex[liveAssistantMessageId] ?? events.length,
          messageId: liveAssistantMessageId,
        ),
      );
    }
  }

  rows.sort((a, b) {
    final byIndex = a.eventIndex.compareTo(b.eventIndex);
    if (byIndex != 0) return byIndex;
    return _timelineKindRank(a).compareTo(_timelineKindRank(b));
  });

  return rows;
}

int _timelineKindRank(ThreadTimelineItem item) {
  return switch (item) {
    TimelineUserMessage() => 0,
    TimelineReasoning() => 1,
    TimelineToolCall() => 2,
    TimelineAssistantText() => 3,
    TimelineAssistantPlaceholder() => 4,
    TimelineError() => 5,
    TimelineClosed() => 6,
  };
}

class _ContentSegment {
  _ContentSegment({
    required this.messageId,
    required this.eventIndex,
    required this.text,
  });

  final String messageId;
  final int eventIndex;
  final StringBuffer text;
}

class _ToolAccum {
  _ToolAccum({
    required this.messageId,
    required this.name,
    required this.argsJson,
    this.output,
    this.isError = false,
  });

  final String messageId;
  final String name;
  final String argsJson;
  String? output;
  bool isError;
}
