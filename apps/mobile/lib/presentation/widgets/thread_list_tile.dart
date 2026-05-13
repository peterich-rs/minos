import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

/// One row in the thread list. iOS-flat: circular gradient avatar on the
/// left, two-line title/preview in the middle, right-rail timestamp +
/// optional "ended" lock icon. Inspired by WeChat / iMessage list rows.
class ThreadListTile extends ConsumerWidget {
  const ThreadListTile({
    super.key,
    required ThreadSummary this.summary,
    this.onTap,
    this.onDelete,
  }) : _socialTitle = null,
       _socialPreview = null,
       _socialTimestampMs = null,
       _socialAvatarLabel = null,
       _socialAvatarTint = null,
       _socialUnreadCount = null,
       _socialHasUnreadMention = null;

  const ThreadListTile.social({
    super.key,
    required String title,
    required String preview,
    required int timestampMs,
    required String avatarLabel,
    required Color avatarTint,
    int unreadCount = 0,
    bool hasUnreadMention = false,
    this.onTap,
  }) : summary = null,
       onDelete = null,
       _socialTitle = title,
       _socialPreview = preview,
       _socialTimestampMs = timestampMs,
       _socialAvatarLabel = avatarLabel,
       _socialAvatarTint = avatarTint,
       _socialUnreadCount = unreadCount,
       _socialHasUnreadMention = hasUnreadMention;

  final ThreadSummary? summary;
  final VoidCallback? onTap;
  final VoidCallback? onDelete;
  final String? _socialTitle;
  final String? _socialPreview;
  final int? _socialTimestampMs;
  final String? _socialAvatarLabel;
  final Color? _socialAvatarTint;
  final int? _socialUnreadCount;
  final bool? _socialHasUnreadMention;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final shadTheme = ShadTheme.of(context);
    if (summary == null) {
      return _SocialThreadTile(
        title: _socialTitle!,
        preview: _socialPreview!,
        timestampMs: _socialTimestampMs!,
        avatarLabel: _socialAvatarLabel!,
        avatarTint: _socialAvatarTint!,
        unreadCount: _socialUnreadCount!,
        hasUnreadMention: _socialHasUnreadMention!,
        onTap: onTap,
      );
    }
    final boundProfile = ref.watch(
      threadBoundAgentProfileProvider(summary!.threadId),
    );
    final title = summary!.title?.trim().isNotEmpty == true
        ? summary!.title!
        : (boundProfile?.name ?? '新对话');
    final previewSource = boundProfile?.name ?? _agentLabel(summary!.agent);
    final preview = '$previewSource · ${summary!.messageCount} 条消息';
    final ended = summary!.endedAtMs != null;

    final tileContent = Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
      child: Material(
        color: shadTheme.colorScheme.card,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
        clipBehavior: Clip.antiAlias,
        elevation: 0,
        child: InkWell(
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(12, 12, 10, 12),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.center,
              children: <Widget>[
                _AgentAvatar(
                  agent: boundProfile?.runtimeAgent ?? summary!.agent,
                ),
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
                            _formatRelativeTimestamp(summary!.lastTsMs.toInt()),
                            style: shadTheme.textTheme.muted.copyWith(
                              fontSize: 11,
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
                              LucideIcons.lock,
                              size: 13,
                              color: shadTheme.colorScheme.mutedForeground,
                            ),
                        ],
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                Icon(
                  LucideIcons.chevronRight,
                  size: 16,
                  color: shadTheme.colorScheme.mutedForeground,
                ),
              ],
            ),
          ),
        ),
      ),
    );

    if (onDelete == null) return tileContent;

    return Dismissible(
      key: ValueKey(summary!.threadId),
      direction: DismissDirection.endToStart,
      confirmDismiss: (_) async => true,
      onDismissed: (_) => onDelete!(),
      background: Container(
        alignment: Alignment.centerRight,
        padding: const EdgeInsets.only(right: 24),
        margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
        decoration: BoxDecoration(
          color: shadTheme.colorScheme.destructive,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Icon(
          LucideIcons.trash2,
          color: shadTheme.colorScheme.destructiveForeground,
          size: 20,
        ),
      ),
      child: tileContent,
    );
  }
}

class _SocialThreadTile extends StatelessWidget {
  const _SocialThreadTile({
    required this.title,
    required this.preview,
    required this.timestampMs,
    required this.avatarLabel,
    required this.avatarTint,
    required this.unreadCount,
    required this.hasUnreadMention,
    this.onTap,
  });

  final String title;
  final String preview;
  final int timestampMs;
  final String avatarLabel;
  final Color avatarTint;
  final int unreadCount;
  final bool hasUnreadMention;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final shadTheme = ShadTheme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
      child: Material(
        color: shadTheme.colorScheme.card,
        borderRadius: BorderRadius.circular(8),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(12, 12, 10, 12),
            child: Row(
              children: <Widget>[
                Container(
                  width: 44,
                  height: 44,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: avatarTint.withValues(alpha: isDark ? 0.28 : 0.14),
                  ),
                  alignment: Alignment.center,
                  child: Text(
                    avatarLabel,
                    style: TextStyle(
                      color: avatarTint,
                      fontWeight: FontWeight.w700,
                      fontSize: 18,
                    ),
                  ),
                ),
                const SizedBox(width: 12),
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
                              style: theme.textTheme.titleMedium?.copyWith(
                                fontWeight: FontWeight.w600,
                              ),
                            ),
                          ),
                          const SizedBox(width: 8),
                          Text(
                            _formatRelativeTimestamp(timestampMs),
                            style: shadTheme.textTheme.muted.copyWith(
                              fontSize: 11,
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 4),
                      Text(
                        preview,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          color: unreadCount > 0 || hasUnreadMention
                              ? theme.colorScheme.onSurface
                              : theme.colorScheme.onSurfaceVariant,
                          fontWeight: unreadCount > 0 || hasUnreadMention
                              ? FontWeight.w600
                              : FontWeight.w400,
                        ),
                      ),
                      if (hasUnreadMention || unreadCount > 0) ...<Widget>[
                        const SizedBox(height: 8),
                        Row(
                          children: <Widget>[
                            if (hasUnreadMention) const _MentionBadge(),
                            if (hasUnreadMention && unreadCount > 0)
                              const SizedBox(width: 6),
                            if (unreadCount > 0)
                              _UnreadCountBadge(count: unreadCount),
                          ],
                        ),
                      ],
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                Icon(
                  LucideIcons.chevronRight,
                  size: 16,
                  color: shadTheme.colorScheme.mutedForeground,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _UnreadCountBadge extends StatelessWidget {
  const _UnreadCountBadge({required this.count});

  final int count;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: shadTheme.colorScheme.primary,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        child: Text(
          count > 99 ? '99+' : '$count',
          style: shadTheme.textTheme.small.copyWith(
            color: shadTheme.colorScheme.primaryForeground,
            fontWeight: FontWeight.w700,
          ),
        ),
      ),
    );
  }
}

class _MentionBadge extends StatelessWidget {
  const _MentionBadge();

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: const Color(0xFFF59E0B).withValues(alpha: 0.16),
        borderRadius: BorderRadius.circular(999),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        child: Text(
          '@你',
          style: shadTheme.textTheme.small.copyWith(
            color: const Color(0xFFB45309),
            fontWeight: FontWeight.w700,
          ),
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
    final isDark = Theme.of(context).brightness == Brightness.dark;
    final (label, bgColor, fgColor) = switch (agent) {
      AgentName.codex => (
        'C',
        isDark ? const Color(0xFF14532D) : const Color(0xFFDCFCE7),
        isDark ? const Color(0xFF4ADE80) : const Color(0xFF16A34A),
      ),
      AgentName.claude => (
        'A',
        isDark ? const Color(0xFF7C2D12) : const Color(0xFFFFEDD5),
        isDark ? const Color(0xFFFB923C) : const Color(0xFFEA580C),
      ),
      AgentName.gemini => (
        'G',
        isDark ? const Color(0xFF164E63) : const Color(0xFFCFFAFE),
        isDark ? const Color(0xFF22D3EE) : const Color(0xFF0891B2),
      ),
    };
    return Container(
      width: 44,
      height: 44,
      decoration: BoxDecoration(shape: BoxShape.circle, color: bgColor),
      alignment: Alignment.center,
      child: Text(
        label,
        style: TextStyle(
          color: fgColor,
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
