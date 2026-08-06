import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/features/social/lib/message_grouping.dart';
import 'package:minos/ui/features/social/widgets/conversation_message_chrome.dart';
import 'package:minos/ui/features/social/widgets/conversation_reply_preview.dart';
import 'package:minos/ui/features/social/widgets/conversation_system_message.dart';
import 'package:minos/ui/theme/theme.dart';

/// Collaboration timeline row — Desktop Slack/Buzz full-width left-aligned style.
class ConversationMessageRow extends StatelessWidget {
  const ConversationMessageRow({
    super.key,
    required this.message,
    required this.isMine,
    this.groupedWithPrevious = false,
    this.mentionsMe = false,
    this.onLongPress,
    this.onRetry,
    this.onOpenAgentSession,
    this.onToggleReaction,
  });

  final SocialChatMessage message;
  final bool isMine;
  final bool groupedWithPrevious;
  final bool mentionsMe;
  final VoidCallback? onLongPress;
  final VoidCallback? onRetry;
  final VoidCallback? onOpenAgentSession;
  final void Function(String emoji)? onToggleReaction;

  @override
  Widget build(BuildContext context) {
    if (message.isRecalled) {
      final text = isMine
          ? '你撤回了一条消息'
          : '${message.sender.displayName} 撤回了一条消息';
      return ConversationSystemMessage(text: text);
    }

    final isAgent = message.senderType == SenderType.agent;
    final authorLabel = isMine
        ? '我'
        : (message.sender.displayName.isEmpty
              ? (isAgent ? 'Agent' : '用户')
              : message.sender.displayName);
    final timeLabel = formatMessageClock(message.createdAtMs);
    final sessionId = message.agentSessionIdFromMessageId;
    final sessionShort = sessionId == null || sessionId.length < 4
        ? sessionId
        : sessionId.substring(sessionId.length - 4);

    final delivery = message.deliveryState;
    final isSending = delivery == SocialMessageDeliveryState.sending;
    final isFailed = delivery == SocialMessageDeliveryState.failed;

    String? deliveryLabel;
    var deliveryIsError = false;
    if (isSending) {
      deliveryLabel = '发送中';
    } else if (isFailed) {
      deliveryLabel = '失败';
      deliveryIsError = true;
    }

    final avatar = ConversationAvatarGutter(
      child: groupedWithPrevious
          ? const SizedBox(
              width: ConversationAvatarGutter.width,
              height: ConversationAvatarGutter.width,
            )
          : _MessageAvatar(
              label: message.sender.displayName,
              isMine: isMine,
              isAgent: isAgent,
              onTap: isAgent ? onOpenAgentSession : null,
            ),
    );

    final header = groupedWithPrevious
        ? null
        : ConversationMessageHeader(
            authorLabel: authorLabel,
            timeLabel: timeLabel,
            isAgent: isAgent,
            sessionShort: isAgent ? sessionShort : null,
            deliveryLabel: deliveryLabel,
            deliveryIsError: deliveryIsError,
            onAuthorTap: isAgent ? onOpenAgentSession : null,
          );

    final reply = message.replyTo;
    final body = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        if (mentionsMe) ...<Widget>[
          _MentionsMeChip(),
          const SizedBox(height: MinosSpacing.xs),
        ],
        if (reply != null) ...<Widget>[
          ConversationReplyPreview(
            senderName: reply.sender.displayName,
            text: reply.text,
            isRecalled: reply.recalledAtMs != null,
          ),
          const SizedBox(height: MinosSpacing.xs + 2),
        ],
        Opacity(
          opacity: isSending ? 0.7 : 1,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              if (isMine && isFailed) ...<Widget>[
                _RetryBadge(onTap: onRetry),
                const SizedBox(width: MinosSpacing.xs + 2),
              ],
              Expanded(
                child: _MessageMarkdownBody(
                  text: message.text,
                  dimmed: isSending,
                ),
              ),
            ],
          ),
        ),
        if (message.reactions.isNotEmpty ||
            onToggleReaction != null) ...<Widget>[
          const SizedBox(height: MinosSpacing.xs),
          _ReactionStrip(groups: message.reactions, onToggle: onToggleReaction),
        ],
      ],
    );

    return ConversationMessageChrome(
      avatar: avatar,
      header: header,
      body: body,
      groupedWithPrevious: groupedWithPrevious,
      mentionsMe: mentionsMe,
      onLongPress: onLongPress,
    );
  }
}

class _MentionsMeChip extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: const Color(0xFFF59E0B).withValues(alpha: 0.16),
        borderRadius: MinosRadii.pillAll,
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: MinosSpacing.sm,
          vertical: 3,
        ),
        child: Text(
          '提到了你',
          style: theme.textTheme.labelSmall?.copyWith(
            color: const Color(0xFFB45309),
            fontWeight: FontWeight.w700,
          ),
        ),
      ),
    );
  }
}

const _kQuickReactionEmojis = <String>['👍', '❤️', '🎉', '👀', '😄'];

class _ReactionStrip extends StatelessWidget {
  const _ReactionStrip({required this.groups, this.onToggle});

  final List<ReactionGroup> groups;
  final void Function(String emoji)? onToggle;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = context.minosColors;
    return Wrap(
      spacing: 6,
      runSpacing: 4,
      children: <Widget>[
        for (final g in groups)
          GestureDetector(
            onTap: onToggle == null ? null : () => onToggle!(g.emoji),
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: g.reactedByMe
                    ? colors.accent.withValues(alpha: 0.18)
                    : colors.surfaceMuted,
                borderRadius: MinosRadii.pillAll,
                border: g.reactedByMe
                    ? Border.all(color: colors.accent.withValues(alpha: 0.5))
                    : null,
              ),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                child: Text(
                  '${g.emoji} ${g.count}',
                  style: theme.textTheme.labelSmall,
                ),
              ),
            ),
          ),
        if (onToggle != null)
          GestureDetector(
            onTap: () => _showPicker(context),
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: colors.surfaceMuted,
                borderRadius: MinosRadii.pillAll,
              ),
              child: const Padding(
                padding: EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                child: Text('+'),
              ),
            ),
          ),
      ],
    );
  }

  void _showPicker(BuildContext context) {
    showModalBottomSheet<void>(
      context: context,
      builder: (ctx) {
        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.all(MinosSpacing.md),
            child: Wrap(
              spacing: 12,
              children: [
                for (final emoji in _kQuickReactionEmojis)
                  GestureDetector(
                    onTap: () {
                      Navigator.of(ctx).pop();
                      onToggle?.call(emoji);
                    },
                    child: Text(emoji, style: const TextStyle(fontSize: 28)),
                  ),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _RetryBadge extends StatelessWidget {
  const _RetryBadge({this.onTap});

  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: '发送失败，点击重试',
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          onTap: onTap,
          customBorder: const CircleBorder(),
          child: Container(
            width: 20,
            height: 20,
            alignment: Alignment.center,
            decoration: const BoxDecoration(
              color: Color(0xFFE11D48),
              shape: BoxShape.circle,
            ),
            child: const Text(
              '!',
              style: TextStyle(
                color: Colors.white,
                fontSize: 12,
                fontWeight: FontWeight.w800,
                height: 1,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _MessageAvatar extends StatelessWidget {
  const _MessageAvatar({
    required this.label,
    required this.isMine,
    required this.isAgent,
    this.onTap,
  });

  final String label;
  final bool isMine;
  final bool isAgent;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final bg = isAgent
        ? colors.accent.withValues(alpha: 0.14)
        : isMine
        ? colors.accent.withValues(alpha: 0.16)
        : colors.surfaceMuted;
    final fg = isAgent
        ? colors.accent
        : isMine
        ? colors.accent
        : colors.textSecondary;
    final glyph = isAgent
        ? Icon(CupertinoIcons.gear_alt_fill, size: 18, color: fg)
        : Text(
            _avatarInitials(label),
            style: theme.textTheme.labelMedium?.copyWith(
              color: fg,
              fontWeight: FontWeight.w800,
              height: 1,
            ),
          );

    final circle = Container(
      width: ConversationAvatarGutter.width,
      height: ConversationAvatarGutter.width,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: bg,
        shape: BoxShape.circle,
        border: Border.all(
          color: isAgent
              ? colors.accent.withValues(alpha: 0.28)
              : colors.border.withValues(alpha: 0.7),
        ),
      ),
      child: glyph,
    );

    if (onTap == null) return circle;
    return Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: onTap,
        customBorder: const CircleBorder(),
        child: circle,
      ),
    );
  }
}

String _avatarInitials(String label) {
  final cleaned = label.replaceAll(RegExp(r'[🤖\s]+'), ' ').trim();
  if (cleaned.isEmpty) return '?';
  final parts = cleaned
      .split(RegExp(r'\s+'))
      .where((p) => p.isNotEmpty)
      .toList();
  if (parts.length >= 2) {
    final a = parts[0].substring(0, 1);
    final b = parts[1].substring(0, 1);
    return '$a$b'.toUpperCase();
  }
  final end = cleaned.length >= 2 ? 2 : 1;
  return cleaned.substring(0, end).toUpperCase();
}

/// Markdown body with @mention emphasis (Desktop `MarkdownText` analogue).
class _MessageMarkdownBody extends StatelessWidget {
  const _MessageMarkdownBody({required this.text, this.dimmed = false});

  final String text;
  final bool dimmed;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final data = _emphasizeMentions(text);

    return Opacity(
      opacity: dimmed ? 0.85 : 1,
      child: MarkdownBody(
        data: data,
        selectable: false,
        softLineBreak: true,
        styleSheet: MarkdownStyleSheet.fromTheme(theme).copyWith(
          p: theme.textTheme.bodyMedium?.copyWith(
            color: colors.textPrimary,
            height: 1.45,
          ),
          pPadding: EdgeInsets.zero,
          strong: theme.textTheme.bodyMedium?.copyWith(
            color: colors.accent,
            fontWeight: FontWeight.w700,
            height: 1.45,
          ),
          code: theme.textTheme.bodySmall?.copyWith(
            fontFamily: 'monospace',
            color: colors.textPrimary,
            backgroundColor: colors.surfaceMuted,
          ),
          codeblockDecoration: BoxDecoration(
            color: colors.surfaceMuted,
            borderRadius: MinosRadii.smAll,
          ),
          blockquoteDecoration: BoxDecoration(
            color: colors.surfaceMuted.withValues(alpha: 0.6),
            border: Border(left: BorderSide(color: colors.border, width: 2)),
          ),
          a: TextStyle(
            color: colors.accent,
            decoration: TextDecoration.underline,
          ),
          listBullet: theme.textTheme.bodyMedium?.copyWith(
            color: colors.textPrimary,
          ),
        ),
      ),
    );
  }
}

/// Wrap `@token` mentions in `**…**` so markdown strong picks them up.
String _emphasizeMentions(String text) {
  if (text.isEmpty) return text;
  return text.replaceAllMapped(
    RegExp(r'@[A-Za-z0-9_\-./]+'),
    (match) => '**${match.group(0)!}**',
  );
}
