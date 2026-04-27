import 'package:flutter/material.dart';

import 'package:minos/src/rust/api/minos.dart' show AgentName;

/// Caption row that anchors a message to the wall clock (timestamp) and
/// the model that produced it. Rendered above an assistant bubble or
/// below a user bubble depending on caller layout — the row itself is
/// alignment-neutral.
class MessageMetaRow extends StatelessWidget {
  const MessageMetaRow({super.key, required this.timestamp, this.agent});

  /// Wall-clock time (UTC) the message originated. Rendered as
  /// HH:MM in the local zone for readability.
  final DateTime timestamp;

  /// The agent the message came from. Null for user messages — only
  /// the timestamp is shown then.
  final AgentName? agent;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final style = theme.textTheme.labelSmall?.copyWith(
      color: theme.colorScheme.onSurfaceVariant,
    );
    final time = _formatHHMM(timestamp.toLocal());
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 2),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(time, style: style),
          if (agent != null) ...[
            const SizedBox(width: 6),
            Text('•', style: style),
            const SizedBox(width: 6),
            Text(_agentLabel(agent!), style: style),
          ],
        ],
      ),
    );
  }

  static String _formatHHMM(DateTime dt) {
    final hh = dt.hour.toString().padLeft(2, '0');
    final mm = dt.minute.toString().padLeft(2, '0');
    return '$hh:$mm';
  }

  static String _agentLabel(AgentName a) {
    return switch (a) {
      AgentName.codex => 'Codex',
      AgentName.claude => 'Claude',
      AgentName.gemini => 'Gemini',
    };
  }
}
