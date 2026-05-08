import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/social_providers.dart';
import 'package:minos/src/rust/api/minos.dart';

class SocialChatPage extends ConsumerStatefulWidget {
  const SocialChatPage({
    super.key,
    required this.conversationId,
    required this.title,
    required this.kind,
  });

  final String conversationId;
  final String title;
  final ConversationKind kind;

  @override
  ConsumerState<SocialChatPage> createState() => _SocialChatPageState();
}

class _SocialChatPageState extends ConsumerState<SocialChatPage> {
  final TextEditingController _controller = TextEditingController();
  final ScrollController _scrollController = ScrollController();

  @override
  void dispose() {
    _controller.dispose();
    _scrollController.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    final text = _controller.text.trim();
    if (text.isEmpty) {
      return;
    }
    try {
      await ref
          .read(socialConversationProvider(widget.conversationId).notifier)
          .sendMessage(text);
      if (!mounted) {
        return;
      }
      _controller.clear();
    } catch (error) {
      if (!mounted) {
        return;
      }
      _showError(context, '发送失败', error);
    }
  }

  void _insertMention(UserSummary member) {
    final mention = '@${member.minosId} ';
    final value = _controller.value;
    final selection = value.selection;
    final start = selection.isValid ? selection.start : value.text.length;
    final end = selection.isValid ? selection.end : value.text.length;
    final nextText = value.text.replaceRange(start, end, mention);
    final nextOffset = start + mention.length;
    _controller.value = TextEditingValue(
      text: nextText,
      selection: TextSelection.collapsed(offset: nextOffset),
    );
  }

  Future<void> _showMentionPicker(List<UserSummary> members) async {
    if (members.isEmpty) return;

    final selected = await showModalBottomSheet<UserSummary>(
      context: context,
      useSafeArea: true,
      builder: (context) => SafeArea(
        child: ListView(
          shrinkWrap: true,
          children: <Widget>[
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
              child: Text(
                '选择要艾特的成员',
                style: ShadTheme.of(context).textTheme.h4,
              ),
            ),
            for (final member in members)
              ListTile(
                title: Text(member.displayName),
                subtitle: Text('@${member.minosId}'),
                onTap: () => Navigator.of(context).pop(member),
              ),
          ],
        ),
      ),
    );
    if (selected != null) {
      _insertMention(selected);
    }
  }

  void _jumpToBottom() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_scrollController.hasClients) return;
      _scrollController.jumpTo(_scrollController.position.maxScrollExtent);
    });
  }

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    ref.listen<SocialConversationState>(
      socialConversationProvider(widget.conversationId),
      (previous, next) {
        final previousCount = previous?.messages.length ?? 0;
        if (next.messages.length > previousCount) {
          _jumpToBottom();
        }
      },
    );

    return Scaffold(
      backgroundColor: shadTheme.colorScheme.background,
      appBar: AppBar(
        title: Text(widget.title),
        surfaceTintColor: Colors.transparent,
      ),
      body: SafeArea(
        bottom: false,
        child: Column(
          children: <Widget>[
            Expanded(
              child: _ConversationMessagePane(
                conversationId: widget.conversationId,
                kind: widget.kind,
                scrollController: _scrollController,
              ),
            ),
            _ConversationComposer(
              conversationId: widget.conversationId,
              kind: widget.kind,
              controller: _controller,
              onSend: _send,
              onShowMentionPicker: _showMentionPicker,
            ),
          ],
        ),
      ),
    );
  }
}

class _ConversationMessagePane extends ConsumerWidget {
  const _ConversationMessagePane({
    required this.conversationId,
    required this.kind,
    required this.scrollController,
  });

  final String conversationId;
  final ConversationKind kind;
  final ScrollController scrollController;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(socialConversationProvider(conversationId));
    return RefreshIndicator(
      onRefresh: () => ref
          .read(socialConversationProvider(conversationId).notifier)
          .refresh(),
      child: state.isLoading && state.messages.isEmpty
          ? ListView(
              controller: scrollController,
              padding: const EdgeInsets.fromLTRB(12, 12, 12, 20),
              physics: const AlwaysScrollableScrollPhysics(),
              children: const <Widget>[],
            )
          : state.error != null && state.messages.isEmpty
          ? ListView(
              controller: scrollController,
              physics: const AlwaysScrollableScrollPhysics(),
              children: <Widget>[
                Padding(
                  padding: const EdgeInsets.all(24),
                  child: _ChatInlineError(
                    title: '聊天暂时不可用',
                    description: state.error.toString(),
                  ),
                ),
              ],
            )
          : ListView.builder(
              controller: scrollController,
              padding: const EdgeInsets.fromLTRB(12, 12, 12, 20),
              physics: const AlwaysScrollableScrollPhysics(),
              itemCount: state.messages.length,
              itemBuilder: (context, index) {
                final message = state.messages[index];
                final previous = index == 0 ? null : state.messages[index - 1];
                final showTimeSeparator = _shouldShowTimeSeparator(
                  previous?.createdAtMs,
                  message.createdAtMs,
                );
                final isMine = message.sender.accountId == state.myAccountId;
                final mentionsMe =
                    !isMine &&
                    state.myAccountId != null &&
                    message.mentionedAccountIds.contains(state.myAccountId);
                return Column(
                  mainAxisSize: MainAxisSize.min,
                  children: <Widget>[
                    if (showTimeSeparator)
                      _ChatTimeSeparator(
                        label: _formatTimelineLabel(message.createdAtMs),
                      ),
                    _ChatBubble(
                      title: kind == ConversationKind.group
                          ? message.sender.displayName
                          : null,
                      text: message.text,
                      isMine: isMine,
                      mentionsMe: mentionsMe,
                    ),
                  ],
                );
              },
            ),
    );
  }
}

class _ConversationComposer extends ConsumerWidget {
  const _ConversationComposer({
    required this.conversationId,
    required this.kind,
    required this.controller,
    required this.onSend,
    required this.onShowMentionPicker,
  });

  final String conversationId;
  final ConversationKind kind;
  final TextEditingController controller;
  final VoidCallback onSend;
  final Future<void> Function(List<UserSummary> members) onShowMentionPicker;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final shadTheme = ShadTheme.of(context);
    final isSending = ref.watch(
      socialConversationProvider(conversationId).select(
        (SocialConversationState state) => state.isSending,
      ),
    );
    final myAccountId = ref.watch(
      socialConversationProvider(conversationId).select(
        (SocialConversationState state) => state.myAccountId,
      ),
    );
    final groupMembers = kind == ConversationKind.group
        ? ref.watch(conversationMembersProvider(conversationId)).asData?.value ??
              const <UserSummary>[]
        : const <UserSummary>[];
    final mentionable = groupMembers
        .where((member) => member.accountId != myAccountId)
        .toList(growable: false);

    return Container(
      decoration: BoxDecoration(
        color: shadTheme.colorScheme.background,
        border: Border(top: BorderSide(color: shadTheme.colorScheme.border)),
      ),
      padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
      child: Row(
        children: <Widget>[
          if (kind == ConversationKind.group) ...<Widget>[
            ShadButton.outline(
              onPressed: mentionable.isEmpty
                  ? null
                  : () => onShowMentionPicker(mentionable),
              child: const Text('@'),
            ),
            const SizedBox(width: 8),
          ],
          Expanded(
            child: ShadInput(
              controller: controller,
              minLines: 1,
              maxLines: 4,
              placeholder: const Text('发送消息...'),
              onSubmitted: (_) => onSend(),
            ),
          ),
          const SizedBox(width: 10),
          ShadButton(
            onPressed: isSending ? null : onSend,
            child: isSending
                ? const SizedBox.square(
                    dimension: 14,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Text('发送'),
          ),
        ],
      ),
    );
  }
}

class _ChatInlineError extends StatelessWidget {
  const _ChatInlineError({required this.title, required this.description});

  final String title;
  final String description;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: <Widget>[
        Icon(
          LucideIcons.circleAlert,
          size: 36,
          color: shadTheme.colorScheme.mutedForeground,
        ),
        const SizedBox(height: 10),
        Text(title, style: shadTheme.textTheme.h4),
        const SizedBox(height: 6),
        Text(
          description,
          textAlign: TextAlign.center,
          style: shadTheme.textTheme.muted,
        ),
      ],
    );
  }
}

class _ChatBubble extends StatelessWidget {
  const _ChatBubble({
    required this.text,
    required this.isMine,
    this.title,
    this.mentionsMe = false,
  });

  final String? title;
  final String text;
  final bool isMine;
  final bool mentionsMe;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    final bubbleColor = isMine
        ? shadTheme.colorScheme.primary
        : shadTheme.colorScheme.secondary;
    final foreground = isMine
        ? shadTheme.colorScheme.primaryForeground
        : shadTheme.colorScheme.secondaryForeground;
    final mentionAccent = isMine
        ? foreground.withValues(alpha: 0.88)
        : shadTheme.colorScheme.primary;
    return Padding(
      padding: EdgeInsets.fromLTRB(isMine ? 52 : 0, 0, isMine ? 0 : 52, 12),
      child: Align(
        alignment: isMine ? Alignment.centerRight : Alignment.centerLeft,
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxWidth: MediaQuery.of(context).size.width * 0.76,
          ),
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: bubbleColor,
              borderRadius: BorderRadius.circular(12),
              border: Border.all(
                color: mentionsMe
                    ? const Color(0xFFF59E0B)
                    : isMine
                    ? bubbleColor
                    : shadTheme.colorScheme.border.withValues(alpha: 0.9),
              ),
            ),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  if (mentionsMe) ...<Widget>[
                    DecoratedBox(
                      decoration: BoxDecoration(
                        color: const Color(0xFFF59E0B).withValues(alpha: 0.16),
                        borderRadius: BorderRadius.circular(999),
                      ),
                      child: Padding(
                        padding: const EdgeInsets.symmetric(
                          horizontal: 8,
                          vertical: 3,
                        ),
                        child: Text(
                          '提到了你',
                          style: shadTheme.textTheme.small.copyWith(
                            color: const Color(0xFFB45309),
                            fontWeight: FontWeight.w700,
                          ),
                        ),
                      ),
                    ),
                    const SizedBox(height: 6),
                  ],
                  if (title != null) ...<Widget>[
                    Text(
                      title!,
                      style: shadTheme.textTheme.small.copyWith(
                        color: foreground.withValues(alpha: 0.8),
                      ),
                    ),
                    const SizedBox(height: 4),
                  ],
                  RichText(
                    text: TextSpan(
                      style: TextStyle(color: foreground, height: 1.35),
                      children: _buildMentionSpans(
                        text,
                        foreground,
                        mentionAccent,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

List<InlineSpan> _buildMentionSpans(
  String text,
  Color foreground,
  Color accent,
) {
  final pattern = RegExp(r'@[A-Za-z0-9]+');
  final matches = pattern.allMatches(text);
  if (matches.isEmpty) {
    return <InlineSpan>[TextSpan(text: text)];
  }

  final spans = <InlineSpan>[];
  var cursor = 0;
  for (final match in matches) {
    if (match.start > cursor) {
      spans.add(TextSpan(text: text.substring(cursor, match.start)));
    }
    spans.add(
      TextSpan(
        text: match.group(0),
        style: TextStyle(color: accent, fontWeight: FontWeight.w700),
      ),
    );
    cursor = match.end;
  }
  if (cursor < text.length) {
    spans.add(TextSpan(text: text.substring(cursor)));
  }
  return spans;
}

class _ChatTimeSeparator extends StatelessWidget {
  const _ChatTimeSeparator({required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 6, 12, 14),
      child: Center(
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: shadTheme.colorScheme.muted,
            borderRadius: BorderRadius.circular(999),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
            child: Text(label, style: shadTheme.textTheme.muted),
          ),
        ),
      ),
    );
  }
}

bool _shouldShowTimeSeparator(int? previousTsMs, int currentTsMs) {
  if (previousTsMs == null) return true;
  return currentTsMs - previousTsMs >=
      const Duration(minutes: 1).inMilliseconds;
}

String _formatTimelineLabel(int tsMs) {
  final date = DateTime.fromMillisecondsSinceEpoch(tsMs, isUtc: false);
  final now = DateTime.now();
  if (_isSameDay(date, now)) {
    return _formatClock(date);
  }

  if (date.year == now.year) {
    return '${date.month}月${date.day}日 ${_formatClock(date)}';
  }

  return '${date.year}年${date.month}月${date.day}日 ${_formatClock(date)}';
}

String _formatClock(DateTime date) {
  final hh = date.hour.toString().padLeft(2, '0');
  final mm = date.minute.toString().padLeft(2, '0');
  return '$hh:$mm';
}

bool _isSameDay(DateTime lhs, DateTime rhs) {
  return lhs.year == rhs.year && lhs.month == rhs.month && lhs.day == rhs.day;
}

void _showError(BuildContext context, String title, Object error) {
  ShadToaster.maybeOf(context)?.show(
    ShadToast.destructive(
      title: Text(title),
      description: Text(error.toString()),
    ),
  );
}
