import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/minos_surface.dart';
import 'package:minos/ui/theme/theme.dart';

/// One row in the Sessions inbox.
class SessionTile extends StatelessWidget {
  const SessionTile({super.key, required this.session, required this.onTap});

  final SessionSummary session;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final ended = session.endedAtMs != null;
    final title = _sessionTitle(session);
    final agent = _agentLabel(session.agent);

    return MinosSurface(
      bordered: true,
      onTap: onTap,
      padding: const EdgeInsets.symmetric(
        horizontal: MinosSpacing.lg,
        vertical: MinosSpacing.md,
      ),
      child: Row(
        children: <Widget>[
          Container(
            width: 42,
            height: 42,
            decoration: BoxDecoration(
              color: ended ? colors.surfaceMuted : colors.accentSoft,
              borderRadius: MinosRadii.smAll,
            ),
            alignment: Alignment.center,
            child: Icon(
              CupertinoIcons.bubble_left_bubble_right_fill,
              size: 18,
              color: ended ? colors.textTertiary : colors.accent,
            ),
          ),
          const SizedBox(width: MinosSpacing.md),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Row(
                  children: <Widget>[
                    Expanded(
                      child: Text(
                        title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.titleSmall?.copyWith(
                          color: ended
                              ? colors.textSecondary
                              : colors.textPrimary,
                        ),
                      ),
                    ),
                    const SizedBox(width: MinosSpacing.sm),
                    Text(
                      _formatRelativeTimestamp(session.lastTsMs.toInt()),
                      style: theme.textTheme.labelSmall?.copyWith(
                        color: colors.textTertiary,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: MinosSpacing.xs),
                Text(
                  '$agent · ${session.messageCount} 条消息'
                  '${ended ? ' · 已结束' : ''}',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: colors.textSecondary,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: MinosSpacing.sm),
          Icon(
            ended ? CupertinoIcons.lock : CupertinoIcons.chevron_right,
            size: 16,
            color: colors.textTertiary,
          ),
        ],
      ),
    );
  }
}

String _sessionTitle(SessionSummary session) {
  final title = session.title?.trim();
  if (title != null && title.isNotEmpty) return title;
  final ts = DateTime.fromMillisecondsSinceEpoch(session.lastTsMs.toInt());
  return '${_agentLabel(session.agent)} · ${ts.month}/${ts.day} '
      '${_two(ts.hour)}:${_two(ts.minute)}';
}

String _agentLabel(AgentName agent) {
  return switch (agent) {
    AgentName.codex => 'Codex',
    AgentName.claude => 'Claude',
    AgentName.gemini => 'Gemini',
    AgentName.opencode => 'OpenCode',
    AgentName.grok => 'Grok',
  };
}

String _formatRelativeTimestamp(int ms) {
  final now = DateTime.now();
  final ts = DateTime.fromMillisecondsSinceEpoch(ms).toLocal();
  final diff = now.difference(ts);

  if (diff.inSeconds < 60) return '刚刚';
  if (diff.inMinutes < 60) return '${diff.inMinutes} 分钟前';

  final today = DateTime(now.year, now.month, now.day);
  final tsDay = DateTime(ts.year, ts.month, ts.day);
  if (tsDay == today) return '${_two(ts.hour)}:${_two(ts.minute)}';

  final yesterday = today.subtract(const Duration(days: 1));
  if (tsDay == yesterday) return '昨天';

  if (ts.year == now.year) return '${_two(ts.month)}-${_two(ts.day)}';
  return '${ts.year}-${_two(ts.month)}-${_two(ts.day)}';
}

String _two(int n) => n.toString().padLeft(2, '0');
