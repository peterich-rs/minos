import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';

import 'package:minos/src/rust/api/minos.dart';

/// One row in the thread list. iOS-flat: circular gradient avatar on the
/// left, two-line title/preview in the middle, right-rail timestamp +
/// optional "ended" lock icon. Inspired by WeChat / iMessage list rows.
class ThreadListTile extends StatelessWidget {
  const ThreadListTile({super.key, required this.summary, this.onTap});

  final ThreadSummary summary;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final title = summary.title?.trim().isNotEmpty == true
        ? summary.title!
        : '新对话';
    final preview =
        '${_agentLabel(summary.agent)} · ${summary.messageCount} 条消息';
    final ended = summary.endedAtMs != null;

    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: <Widget>[
            _AgentAvatar(agent: summary.agent),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisAlignment: MainAxisAlignment.center,
                children: <Widget>[
                  Row(
                    children: <Widget>[
                      Expanded(
                        child: Text(
                          title,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.titleMedium?.copyWith(
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Text(
                        _formatRelativeTimestamp(summary.lastTsMs.toInt()),
                        style: theme.textTheme.bodySmall?.copyWith(
                          color: theme.colorScheme.onSurfaceVariant,
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 4),
                  Row(
                    children: <Widget>[
                      Expanded(
                        child: Text(
                          preview,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.bodyMedium?.copyWith(
                            color: theme.colorScheme.onSurfaceVariant,
                          ),
                        ),
                      ),
                      if (ended)
                        Icon(
                          CupertinoIcons.lock_fill,
                          size: 13,
                          color: theme.colorScheme.outline,
                        ),
                    ],
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

class _AgentAvatar extends StatelessWidget {
  const _AgentAvatar({required this.agent});

  final AgentName agent;

  @override
  Widget build(BuildContext context) {
    final (label, gradient) = switch (agent) {
      AgentName.codex => (
        'C',
        const LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: <Color>[Color(0xFF30D158), Color(0xFF248A3D)],
        ),
      ),
      AgentName.claude => (
        'A',
        const LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: <Color>[Color(0xFFFF9F0A), Color(0xFFC93400)],
        ),
      ),
      AgentName.gemini => (
        'G',
        const LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: <Color>[Color(0xFF64D2FF), Color(0xFF0A84FF)],
        ),
      ),
    };
    return Container(
      width: 44,
      height: 44,
      decoration: BoxDecoration(shape: BoxShape.circle, gradient: gradient),
      alignment: Alignment.center,
      child: Text(
        label,
        style: const TextStyle(
          color: Colors.white,
          fontWeight: FontWeight.w700,
          fontSize: 18,
          letterSpacing: 0.3,
        ),
      ),
    );
  }
}

String _agentLabel(AgentName agent) {
  return switch (agent) {
    AgentName.codex => 'Codex',
    AgentName.claude => 'Claude',
    AgentName.gemini => 'Gemini',
  };
}

/// WeChat-style relative timestamp:
///   - <1 min   → "刚刚"
///   - <1 hour  → "x 分钟前"
///   - same day → "HH:MM"
///   - yesterday → "昨天"
///   - this year → "MM-DD"
///   - older    → "YYYY-MM-DD"
String _formatRelativeTimestamp(int ms) {
  final now = DateTime.now();
  final ts = DateTime.fromMillisecondsSinceEpoch(ms).toLocal();
  final diff = now.difference(ts);

  if (diff.inSeconds < 60) return '刚刚';
  if (diff.inMinutes < 60) return '${diff.inMinutes} 分钟前';

  final today = DateTime(now.year, now.month, now.day);
  final tsDay = DateTime(ts.year, ts.month, ts.day);
  if (tsDay == today) {
    return '${_two(ts.hour)}:${_two(ts.minute)}';
  }
  final yesterday = today.subtract(const Duration(days: 1));
  if (tsDay == yesterday) return '昨天';

  if (ts.year == now.year) {
    return '${_two(ts.month)}-${_two(ts.day)}';
  }
  return '${ts.year}-${_two(ts.month)}-${_two(ts.day)}';
}

String _two(int n) => n.toString().padLeft(2, '0');
