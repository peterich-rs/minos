import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/social_providers.dart';
import 'package:minos/application/minos_providers.dart';
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
  StreamSubscription<SocialEventFrame>? _socialSub;

  List<ChatMessageSummary> _messages = const <ChatMessageSummary>[];
  String? _myAccountId;
  bool _loading = true;
  bool _sending = false;
  Object? _error;

  @override
  void initState() {
    super.initState();
    _socialSub = ref
        .read(minosCoreProvider)
        .socialEvents
        .listen(_onSocialEvent);
    _load();
  }

  @override
  void dispose() {
    _socialSub?.cancel();
    _controller.dispose();
    _scrollController.dispose();
    super.dispose();
  }

  void _onSocialEvent(SocialEventFrame frame) {
    if (!mounted || frame.conversationId != widget.conversationId) return;
    final changed = _appendMessage(frame.message);
    if (changed) {
      unawaited(_markConversationRead());
      ref.invalidate(conversationsProvider);
      _jumpToBottom();
    }
  }

  Future<void> _markConversationRead() async {
    try {
      await ref
          .read(minosCoreProvider)
          .markConversationRead(conversationId: widget.conversationId);
      if (mounted) {
        ref.invalidate(conversationsProvider);
      }
    } catch (_) {}
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final core = ref.read(minosCoreProvider);
      final profile = await core.myProfile();
      final response = await core.listChatMessages(
        conversationId: widget.conversationId,
        limit: 100,
      );
      await core.markConversationRead(conversationId: widget.conversationId);
      if (!mounted) return;
      setState(() {
        _myAccountId = profile.accountId;
        _messages = response.messages;
        _loading = false;
      });
      ref.invalidate(conversationsProvider);
      _jumpToBottom();
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error;
        _loading = false;
      });
    }
  }

  Future<void> _send() async {
    final text = _controller.text.trim();
    if (text.isEmpty || _sending) return;
    setState(() => _sending = true);
    try {
      final message = await ref
          .read(minosCoreProvider)
          .sendChatMessage(conversationId: widget.conversationId, text: text);
      if (!mounted) return;
      _controller.clear();
      setState(() {
        _messages = _mergeMessage(_messages, message);
        _sending = false;
      });
      _jumpToBottom();
      ref.invalidate(conversationsProvider);
    } catch (error) {
      if (!mounted) return;
      setState(() => _sending = false);
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
    final mentionable = members
        .where((member) => member.accountId != _myAccountId)
        .toList(growable: false);
    if (mentionable.isEmpty) return;

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
            for (final member in mentionable)
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

  bool _appendMessage(ChatMessageSummary message) {
    final next = _mergeMessage(_messages, message);
    if (identical(next, _messages)) return false;
    setState(() => _messages = next);
    return true;
  }

  List<ChatMessageSummary> _mergeMessage(
    List<ChatMessageSummary> existing,
    ChatMessageSummary incoming,
  ) {
    if (existing.any((message) => message.messageId == incoming.messageId)) {
      return existing;
    }
    return <ChatMessageSummary>[...existing, incoming];
  }

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    final groupMembers = widget.kind == ConversationKind.group
      ? ref
            .watch(conversationMembersProvider(widget.conversationId))
            .asData
            ?.value ??
          const <UserSummary>[]
        : const <UserSummary>[];
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
              child: RefreshIndicator(
                onRefresh: _load,
                child: _loading
                    ? ListView(
                        controller: _scrollController,
                        padding: const EdgeInsets.fromLTRB(12, 12, 12, 20),
                        physics: const AlwaysScrollableScrollPhysics(),
                        children: const <Widget>[],
                      )
                    : _error != null
                    ? ListView(
                        children: <Widget>[
                          Padding(
                            padding: const EdgeInsets.all(24),
                            child: _ChatInlineError(
                              title: '聊天暂时不可用',
                              description: _error.toString(),
                            ),
                          ),
                        ],
                      )
                    : ListView.builder(
                        controller: _scrollController,
                        padding: const EdgeInsets.fromLTRB(12, 12, 12, 20),
                        itemCount: _messages.length,
                        itemBuilder: (context, index) {
                          final message = _messages[index];
                          final previous = index == 0
                              ? null
                              : _messages[index - 1];
                          final showTimeSeparator = _shouldShowTimeSeparator(
                            previous?.createdAtMs,
                            message.createdAtMs,
                          );
                          final isMine =
                              message.sender.accountId == _myAccountId;
                          final mentionsMe =
                              !isMine &&
                              _myAccountId != null &&
                              message.mentionedAccountIds.contains(_myAccountId);
                          return Column(
                            mainAxisSize: MainAxisSize.min,
                            children: <Widget>[
                              if (showTimeSeparator)
                                _ChatTimeSeparator(
                                  label: _formatTimelineLabel(
                                    message.createdAtMs,
                                  ),
                                ),
                              _ChatBubble(
                                title: widget.kind == ConversationKind.group
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
              ),
            ),
            Container(
              decoration: BoxDecoration(
                color: shadTheme.colorScheme.background,
                border: Border(
                  top: BorderSide(color: shadTheme.colorScheme.border),
                ),
              ),
              padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
              child: Row(
                children: <Widget>[
                  if (widget.kind == ConversationKind.group) ...<Widget>[
                    ShadButton.outline(
                      onPressed: groupMembers.isEmpty
                          ? null
                          : () => _showMentionPicker(groupMembers),
                      child: const Text('@'),
                    ),
                    const SizedBox(width: 8),
                  ],
                  Expanded(
                    child: ShadInput(
                      controller: _controller,
                      minLines: 1,
                      maxLines: 4,
                      placeholder: const Text('发送消息...'),
                      onSubmitted: (_) => _send(),
                    ),
                  ),
                  const SizedBox(width: 10),
                  ShadButton(
                    onPressed: _sending ? null : _send,
                    child: _sending
                        ? const SizedBox.square(
                            dimension: 14,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Text('发送'),
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
