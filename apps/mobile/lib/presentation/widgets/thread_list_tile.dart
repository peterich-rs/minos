import 'package:flutter/material.dart';

import 'package:minos/src/rust/api/minos.dart';

/// One row in the thread list. Deliberately plain — the spec explicitly
/// scopes the viewer UI to "unstyled debug viewer" (plan §D6).
class ThreadListTile extends StatelessWidget {
  const ThreadListTile({super.key, required this.summary, this.onTap});

  final ThreadSummary summary;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: _AgentBadge(agent: summary.agent),
      title: Text(summary.title ?? '<untitled>'),
      subtitle: Text(
        '${summary.messageCount} msg · ${_formatTs(summary.lastTsMs.toInt())}',
      ),
      trailing: summary.endedAtMs != null ? const Icon(Icons.lock) : null,
      onTap: onTap,
    );
  }

  String _formatTs(int ms) {
    final d = DateTime.fromMillisecondsSinceEpoch(ms);
    return d.toLocal().toIso8601String();
  }
}

class _AgentBadge extends StatelessWidget {
  const _AgentBadge({required this.agent});

  final AgentName agent;

  @override
  Widget build(BuildContext context) {
    final (label, color) = switch (agent) {
      AgentName.codex => ('CDX', Colors.green),
      AgentName.claude => ('CLD', Colors.purple),
      AgentName.gemini => ('GEM', Colors.blue),
    };
    return Container(
      width: 40,
      height: 40,
      decoration: BoxDecoration(
        color: color,
        borderRadius: BorderRadius.circular(8),
      ),
      alignment: Alignment.center,
      child: Text(
        label,
        style: const TextStyle(
          color: Colors.white,
          fontWeight: FontWeight.bold,
          fontSize: 12,
        ),
      ),
    );
  }
}
