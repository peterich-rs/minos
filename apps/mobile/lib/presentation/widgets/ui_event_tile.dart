import 'package:flutter/material.dart';

import 'package:minos/src/rust/api/minos.dart';

/// One row per `UiEventMessage`. Deliberately plain — spec scopes the
/// viewer UI to a debug list (plan §D7). Each variant renders its kind
/// plus a single primary-text line; the caller can tap to see structure
/// later, but MVP surfaces the raw fields directly.
class UiEventTile extends StatelessWidget {
  const UiEventTile({super.key, required this.event});

  final UiEventMessage event;

  @override
  Widget build(BuildContext context) {
    final (label, body) = _describe(event);
    return ListTile(
      dense: true,
      title: Text(
        label,
        style: const TextStyle(
          fontFamily: 'monospace',
          fontSize: 11,
          fontWeight: FontWeight.bold,
        ),
      ),
      subtitle: Text(
        body,
        style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
      ),
    );
  }

  (String, String) _describe(UiEventMessage e) {
    return switch (e) {
      UiEventMessage_ThreadOpened(
        :final threadId,
        :final agent,
        :final title,
      ) =>
        ('ThreadOpened', 'thread=$threadId agent=$agent title=${title ?? ""}'),
      UiEventMessage_ThreadTitleUpdated(:final threadId, :final title) => (
        'ThreadTitleUpdated',
        'thread=$threadId title=$title',
      ),
      UiEventMessage_ThreadClosed(:final threadId, :final reason) => (
        'ThreadClosed',
        'thread=$threadId reason=$reason',
      ),
      UiEventMessage_MessageStarted(:final messageId, :final role) => (
        'MessageStarted',
        'id=$messageId role=$role',
      ),
      UiEventMessage_MessageCompleted(:final messageId) => (
        'MessageCompleted',
        'id=$messageId',
      ),
      UiEventMessage_TextDelta(:final messageId, :final text) => (
        'TextDelta',
        '[$messageId] $text',
      ),
      UiEventMessage_ReasoningDelta(:final messageId, :final text) => (
        'ReasoningDelta',
        '[$messageId] $text',
      ),
      UiEventMessage_ToolCallPlaced(
        :final toolCallId,
        :final name,
        :final argsJson,
      ) =>
        ('ToolCallPlaced', '$name($toolCallId) args=$argsJson'),
      UiEventMessage_ToolCallCompleted(
        :final toolCallId,
        :final output,
        :final isError,
      ) =>
        ('ToolCallCompleted', '$toolCallId isError=$isError out=$output'),
      UiEventMessage_Error(:final code, :final message) => (
        'Error',
        '[$code] $message',
      ),
      UiEventMessage_Raw(:final kind, :final payloadJson) => (
        'Raw',
        '$kind: $payloadJson',
      ),
    };
  }
}
