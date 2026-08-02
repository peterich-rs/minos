import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:minos/ui/theme/theme.dart';

/// iOS-style conversation row for the Messages inbox.
class ConversationTile extends StatelessWidget {
  const ConversationTile({
    super.key,
    required this.conversation,
    this.onConfirmDelete,
    this.onDelete,
  });

  final ConversationSummary conversation;
  final Future<bool> Function(ConversationSummary conversation)?
  onConfirmDelete;
  final Future<void> Function(ConversationSummary conversation)? onDelete;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final isGroup = conversation.kind == ConversationKind.group;
    final subtitle = conversation.lastMessagePreview?.trim();
    final unread = conversation.unreadCount;
    final mentionUnread = conversation.unreadMentionCount;

    final row = Material(
      color: colors.surface,
      child: InkWell(
        onTap: () => context.push(
          '/social/chat/${conversation.conversationId}',
          extra: SocialChatRouteExtra(
            title: conversation.title,
            kind: conversation.kind,
          ),
        ),
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: MinosSpacing.pageX,
            vertical: MinosSpacing.md,
          ),
          child: Row(
            children: <Widget>[
              Container(
                width: 48,
                height: 48,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: isGroup ? colors.accentSoft : colors.surfaceMuted,
                ),
                alignment: Alignment.center,
                child: Icon(
                  isGroup
                      ? CupertinoIcons.person_2_fill
                      : CupertinoIcons.person_fill,
                  size: 22,
                  color: isGroup ? colors.accent : colors.textSecondary,
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
                            conversation.title,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: theme.textTheme.titleMedium?.copyWith(
                              fontWeight: unread > 0
                                  ? FontWeight.w700
                                  : FontWeight.w600,
                            ),
                          ),
                        ),
                        const SizedBox(width: MinosSpacing.sm),
                        Text(
                          formatConversationTime(
                            conversation.lastMessageAtMs.toInt(),
                          ),
                          style: theme.textTheme.bodySmall?.copyWith(
                            color: unread > 0
                                ? colors.accent
                                : colors.textTertiary,
                            fontWeight: unread > 0
                                ? FontWeight.w600
                                : FontWeight.w400,
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: MinosSpacing.xs),
                    Row(
                      children: <Widget>[
                        Expanded(
                          child: Text(
                            subtitle == null || subtitle.isEmpty
                                ? isGroup
                                      ? '${conversation.memberCount} 位成员'
                                      : '还没有消息'
                                : subtitle,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: theme.textTheme.bodyMedium?.copyWith(
                              color: colors.textSecondary,
                              fontWeight: unread > 0
                                  ? FontWeight.w500
                                  : FontWeight.w400,
                            ),
                          ),
                        ),
                        if (unread > 0) ...<Widget>[
                          const SizedBox(width: MinosSpacing.sm),
                          _UnreadBadge(
                            count: unread,
                            highlighted: mentionUnread > 0,
                          ),
                        ],
                      ],
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );

    final confirm = onConfirmDelete;
    final delete = onDelete;
    if (confirm == null || delete == null) {
      return DecoratedBox(
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: colors.borderSubtle)),
        ),
        child: row,
      );
    }

    return Dismissible(
      key: ValueKey<String>('conversation-${conversation.conversationId}'),
      direction: DismissDirection.endToStart,
      confirmDismiss: (_) async {
        if (!await confirm(conversation)) {
          return false;
        }
        await delete(conversation);
        return false;
      },
      background: Container(
        alignment: Alignment.centerRight,
        padding: const EdgeInsets.only(right: 22),
        color: colors.danger,
        child: Icon(CupertinoIcons.trash, color: colors.textOnAccent),
      ),
      child: DecoratedBox(
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: colors.borderSubtle)),
        ),
        child: row,
      ),
    );
  }
}

class _UnreadBadge extends StatelessWidget {
  const _UnreadBadge({required this.count, required this.highlighted});

  final int count;
  final bool highlighted;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final bg = highlighted ? colors.warning : colors.accent;
    final label = count > 99 ? '99+' : '$count';
    return DecoratedBox(
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        child: Text(
          label,
          style: theme.textTheme.labelSmall?.copyWith(
            color: colors.textOnAccent,
            fontWeight: FontWeight.w700,
          ),
        ),
      ),
    );
  }
}

String formatConversationTime(int ms) {
  if (ms <= 0) {
    return '';
  }
  final date = DateTime.fromMillisecondsSinceEpoch(ms);
  final now = DateTime.now();
  if (date.year == now.year && date.month == now.month && date.day == now.day) {
    final hour = date.hour.toString().padLeft(2, '0');
    final minute = date.minute.toString().padLeft(2, '0');
    return '$hour:$minute';
  }
  if (date.year == now.year) {
    return '${date.month}/${date.day}';
  }
  return '${date.year}/${date.month}/${date.day}';
}
