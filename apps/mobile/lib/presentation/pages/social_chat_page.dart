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

  List<ChatMessageSummary> _messages = const <ChatMessageSummary>[];
  String? _myAccountId;
  bool _loading = true;
  bool _sending = false;
  Object? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _controller.dispose();
    _scrollController.dispose();
    super.dispose();
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
      if (!mounted) return;
      setState(() {
        _myAccountId = profile.accountId;
        _messages = response.messages;
        _loading = false;
      });
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
        _messages = <ChatMessageSummary>[..._messages, message];
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

  void _jumpToBottom() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_scrollController.hasClients) return;
      _scrollController.jumpTo(_scrollController.position.maxScrollExtent);
    });
  }

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
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
                    ? const Center(child: ShadProgress())
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
                          final isMine =
                              message.sender.accountId == _myAccountId;
                          return _ChatBubble(
                            title: widget.kind == ConversationKind.group
                                ? message.sender.displayName
                                : null,
                            text: message.text,
                            timestamp: _formatTime(message.createdAtMs),
                            isMine: isMine,
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
    required this.timestamp,
    required this.isMine,
    this.title,
  });

  final String? title;
  final String text;
  final String timestamp;
  final bool isMine;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    final bubbleColor = isMine
        ? shadTheme.colorScheme.primary
        : shadTheme.colorScheme.card;
    final foreground = isMine
        ? shadTheme.colorScheme.primaryForeground
        : shadTheme.colorScheme.foreground;
    return Padding(
      padding: EdgeInsets.fromLTRB(isMine ? 52 : 0, 0, isMine ? 0 : 52, 12),
      child: Align(
        alignment: isMine ? Alignment.centerRight : Alignment.centerLeft,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: bubbleColor,
            borderRadius: BorderRadius.circular(10),
          ),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(12, 10, 12, 8),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                if (title != null) ...<Widget>[
                  Text(
                    title!,
                    style: shadTheme.textTheme.small.copyWith(
                      color: foreground.withValues(alpha: 0.78),
                    ),
                  ),
                  const SizedBox(height: 4),
                ],
                Text(text, style: TextStyle(color: foreground, height: 1.35)),
                const SizedBox(height: 6),
                Text(
                  timestamp,
                  style: shadTheme.textTheme.muted.copyWith(
                    color: foreground.withValues(alpha: 0.72),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

String _formatTime(int tsMs) {
  final date = DateTime.fromMillisecondsSinceEpoch(tsMs, isUtc: false);
  final hh = date.hour.toString().padLeft(2, '0');
  final mm = date.minute.toString().padLeft(2, '0');
  return '$hh:$mm';
}

void _showError(BuildContext context, String title, Object error) {
  ShadToaster.maybeOf(context)?.show(
    ShadToast.destructive(
      title: Text(title),
      description: Text(error.toString()),
    ),
  );
}
