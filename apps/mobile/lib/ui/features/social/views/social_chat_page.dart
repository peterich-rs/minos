import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_list_view/flutter_list_view.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/agent_activity_provider.dart';
import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

class SocialChatPage extends ConsumerStatefulWidget {
  const SocialChatPage({
    super.key,
    required this.conversationId,
    required this.title,
    required this.kind,
  });

  final String conversationId;
  final String title;
  final ConversationKind? kind;

  @override
  ConsumerState<SocialChatPage> createState() => _SocialChatPageState();
}

class _SocialChatPageState extends ConsumerState<SocialChatPage> {
  static const double _bottomStickThreshold = 120;
  static const Duration _keyboardRevealDelay = Duration(milliseconds: 120);
  static const Duration _keyboardRevealSettleDelay = Duration(
    milliseconds: 260,
  );
  static const Duration _initialBottomSettleDelay = Duration(milliseconds: 80);
  static const Duration _initialBottomFinalSettleDelay = Duration(
    milliseconds: 240,
  );

  final TextEditingController _controller = TextEditingController();
  final FocusNode _composerFocusNode = FocusNode();
  final FlutterListViewController _scrollController =
      FlutterListViewController();
  double _lastKeyboardInsetBottom = 0;
  int _bottomAnchorGeneration = 0;

  @override
  void dispose() {
    _controller.dispose();
    _composerFocusNode.dispose();
    _scrollController.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    final text = _controller.text.trim();
    if (text.isEmpty) {
      return;
    }
    final replyTarget = ref.read(
      socialReplyMessageProvider(widget.conversationId),
    );
    _controller.clear();
    ref.read(socialReplyDraftProvider(widget.conversationId).notifier).clear();
    _jumpToBottom();
    try {
      await ref
          .read(socialConversationProvider(widget.conversationId).notifier)
          .sendMessage(text, replyToMessage: replyTarget);
      if (!mounted) {
        return;
      }
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

  void _insertAgentMention(AgentProfile agent) {
    final mention = '@${agent.agentId} ';
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
    final groupAgents = ref.read(groupAgentsProvider(widget.conversationId));
    if (members.isEmpty && groupAgents.isEmpty) return;

    final selected = await showModalBottomSheet<_MentionSelection>(
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
            if (groupAgents.isNotEmpty) ...<Widget>[
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
                child: Text(
                  'Agents',
                  style: ShadTheme.of(context).textTheme.small.copyWith(
                    color: ShadTheme.of(context).colorScheme.mutedForeground,
                  ),
                ),
              ),
              for (final agent in groupAgents)
                ListTile(
                  leading: const Icon(LucideIcons.bot, size: 20),
                  title: Text('🤖 ${agent.name}'),
                  subtitle: Text('@${agent.agentId}'),
                  onTap: () =>
                      Navigator.of(context).pop(_MentionSelection.agent(agent)),
                ),
              const Divider(),
            ],
            if (members.isNotEmpty) ...<Widget>[
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
                child: Text(
                  '成员',
                  style: ShadTheme.of(context).textTheme.small.copyWith(
                    color: ShadTheme.of(context).colorScheme.mutedForeground,
                  ),
                ),
              ),
              for (final member in members)
                ListTile(
                  title: Text(member.displayName),
                  subtitle: Text('@${member.minosId}'),
                  onTap: () =>
                      Navigator.of(context).pop(_MentionSelection.user(member)),
                ),
            ],
          ],
        ),
      ),
    );
    if (selected != null) {
      switch (selected) {
        case _MentionSelectionUser(:final user):
          _insertMention(user);
        case _MentionSelectionAgent(:final agent):
          _insertAgentMention(agent);
      }
    }
  }

  void _jumpToBottom({int? messageCount, bool settle = false}) {
    final generation = ++_bottomAnchorGeneration;

    void jump() {
      if (!mounted || !_scrollController.hasClients) return;
      if (messageCount != null && messageCount > 0) {
        _scrollController.sliverController.jumpToIndex(
          messageCount - 1,
          offsetBasedOnBottom: true,
        );
        return;
      }
      _scrollController.jumpTo(_scrollController.position.maxScrollExtent);
    }

    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (generation != _bottomAnchorGeneration) return;
      jump();
    });
    if (!settle) return;
    unawaited(
      Future<void>.delayed(_initialBottomSettleDelay, () {
        if (generation != _bottomAnchorGeneration) return;
        jump();
      }),
    );
    unawaited(
      Future<void>.delayed(_initialBottomFinalSettleDelay, () {
        if (generation != _bottomAnchorGeneration) return;
        jump();
      }),
    );
  }

  bool _isNearBottom() {
    if (!_scrollController.hasClients) return true;
    final position = _scrollController.position;
    return position.maxScrollExtent - position.pixels <= _bottomStickThreshold;
  }

  void _syncKeyboardInset(double keyboardInsetBottom) {
    if ((_lastKeyboardInsetBottom - keyboardInsetBottom).abs() < 0.5) return;
    final shouldStick = _composerFocusNode.hasFocus || _isNearBottom();
    _lastKeyboardInsetBottom = keyboardInsetBottom;
    if (!shouldStick) return;

    _jumpToBottom();
    unawaited(Future<void>.delayed(_keyboardRevealDelay, _jumpToBottom));
    unawaited(Future<void>.delayed(_keyboardRevealSettleDelay, _jumpToBottom));
  }

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    final keyboardInsetBottom = MediaQuery.of(context).viewInsets.bottom;
    final conversationSummary = _findConversationSummary(
      ref.watch(conversationsProvider).asData?.value,
      widget.conversationId,
    );
    final effectiveKind = conversationSummary?.kind ?? widget.kind;
    final effectiveTitle = _firstNonEmpty(<String?>[
      conversationSummary?.title,
      widget.title,
    ]);
    _syncKeyboardInset(keyboardInsetBottom);

    ref.listen<SocialConversationState>(
      socialConversationProvider(widget.conversationId),
      (previous, next) {
        final previousCount = previous?.messages.length ?? 0;
        if (next.messages.length > previousCount) {
          _jumpToBottom(
            messageCount: next.messages.length,
            settle: previousCount == 0,
          );
        }
      },
    );

    return Scaffold(
      resizeToAvoidBottomInset: false,
      backgroundColor: shadTheme.colorScheme.background,
      appBar: AppBar(
        toolbarHeight: 64,
        centerTitle: true,
        titleSpacing: 0,
        title: SizedBox(
          width: double.infinity,
          child: _ConversationTitle(
            conversationId: widget.conversationId,
            title: effectiveTitle,
            kind: effectiveKind,
          ),
        ),
        surfaceTintColor: Colors.transparent,
        actions: <Widget>[
          SizedBox(
            width: kToolbarHeight,
            child: effectiveKind == ConversationKind.group
                ? IconButton(
                    icon: const Icon(LucideIcons.users),
                    tooltip: '群成员',
                    onPressed: () => context.push(
                      '/social/chat/${widget.conversationId}/members',
                      extra: GroupMembersRouteExtra(title: effectiveTitle),
                    ),
                  )
                : const SizedBox.shrink(),
          ),
        ],
      ),
      body: SafeArea(
        bottom: false,
        child: Column(
          children: <Widget>[
            Expanded(
              child: GestureDetector(
                behavior: HitTestBehavior.translucent,
                onTap: _composerFocusNode.unfocus,
                child: _ConversationMessagePane(
                  conversationId: widget.conversationId,
                  kind: effectiveKind,
                  scrollController: _scrollController,
                ),
              ),
            ),
            AnimatedPadding(
              duration: const Duration(milliseconds: 220),
              curve: Curves.easeOutCubic,
              padding: EdgeInsets.only(bottom: keyboardInsetBottom),
              child: SafeArea(
                top: false,
                child: _ConversationComposer(
                  conversationId: widget.conversationId,
                  kind: effectiveKind,
                  controller: _controller,
                  focusNode: _composerFocusNode,
                  onSend: _send,
                  onShowMentionPicker: _showMentionPicker,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ConversationTitle extends ConsumerWidget {
  const _ConversationTitle({
    required this.conversationId,
    required this.title,
    required this.kind,
  });

  final String conversationId;
  final String title;
  final ConversationKind? kind;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final shadTheme = ShadTheme.of(context);
    final conversation = ref.watch(socialConversationProvider(conversationId));
    final conversationSummary = _findConversationSummary(
      ref.watch(conversationsProvider).asData?.value,
      conversationId,
    );
    final effectiveKind = conversationSummary?.kind ?? kind;
    final agents = effectiveKind == ConversationKind.group
        ? ref.watch(groupAgentsProvider(conversationId))
        : const <AgentProfile>[];
    final hosts =
        ref.watch(pairedMacsProvider).asData?.value ?? const <HostSummaryDto>[];
    final activeHostId = ref.watch(activeMacProvider).asData?.value;
    final connectionState = ref.watch(connectionStateProvider).asData?.value;
    final status = _resolveConversationStatus(
      conversation: conversation,
      agents: agents,
      hosts: hosts,
      activeHostId: activeHostId,
      connectionState: connectionState,
    );
    final resolvedTitle = _resolveConversationTitle(
      routeTitle: title,
      kind: effectiveKind,
      summary: conversationSummary,
      conversation: conversation,
    );
    final color = _conversationStatusColor(context, status);

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.center,
      children: <Widget>[
        Text(
          resolvedTitle,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          textAlign: TextAlign.center,
          style: Theme.of(context).textTheme.titleLarge?.copyWith(
            color: Theme.of(context).colorScheme.onSurface,
            fontWeight: FontWeight.w700,
            fontSize: 18,
            height: 1.1,
          ),
        ),
        const SizedBox(height: 2),
        Row(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            DecoratedBox(
              decoration: BoxDecoration(color: color, shape: BoxShape.circle),
              child: const SizedBox(width: 7, height: 7),
            ),
            const SizedBox(width: 6),
            Text(
              status.label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.center,
              style: shadTheme.textTheme.small.copyWith(
                color: color,
                fontSize: 11,
                fontWeight: FontWeight.w700,
                height: 1.05,
              ),
            ),
          ],
        ),
      ],
    );
  }
}

ConversationSummary? _findConversationSummary(
  ConversationsResponse? response,
  String conversationId,
) {
  if (response == null) return null;
  for (final conversation in response.conversations) {
    if (conversation.conversationId == conversationId) {
      return conversation;
    }
  }
  return null;
}

String _resolveConversationTitle({
  required String routeTitle,
  required ConversationKind? kind,
  required ConversationSummary? summary,
  required SocialConversationState conversation,
}) {
  final route = routeTitle.trim();
  final summaryTitle = summary?.title.trim();

  if (kind == ConversationKind.direct) {
    return _firstNonEmpty(<String?>[
      _userTitle(summary?.counterpart),
      route,
      summaryTitle,
      _counterpartTitleFromMessages(conversation),
      '私聊',
    ]);
  }

  return _firstNonEmpty(<String?>[
    route,
    summaryTitle,
    _counterpartTitleFromMessages(conversation),
    '群聊',
  ]);
}

String? _counterpartTitleFromMessages(SocialConversationState conversation) {
  for (final message in conversation.messages.reversed) {
    if (conversation.myAccountId != null &&
        message.sender.accountId == conversation.myAccountId) {
      continue;
    }
    if (conversation.myAccountId == null &&
        (message.sender.minosId == 'me' ||
            message.sender.displayName.trim() == '我')) {
      continue;
    }
    return _userTitle(message.sender);
  }
  return null;
}

String? _userTitle(UserSummary? user) {
  if (user == null) return null;
  return _firstNonEmpty(<String?>[
    user.displayName.trim(),
    user.minosId.trim(),
    user.accountId.trim(),
  ]);
}

String _firstNonEmpty(List<String?> candidates) {
  for (final candidate in candidates) {
    final value = candidate?.trim();
    if (value != null && value.isNotEmpty) {
      return value;
    }
  }
  return '';
}

class _ConversationMessagePane extends ConsumerWidget {
  const _ConversationMessagePane({
    required this.conversationId,
    required this.kind,
    required this.scrollController,
  });

  final String conversationId;
  final ConversationKind? kind;
  final FlutterListViewController scrollController;

  Future<void> _showMessageActions(
    BuildContext context,
    WidgetRef ref, {
    required SocialChatMessage message,
    required bool isMine,
  }) async {
    final canReply = message.canReply;
    final canRecall = isMine && message.canRecall;
    if (!canReply && !canRecall) {
      return;
    }

    final action = await showModalBottomSheet<_MessageAction>(
      context: context,
      useSafeArea: true,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            if (canReply)
              ListTile(
                leading: const Icon(LucideIcons.reply),
                title: const Text('引用消息'),
                onTap: () => Navigator.of(context).pop(_MessageAction.reply),
              ),
            if (canRecall)
              ListTile(
                leading: const Icon(LucideIcons.undo2),
                title: const Text('撤回消息'),
                onTap: () => Navigator.of(context).pop(_MessageAction.recall),
              ),
          ],
        ),
      ),
    );
    if (!context.mounted || action == null) {
      return;
    }

    switch (action) {
      case _MessageAction.reply:
        ref
            .read(socialReplyDraftProvider(conversationId).notifier)
            .select(message.localId);
        return;
      case _MessageAction.recall:
        final confirmed = await showDialog<bool>(
          context: context,
          builder: (dialogContext) => AlertDialog(
            title: const Text('撤回这条消息？'),
            content: const Text('撤回后，对话中会显示该消息已被撤回。'),
            actions: <Widget>[
              TextButton(
                onPressed: () => Navigator.of(dialogContext).pop(false),
                child: const Text('取消'),
              ),
              TextButton(
                onPressed: () => Navigator.of(dialogContext).pop(true),
                child: const Text('撤回'),
              ),
            ],
          ),
        );
        if (confirmed != true || !context.mounted) {
          return;
        }
        try {
          await ref
              .read(socialConversationProvider(conversationId).notifier)
              .recallMessage(message.localId);
        } catch (error) {
          if (!context.mounted) {
            return;
          }
          _showError(context, '撤回失败', error);
        }
        return;
    }
  }

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
              keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
              children: const <Widget>[],
            )
          : state.error != null && state.messages.isEmpty
          ? ListView(
              controller: scrollController,
              physics: const AlwaysScrollableScrollPhysics(),
              keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
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
          : FlutterListView(
              controller: scrollController,
              physics: const AlwaysScrollableScrollPhysics(),
              keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
              delegate: FlutterListViewDelegate(
                (context, index) {
                  final message = state.messages[index];
                  final previous = index == 0
                      ? null
                      : state.messages[index - 1];
                  final showTimeSeparator = _shouldShowTimeSeparator(
                    previous?.createdAtMs,
                    message.createdAtMs,
                  );
                  final isMine = message.sender.accountId == state.myAccountId;
                  final isAgent = message.senderType == SenderType.agent;
                  final mentionsMe =
                      !isMine &&
                      state.myAccountId != null &&
                      message.mentionedAccountIds.contains(state.myAccountId);
                  final retryAction =
                      message.deliveryState == SocialMessageDeliveryState.failed
                      ? () => ref
                            .read(
                              socialConversationProvider(
                                conversationId,
                              ).notifier,
                            )
                            .retryMessage(message.localId)
                      : null;
                  final actionHandler =
                      message.canReply || (isMine && message.canRecall)
                      ? () => _showMessageActions(
                          context,
                          ref,
                          message: message,
                          isMine: isMine,
                        )
                      : null;
                  return Padding(
                    padding: EdgeInsets.fromLTRB(
                      12,
                      index == 0 ? 12 : 0,
                      12,
                      index == state.messages.length - 1 ? 20 : 0,
                    ),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: <Widget>[
                        if (showTimeSeparator)
                          _ChatTimeSeparator(
                            label: _formatTimelineLabel(message.createdAtMs),
                          ),
                        _ChatBubble(
                          title: kind == ConversationKind.group || isAgent
                              ? message.sender.displayName
                              : null,
                          senderName: message.sender.displayName,
                          text: message.text,
                          isMine: isMine,
                          isAgent: isAgent,
                          mentionsMe: mentionsMe,
                          replyTo: message.replyTo,
                          recalledAtMs: message.recalledAtMs,
                          deliveryState: message.deliveryState,
                          onRetry: retryAction,
                          onLongPress: actionHandler,
                        ),
                      ],
                    ),
                  );
                },
                childCount: state.messages.length,
                onItemKey: (index) => state.messages[index].localId,
                keepPosition: true,
                keepPositionOffset: 80,
              ),
            ),
    );
  }
}

class _ConversationComposer extends ConsumerWidget {
  const _ConversationComposer({
    required this.conversationId,
    required this.kind,
    required this.controller,
    required this.focusNode,
    required this.onSend,
    required this.onShowMentionPicker,
  });

  final String conversationId;
  final ConversationKind? kind;
  final TextEditingController controller;
  final FocusNode focusNode;
  final VoidCallback onSend;
  final Future<void> Function(List<UserSummary> members) onShowMentionPicker;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final shadTheme = ShadTheme.of(context);
    final replyTarget = ref.watch(socialReplyMessageProvider(conversationId));
    final myAccountId = ref.watch(
      socialConversationProvider(
        conversationId,
      ).select((SocialConversationState state) => state.myAccountId),
    );
    final groupMembers = kind == ConversationKind.group
        ? ref
                  .watch(conversationMembersProvider(conversationId))
                  .asData
                  ?.value ??
              const <UserSummary>[]
        : const <UserSummary>[];
    final mentionable = groupMembers
        .where((member) => member.accountId != myAccountId)
        .toList(growable: false);
    final groupAgents = kind == ConversationKind.group
        ? ref.watch(groupAgentsProvider(conversationId))
        : const <AgentProfile>[];
    final hasMentionable = mentionable.isNotEmpty || groupAgents.isNotEmpty;
    final activity = ref
        .watch(conversationAgentActivityProvider(conversationId))
        .asData
        ?.value;

    return Container(
      decoration: BoxDecoration(
        color: shadTheme.colorScheme.background,
        border: Border(top: BorderSide(color: shadTheme.colorScheme.border)),
      ),
      padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: <Widget>[
          if (replyTarget != null) ...<Widget>[
            _ComposerReplyBanner(
              senderName: replyTarget.sender.displayName,
              text: replyTarget.text,
              isRecalled: replyTarget.recalledAtMs != null,
              onClear: () => ref
                  .read(socialReplyDraftProvider(conversationId).notifier)
                  .clear(),
            ),
            const SizedBox(height: 10),
          ],
          if (activity != null) ...<Widget>[
            _AgentActivityTicker(activity: activity),
            const SizedBox(height: 8),
          ],
          Row(
            children: <Widget>[
              if (kind == ConversationKind.group) ...<Widget>[
                ShadButton.outline(
                  onPressed: !hasMentionable
                      ? null
                      : () => onShowMentionPicker(mentionable),
                  child: const Text('@'),
                ),
                const SizedBox(width: 8),
              ],
              Expanded(
                child: ShadInput(
                  controller: controller,
                  focusNode: focusNode,
                  minLines: 1,
                  maxLines: 4,
                  placeholder: const Text('发送消息...'),
                  onSubmitted: (_) => onSend(),
                ),
              ),
              const SizedBox(width: 10),
              ShadButton(onPressed: onSend, child: const Text('发送')),
            ],
          ),
        ],
      ),
    );
  }
}

class _AgentActivityTicker extends StatelessWidget {
  const _AgentActivityTicker({required this.activity});

  final AgentActivitySnapshot activity;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    final toneColor = _activityToneColor(context, activity.tone);
    final textStyle = Theme.of(context).textTheme.labelMedium?.copyWith(
      color: shadTheme.colorScheme.foreground,
      fontWeight: FontWeight.w600,
      height: 1.1,
    );
    return AnimatedSwitcher(
      duration: const Duration(milliseconds: 180),
      switchInCurve: Curves.easeOutCubic,
      switchOutCurve: Curves.easeInCubic,
      child: SizedBox(
        key: ValueKey('${activity.sessionId}:${activity.label}'),
        height: 34,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: shadTheme.colorScheme.secondary.withValues(alpha: 0.72),
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: toneColor.withValues(alpha: 0.28)),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              children: <Widget>[
                Icon(_activityIcon(activity.kind), size: 15, color: toneColor),
                const SizedBox(width: 8),
                Expanded(
                  child: DefaultTextStyle.merge(
                    style: textStyle,
                    maxLines: 1,
                    overflow: TextOverflow.clip,
                    child: _MarqueeText(text: activity.label),
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

IconData _activityIcon(AgentActivityKind kind) {
  return switch (kind) {
    AgentActivityKind.reasoning => LucideIcons.sparkles,
    AgentActivityKind.tool => LucideIcons.terminal,
    AgentActivityKind.success => LucideIcons.bot,
    AgentActivityKind.error => LucideIcons.circleAlert,
    AgentActivityKind.running => LucideIcons.bot,
  };
}

Color _activityToneColor(BuildContext context, AgentActivityTone tone) {
  final shadTheme = ShadTheme.of(context);
  return switch (tone) {
    AgentActivityTone.info => shadTheme.colorScheme.primary,
    AgentActivityTone.success => shadTheme.colorScheme.primary,
    AgentActivityTone.error => shadTheme.colorScheme.destructive,
  };
}

class _MarqueeText extends StatefulWidget {
  const _MarqueeText({required this.text});

  final String text;

  @override
  State<_MarqueeText> createState() => _MarqueeTextState();
}

class _MarqueeTextState extends State<_MarqueeText>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 6200),
  );

  @override
  void initState() {
    super.initState();
    unawaited(_controller.repeat());
  }

  @override
  void didUpdateWidget(_MarqueeText oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.text != widget.text) {
      _controller.reset();
      unawaited(_controller.repeat());
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final style = DefaultTextStyle.of(context).style;
    return LayoutBuilder(
      builder: (context, constraints) {
        final painter = TextPainter(
          text: TextSpan(text: widget.text, style: style),
          maxLines: 1,
          textDirection: Directionality.of(context),
        )..layout(maxWidth: double.infinity);
        final textWidth = painter.width;
        if (textWidth <= constraints.maxWidth) {
          return Text(
            widget.text,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            softWrap: false,
          );
        }

        const gap = 42.0;
        final distance = textWidth + gap;
        return ClipRect(
          child: AnimatedBuilder(
            animation: _controller,
            builder: (context, child) {
              return Transform.translate(
                offset: Offset(-distance * _controller.value, 0),
                child: child,
              );
            },
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                Text(widget.text, maxLines: 1, softWrap: false),
                const SizedBox(width: gap),
                Text(widget.text, maxLines: 1, softWrap: false),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _ComposerReplyBanner extends StatelessWidget {
  const _ComposerReplyBanner({
    required this.senderName,
    required this.text,
    required this.isRecalled,
    required this.onClear,
  });

  final String senderName;
  final String text;
  final bool isRecalled;
  final VoidCallback onClear;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: shadTheme.colorScheme.secondary,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: shadTheme.colorScheme.border),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
        child: Row(
          children: <Widget>[
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: <Widget>[
                  Text(
                    '回复 $senderName',
                    style: shadTheme.textTheme.small.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    isRecalled ? '原消息已撤回' : text,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: shadTheme.textTheme.muted,
                  ),
                ],
              ),
            ),
            IconButton(
              onPressed: onClear,
              icon: const Icon(LucideIcons.x, size: 18),
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

class _SenderHeader extends StatelessWidget {
  const _SenderHeader({
    required this.title,
    required this.isAgent,
    required this.color,
  });

  final String title;
  final bool isAgent;
  final Color color;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    if (!isAgent) {
      return Text(
        title,
        style: shadTheme.textTheme.small.copyWith(color: color),
      );
    }

    return Row(
      mainAxisSize: .min,
      children: <Widget>[
        Icon(LucideIcons.bot, size: 14, color: color),
        const SizedBox(width: 5),
        Flexible(
          child: Text(
            title,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: shadTheme.textTheme.small.copyWith(
              color: color,
              fontWeight: .w700,
            ),
          ),
        ),
        const SizedBox(width: 6),
        DecoratedBox(
          decoration: BoxDecoration(
            color: Theme.of(
              context,
            ).colorScheme.tertiary.withValues(alpha: 0.14),
            borderRadius: .circular(999),
          ),
          child: Padding(
            padding: const .symmetric(horizontal: 6, vertical: 2),
            child: Text(
              'Agent',
              style: shadTheme.textTheme.small.copyWith(
                color: Theme.of(context).colorScheme.tertiary,
                fontWeight: .w700,
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _ChatBubble extends StatelessWidget {
  const _ChatBubble({
    required this.text,
    required this.isMine,
    required this.isAgent,
    required this.senderName,
    this.title,
    this.mentionsMe = false,
    this.replyTo,
    this.recalledAtMs,
    this.deliveryState = SocialMessageDeliveryState.sent,
    this.onRetry,
    this.onLongPress,
  });

  final String? title;
  final String senderName;
  final String text;
  final bool isMine;
  final bool isAgent;
  final bool mentionsMe;
  final ChatMessageReplySummary? replyTo;
  final int? recalledAtMs;
  final SocialMessageDeliveryState deliveryState;
  final VoidCallback? onRetry;
  final VoidCallback? onLongPress;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    final theme = Theme.of(context);
    final isRecalled = recalledAtMs != null;
    final bubbleColor = isAgent
        ? theme.colorScheme.surfaceContainerHigh
        : isMine
        ? shadTheme.colorScheme.primary
        : shadTheme.colorScheme.secondary;
    final foreground = isAgent
        ? theme.colorScheme.onSurface
        : isMine
        ? shadTheme.colorScheme.primaryForeground
        : shadTheme.colorScheme.secondaryForeground;
    final mentionAccent = isMine
        ? foreground.withValues(alpha: 0.88)
        : isAgent
        ? theme.colorScheme.tertiary
        : shadTheme.colorScheme.primary;

    return Padding(
      padding: EdgeInsets.fromLTRB(isMine ? 52 : 0, 0, isMine ? 0 : 52, 12),
      child: Align(
        alignment: isMine ? Alignment.centerRight : Alignment.centerLeft,
        child: GestureDetector(
          onLongPress: onLongPress,
          child: ConstrainedBox(
            constraints: BoxConstraints(
              maxWidth: MediaQuery.of(context).size.width * 0.76,
            ),
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: isRecalled ? shadTheme.colorScheme.muted : bubbleColor,
                borderRadius: BorderRadius.circular(12),
                border: Border.all(
                  color: mentionsMe
                      ? const Color(0xFFF59E0B)
                      : isAgent
                      ? theme.colorScheme.tertiary.withValues(alpha: 0.42)
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
                          color: const Color(
                            0xFFF59E0B,
                          ).withValues(alpha: 0.16),
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
                      _SenderHeader(
                        title: title!,
                        isAgent: isAgent,
                        color: foreground.withValues(alpha: 0.82),
                      ),
                      const SizedBox(height: 4),
                    ],
                    if (!isRecalled && replyTo != null) ...<Widget>[
                      _ReplyPreviewBlock(
                        senderName: replyTo!.sender.displayName,
                        text: replyTo!.text,
                        isRecalled: replyTo!.recalledAtMs != null,
                        foreground: foreground,
                        borderColor: foreground.withValues(alpha: 0.22),
                      ),
                      const SizedBox(height: 8),
                    ],
                    if (isRecalled)
                      Text(
                        isMine ? '你撤回了一条消息' : '$senderName 撤回了一条消息',
                        style: shadTheme.textTheme.small.copyWith(
                          color: shadTheme.colorScheme.mutedForeground,
                          fontStyle: FontStyle.italic,
                          fontWeight: FontWeight.w600,
                        ),
                      )
                    else
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
                    if (isMine &&
                        !isRecalled &&
                        deliveryState !=
                            SocialMessageDeliveryState.sent) ...<Widget>[
                      const SizedBox(height: 6),
                      GestureDetector(
                        onTap: onRetry,
                        child: Text(
                          switch (deliveryState) {
                            SocialMessageDeliveryState.sending => '发送中...',
                            SocialMessageDeliveryState.failed => '发送失败，点击重试',
                            SocialMessageDeliveryState.sent => '',
                          },
                          style: shadTheme.textTheme.small.copyWith(
                            color:
                                deliveryState ==
                                    SocialMessageDeliveryState.failed
                                ? const Color(0xFFFCA5A5)
                                : foreground.withValues(alpha: 0.82),
                            fontWeight:
                                deliveryState ==
                                    SocialMessageDeliveryState.failed
                                ? FontWeight.w700
                                : FontWeight.w500,
                          ),
                        ),
                      ),
                    ],
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _ReplyPreviewBlock extends StatelessWidget {
  const _ReplyPreviewBlock({
    required this.senderName,
    required this.text,
    required this.isRecalled,
    required this.foreground,
    required this.borderColor,
  });

  final String senderName;
  final String text;
  final bool isRecalled;
  final Color foreground;
  final Color borderColor;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: Colors.black.withValues(alpha: 0.08),
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: borderColor),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(10, 8, 10, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Text(
              senderName,
              style: shadTheme.textTheme.small.copyWith(
                color: foreground.withValues(alpha: 0.82),
                fontWeight: FontWeight.w700,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              isRecalled ? '原消息已撤回' : text,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: shadTheme.textTheme.small.copyWith(
                color: foreground.withValues(alpha: 0.82),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

enum _MessageAction { reply, recall }

/// Sealed class for mention picker selection result.
sealed class _MentionSelection {
  const _MentionSelection();
  factory _MentionSelection.user(UserSummary user) = _MentionSelectionUser;
  factory _MentionSelection.agent(AgentProfile agent) = _MentionSelectionAgent;
}

class _MentionSelectionUser extends _MentionSelection {
  const _MentionSelectionUser(this.user);
  final UserSummary user;
}

class _MentionSelectionAgent extends _MentionSelection {
  const _MentionSelectionAgent(this.agent);
  final AgentProfile agent;
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

enum _ConversationStatus { online, offline, busy, executing }

extension _ConversationStatusLabel on _ConversationStatus {
  String get label => switch (this) {
    _ConversationStatus.online => '在线',
    _ConversationStatus.offline => '离线',
    _ConversationStatus.busy => '繁忙',
    _ConversationStatus.executing => '执行中',
  };
}

_ConversationStatus _resolveConversationStatus({
  required SocialConversationState conversation,
  required List<AgentProfile> agents,
  required List<HostSummaryDto> hosts,
  required String? activeHostId,
  required ConnectionState? connectionState,
}) {
  final hasAgent = agents.isNotEmpty;
  final isOnline = hasAgent
      ? _isAgentHostOnline(
          agent: agents.first,
          hosts: hosts,
          activeHostId: activeHostId,
          connectionState: connectionState,
        )
      : connectionState is ConnectionState_Connected;

  if (hasAgent &&
      isOnline &&
      _isWaitingForAgentReply(
        conversation.messages,
        conversation.myAccountId,
      )) {
    return _ConversationStatus.executing;
  }

  if (conversation.messages.any(
    (message) => message.deliveryState == SocialMessageDeliveryState.sending,
  )) {
    return _ConversationStatus.busy;
  }

  return isOnline ? _ConversationStatus.online : _ConversationStatus.offline;
}

bool _isAgentHostOnline({
  required AgentProfile agent,
  required List<HostSummaryDto> hosts,
  required String? activeHostId,
  required ConnectionState? connectionState,
}) {
  final agentHostId = agent.hostDeviceId;
  if (agentHostId != null) {
    for (final host in hosts) {
      if (host.hostDeviceId == agentHostId) {
        return host.online;
      }
    }
  }

  if (activeHostId != null) {
    for (final host in hosts) {
      if (host.hostDeviceId == activeHostId) {
        return host.online;
      }
    }
  }

  if (hosts.length == 1) {
    return hosts.first.online;
  }

  if (hosts.any((host) => host.online)) {
    return true;
  }

  return connectionState is ConnectionState_Connected;
}

bool _isWaitingForAgentReply(
  List<SocialChatMessage> messages,
  String? myAccountId,
) {
  if (myAccountId == null) {
    return false;
  }

  for (final message in messages.reversed) {
    if (message.isRecalled) {
      continue;
    }
    if (message.senderType == SenderType.agent) {
      return false;
    }
    if (message.sender.accountId == myAccountId) {
      return message.deliveryState == SocialMessageDeliveryState.sent;
    }
    return false;
  }
  return false;
}

Color _conversationStatusColor(
  BuildContext context,
  _ConversationStatus status,
) {
  final scheme = Theme.of(context).colorScheme;
  return switch (status) {
    _ConversationStatus.online => const Color(0xFF16A34A),
    _ConversationStatus.offline => scheme.onSurfaceVariant.withValues(
      alpha: 0.76,
    ),
    _ConversationStatus.busy => const Color(0xFFD97706),
    _ConversationStatus.executing => const Color(0xFF2563EB),
  };
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
  showLoggedErrorToast(
    context,
    target: 'social_chat',
    title: title,
    error: error,
  );
}
