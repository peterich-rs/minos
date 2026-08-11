import 'dart:async';

import 'package:flutter/cupertino.dart' hide ConnectionState;
import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_list_view/flutter_list_view.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/domain/group_member.dart';
import 'package:minos/domain/message_sender_ext.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/core/widgets/minos_button.dart';
import 'package:minos/ui/core/widgets/minos_text_field.dart';
import 'package:minos/ui/core/widgets/minos_toast.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:minos/ui/features/social/lib/message_grouping.dart';
import 'package:minos/ui/features/social/widgets/conversation_day_divider.dart';
import 'package:minos/ui/features/social/widgets/conversation_message_actions.dart';
import 'package:minos/ui/features/social/widgets/conversation_message_row.dart';
import 'package:minos/ui/theme/theme.dart';

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
  bool _following = true;

  static const double _topLoadOlderThreshold = 96;

  @override
  void initState() {
    super.initState();
    _scrollController.addListener(_onScrollPositionChanged);
    // Focused for unread / markRead (distinct from timeline window open).
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      ref.read(focusedSocialConversationIdProvider.notifier).state =
          widget.conversationId;
    });
  }

  @override
  void dispose() {
    _scrollController.removeListener(_onScrollPositionChanged);
    // Clear focus if we still own it.
    final focused = ref.read(focusedSocialConversationIdProvider);
    if (focused == widget.conversationId) {
      ref.read(focusedSocialConversationIdProvider.notifier).state = null;
    }
    _controller.dispose();
    _composerFocusNode.dispose();
    _scrollController.dispose();
    super.dispose();
  }

  void _onScrollPositionChanged() {
    final near = _isNearBottom();
    if (near != _following) {
      setState(() => _following = near);
    }
    // Near top of ASC list → load older page (before_seq).
    if (_isNearTop()) {
      final notifier = ref.read(
        socialConversationProvider(widget.conversationId).notifier,
      );
      final st = ref.read(socialConversationProvider(widget.conversationId));
      if (st.hasOlder && !st.loadingOlder) {
        unawaited(notifier.loadOlder());
      }
    }
  }

  bool _isNearTop() {
    if (!_scrollController.hasClients) return false;
    return _scrollController.position.pixels <= _topLoadOlderThreshold;
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

  void _insertParticipantMention(GroupMember member) {
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

  /// Membership-first @ picker: only current conversation participants.
  Future<void> _showMentionPicker() async {
    final myAccountId = ref
        .read(socialConversationProvider(widget.conversationId))
        .myAccountId;
    final participants = ref
        .read(groupMentionableMembersProvider(widget.conversationId))
        .where((member) => !member.isAgent || member.id.isNotEmpty)
        .where((member) => member.isAgent || member.id != myAccountId)
        .toList(growable: false);
    if (participants.isEmpty) return;

    final agents = participants
        .where((member) => member.isAgent)
        .toList(growable: false);
    final humans = participants
        .where((member) => !member.isAgent)
        .toList(growable: false);

    final selected = await showModalBottomSheet<GroupMember>(
      context: context,
      useSafeArea: true,
      builder: (context) {
        final colors = context.minosColors;
        final theme = Theme.of(context);
        return SafeArea(
          child: ListView(
            shrinkWrap: true,
            children: <Widget>[
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
                child: Text('选择要艾特的成员', style: theme.textTheme.titleLarge),
              ),
              if (agents.isNotEmpty) ...<Widget>[
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
                  child: Text(
                    'Agents',
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: colors.textSecondary,
                    ),
                  ),
                ),
                for (final agent in agents)
                  ListTile(
                    leading: const Icon(CupertinoIcons.gear_alt_fill, size: 20),
                    title: Text(agent.displayName),
                    subtitle: Text('@${agent.minosId}'),
                    onTap: () => Navigator.of(context).pop(agent),
                  ),
                const Divider(),
              ],
              if (humans.isNotEmpty) ...<Widget>[
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
                  child: Text(
                    '成员',
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: colors.textSecondary,
                    ),
                  ),
                ),
                for (final member in humans)
                  ListTile(
                    title: Text(member.displayName),
                    subtitle: Text('@${member.minosId}'),
                    onTap: () => Navigator.of(context).pop(member),
                  ),
              ],
            ],
          ),
        );
      },
    );
    if (selected != null) {
      _insertParticipantMention(selected);
    }
  }

  void _jumpToBottom({int? messageCount, bool settle = false}) {
    final generation = ++_bottomAnchorGeneration;
    if (!_following) {
      setState(() => _following = true);
    }

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
    final colors = context.minosColors;
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
        if (next.messages.length > previousCount &&
            (_following || previousCount == 0)) {
          _jumpToBottom(
            messageCount: next.messages.length,
            settle: previousCount == 0,
          );
        }
      },
    );

    final messageCount = ref.watch(
      socialConversationProvider(
        widget.conversationId,
      ).select((SocialConversationState s) => s.messages.length),
    );

    return Scaffold(
      resizeToAvoidBottomInset: false,
      backgroundColor: colors.canvas,
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
                    icon: const Icon(CupertinoIcons.person_2),
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
              child: Stack(
                children: <Widget>[
                  GestureDetector(
                    behavior: HitTestBehavior.translucent,
                    onTap: _composerFocusNode.unfocus,
                    child: _ConversationMessagePane(
                      conversationId: widget.conversationId,
                      scrollController: _scrollController,
                    ),
                  ),
                  if (!_following && messageCount > 0)
                    Positioned(
                      right: MinosSpacing.lg,
                      bottom: MinosSpacing.lg,
                      child: _JumpToLatestButton(
                        onPressed: () => _jumpToBottom(
                          messageCount: messageCount,
                          settle: true,
                        ),
                      ),
                    ),
                ],
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

class _JumpToLatestButton extends StatelessWidget {
  const _JumpToLatestButton({required this.onPressed});

  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    return Material(
      elevation: 2,
      shadowColor: colors.scrim.withValues(alpha: 0.2),
      color: colors.surface,
      shape: const CircleBorder(),
      child: InkWell(
        customBorder: const CircleBorder(),
        onTap: onPressed,
        child: SizedBox(
          width: 44,
          height: 44,
          child: Icon(
            CupertinoIcons.arrow_down,
            size: 20,
            color: colors.textPrimary,
          ),
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
    final colors = context.minosColors;
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
            color: colors.textPrimary,
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
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
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
        message.sender.accountIdOrNull == conversation.myAccountId) {
      continue;
    }
    if (conversation.myAccountId == null &&
        (message.sender.minosIdOrEmpty == 'me' ||
            message.sender.displayName.trim() == '我')) {
      continue;
    }
    return _senderTitle(message.sender);
  }
  return null;
}

String? _senderTitle(MessageSender? sender) {
  if (sender == null) return null;
  return _firstNonEmpty(<String?>[
    sender.displayName.trim(),
    sender.minosIdOrEmpty.trim(),
    sender.identityId.trim(),
  ]);
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
    required this.scrollController,
  });

  final String conversationId;
  final FlutterListViewController scrollController;

  Future<void> _showMessageActions(
    BuildContext context,
    WidgetRef ref, {
    required SocialChatMessage message,
    required bool isMine,
  }) async {
    final action = await showConversationMessageActions(
      context,
      message: message,
      isMine: isMine,
    );
    if (!context.mounted || action == null) {
      return;
    }

    switch (action) {
      case ConversationMessageAction.reply:
        ref
            .read(socialReplyDraftProvider(conversationId).notifier)
            .select(message.localId);
        return;
      case ConversationMessageAction.copy:
        await copyMessageText(message.text);
        if (!context.mounted) return;
        showMinosToast(context, title: '已复制');
        return;
      case ConversationMessageAction.retry:
        try {
          await ref
              .read(socialConversationProvider(conversationId).notifier)
              .retryMessage(message.localId);
        } catch (error) {
          if (!context.mounted) return;
          _showError(context, '重试失败', error);
        }
        return;
      case ConversationMessageAction.recall:
        final confirmed = await confirmRecallMessage(context);
        if (!confirmed || !context.mounted) {
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
                  final showDayDivider = shouldShowDayDivider(
                    previous,
                    message,
                  );
                  final isMine =
                      message.sender.accountIdOrNull == state.myAccountId;
                  final mentionsMe =
                      !isMine &&
                      state.myAccountId != null &&
                      message.mentionedAccountIds.contains(state.myAccountId);
                  final groupedWithPrevious = isMessageGroupContinuation(
                    previous,
                    message,
                  );
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
                  final canShowActions =
                      !message.isRecalled &&
                      (message.canReply ||
                          (isMine && message.canRecall) ||
                          (isMine &&
                              message.deliveryState ==
                                  SocialMessageDeliveryState.failed) ||
                          message.text.trim().isNotEmpty);
                  final actionHandler = canShowActions
                      ? () => _showMessageActions(
                          context,
                          ref,
                          message: message,
                          isMine: isMine,
                        )
                      : null;
                  return Padding(
                    padding: EdgeInsets.only(
                      top: index == 0 ? MinosSpacing.sm : 0,
                      bottom: index == state.messages.length - 1
                          ? MinosSpacing.xl
                          : 0,
                    ),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: <Widget>[
                        if (showDayDivider)
                          ConversationDayDivider(
                            label: formatDayDividerLabel(message.createdAtMs),
                          ),
                        ConversationMessageRow(
                          message: message,
                          isMine: isMine,
                          groupedWithPrevious: groupedWithPrevious,
                          mentionsMe: mentionsMe,
                          onRetry: retryAction,
                          onLongPress: actionHandler,
                          onToggleReaction:
                              message.serverMessageId == null ||
                                  message.isRecalled
                              ? null
                              : (emoji) {
                                  unawaited(
                                    ref
                                        .read(
                                          socialConversationProvider(
                                            conversationId,
                                          ).notifier,
                                        )
                                        .toggleReaction(
                                          messageId: message.serverMessageId!,
                                          emoji: emoji,
                                        ),
                                  );
                                },
                        ),
                      ],
                    ),
                  );
                },
                childCount: state.messages.length,
                // Include index so rare localId collisions (hub merge / pending)
                // cannot crash FlutterListView with "Duplicate keys".
                onItemKey: (index) => '${state.messages[index].localId}#$index',
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
  final Future<void> Function() onShowMentionPicker;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final colors = context.minosColors;
    final replyTarget = ref.watch(socialReplyMessageProvider(conversationId));
    final myAccountId = ref.watch(
      socialConversationProvider(
        conversationId,
      ).select((SocialConversationState state) => state.myAccountId),
    );
    final mentionable = kind == ConversationKind.group
        ? ref
              .watch(groupMentionableMembersProvider(conversationId))
              .where((member) => member.isAgent || member.id != myAccountId)
              .toList(growable: false)
        : const <GroupMember>[];
    final hasMentionable = mentionable.isNotEmpty;

    return Container(
      decoration: BoxDecoration(
        color: colors.canvas,
        border: Border(top: BorderSide(color: colors.border)),
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
          Row(
            children: <Widget>[
              if (kind == ConversationKind.group) ...<Widget>[
                MinosButton.outline(
                  onPressed: !hasMentionable ? null : onShowMentionPicker,
                  child: const Text('@'),
                ),
                const SizedBox(width: 8),
              ],
              Expanded(
                child: MinosTextField(
                  controller: controller,
                  focusNode: focusNode,
                  minLines: 1,
                  maxLines: 4,
                  placeholder: '发送消息...',
                  onSubmitted: (_) => onSend(),
                ),
              ),
              const SizedBox(width: 10),
              MinosButton(onPressed: onSend, child: const Text('发送')),
            ],
          ),
        ],
      ),
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
    final colors = context.minosColors;
    final theme = Theme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: colors.surfaceMuted,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: colors.border),
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
                    style: theme.textTheme.bodySmall?.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    isRecalled ? '原消息已撤回' : text,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: colors.textSecondary,
                    ),
                  ),
                ],
              ),
            ),
            IconButton(
              onPressed: onClear,
              icon: const Icon(CupertinoIcons.xmark, size: 18),
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
    final colors = context.minosColors;
    final theme = Theme.of(context);
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: <Widget>[
        Icon(
          CupertinoIcons.exclamationmark_triangle,
          size: 36,
          color: colors.textSecondary,
        ),
        const SizedBox(height: 10),
        Text(title, style: theme.textTheme.titleLarge),
        const SizedBox(height: 6),
        Text(
          description,
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyMedium?.copyWith(
            color: colors.textSecondary,
          ),
        ),
      ],
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
    if (message.sender.accountIdOrNull == myAccountId) {
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
  final colors = context.minosColors;
  return switch (status) {
    _ConversationStatus.online => colors.success,
    _ConversationStatus.offline => colors.textSecondary.withValues(alpha: 0.76),
    _ConversationStatus.busy => colors.warning,
    _ConversationStatus.executing => colors.accent,
  };
}

void _showError(BuildContext context, String title, Object error) {
  showLoggedErrorToast(
    context,
    target: 'social_chat',
    title: title,
    error: error,
  );
}
