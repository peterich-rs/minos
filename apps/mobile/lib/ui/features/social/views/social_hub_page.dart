import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/agent_conversation_actions.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/application/social_actions.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/shimmer_box.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:minos/ui/features/social/widgets/social_management_sections.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

class SocialHubPage extends ConsumerStatefulWidget {
  const SocialHubPage({super.key, this.showAppBar = true});

  final bool showAppBar;

  @override
  ConsumerState<SocialHubPage> createState() => _SocialHubPageState();
}

class _SocialHubPageState extends ConsumerState<SocialHubPage> {
  Future<void> _refreshAll() {
    return ref.read(conversationsProvider.notifier).refresh();
  }

  Future<void> _openDirectChat(FriendSummary friend) async {
    try {
      final response = await ref
          .read(socialActionsProvider)
          .ensureDirectConversation(friendAccountId: friend.accountId);
      if (!mounted) return;
      ref.invalidate(conversationsProvider);
      unawaited(
        context.push(
          '/social/chat/${response.conversationId}',
          extra: SocialChatRouteExtra(
            title: friend.displayName,
            kind: ConversationKind.direct,
          ),
        ),
      );
    } catch (error) {
      if (!mounted) return;
      showSocialFeedbackError(context, '打开聊天失败', error);
    }
  }

  Future<void> _openAgentChat(AgentProfile agent) async {
    try {
      final response = await createAgentConversation(ref, profile: agent);
      if (!mounted) return;
      unawaited(
        context.push(
          '/social/chat/${response.conversationId}',
          extra: SocialChatRouteExtra(
            title: agent.name,
            kind: ConversationKind.group,
          ),
        ),
      );
    } catch (error) {
      if (!mounted) return;
      showSocialFeedbackError(context, '打开 Agent 私信失败', error);
    }
  }

  Future<void> _showStartConversationSheet() async {
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      useSafeArea: true,
      backgroundColor: Theme.of(context).colorScheme.surface,
      builder: (sheetContext) {
        return Consumer(
          builder: (sheetContext, ref, _) {
            final friendsAsync = ref.watch(friendsProvider);
            final agentProfiles =
                ref
                    .watch(agentProfilesControllerProvider)
                    .asData
                    ?.value
                    .profiles ??
                const <AgentProfile>[];
            return _StartConversationSheet(
              friendsAsync: friendsAsync,
              agents: agentProfiles,
              onOpenFriend: (friend) {
                Navigator.of(sheetContext).pop();
                unawaited(_openDirectChat(friend));
              },
              onOpenAgent: (agent) {
                Navigator.of(sheetContext).pop();
                unawaited(_openAgentChat(agent));
              },
              onCreateGroup: (friends, agents) {
                Navigator.of(sheetContext).pop();
                unawaited(_startCreateGroup(friends, agents));
              },
            );
          },
        );
      },
    );
  }

  Future<void> _startCreateGroup(
    List<FriendSummary> friends,
    List<AgentProfile> agents,
  ) async {
    if (friends.isEmpty && agents.isEmpty) {
      showSocialInfoToast(context, '至少需要 1 位好友或 Agent 才能创建群聊');
      return;
    }
    await _createGroup(friends, agents);
  }

  Future<void> _createGroup(
    List<FriendSummary> friends,
    List<AgentProfile> agents,
  ) async {
    final titleController = TextEditingController();
    final selectedIds = <String>{};
    final selectedAgentIds = <String>{};
    final rootContext = context;
    await showModalBottomSheet<void>(
      context: rootContext,
      isScrollControlled: true,
      useSafeArea: true,
      showDragHandle: true,
      backgroundColor: Theme.of(context).colorScheme.surface,
      builder: (context) {
        return StatefulBuilder(
          builder: (context, setSheetState) {
            return Padding(
              padding: EdgeInsets.only(
                left: 16,
                right: 16,
                top: 8,
                bottom: MediaQuery.of(context).viewInsets.bottom + 16,
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text('新建群聊', style: ShadTheme.of(context).textTheme.h4),
                  const SizedBox(height: 12),
                  ShadInput(
                    controller: titleController,
                    placeholder: const Text('群聊名称'),
                  ),
                  const SizedBox(height: 12),
                  Flexible(
                    child: ListView(
                      shrinkWrap: true,
                      children: <Widget>[
                        if (friends.isNotEmpty)
                          const _SheetGroupLabel(label: '好友'),
                        for (final friend in friends)
                          CheckboxListTile(
                            value: selectedIds.contains(friend.accountId),
                            onChanged: (selected) {
                              setSheetState(() {
                                if (selected == true) {
                                  selectedIds.add(friend.accountId);
                                } else {
                                  selectedIds.remove(friend.accountId);
                                }
                              });
                            },
                            title: Text(friend.displayName),
                            subtitle: Text('@${friend.minosId}'),
                          ),
                        if (agents.isNotEmpty) ...<Widget>[
                          const Divider(height: 1),
                          const _SheetGroupLabel(label: 'Agents'),
                        ],
                        for (final agent in agents)
                          CheckboxListTile(
                            value: selectedAgentIds.contains(agent.id),
                            secondary: const Icon(LucideIcons.bot, size: 20),
                            onChanged: (selected) {
                              setSheetState(() {
                                if (selected == true) {
                                  selectedAgentIds.add(agent.id);
                                } else {
                                  selectedAgentIds.remove(agent.id);
                                }
                              });
                            },
                            title: Text(agent.name),
                            subtitle: Text(
                              '${agent.runtimeAgent.name} · @${agent.agentId}',
                            ),
                          ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 12),
                  Row(
                    children: <Widget>[
                      Expanded(
                        child: ShadButton.outline(
                          child: const Text('取消'),
                          onPressed: () => Navigator.of(context).pop(),
                        ),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: ShadButton(
                          child: const Text('创建'),
                          onPressed: () async {
                            final title = titleController.text.trim();
                            if (title.isEmpty) {
                              showSocialInfoToast(rootContext, '请输入群聊名称');
                              return;
                            }
                            if (selectedIds.isEmpty &&
                                selectedAgentIds.isEmpty) {
                              showSocialInfoToast(rootContext, '请选择好友或 Agent');
                              return;
                            }
                            try {
                              final repository = ref.read(
                                socialRepositoryProvider,
                              );
                              final response = await ref
                                  .read(socialActionsProvider)
                                  .createGroupConversation(
                                    title: title,
                                    memberAccountIds: selectedIds.toList(),
                                  );
                              for (final agent in agents) {
                                if (!selectedAgentIds.contains(agent.id)) {
                                  continue;
                                }
                                final serverAgent =
                                    await ensureServerAgentProfile(
                                      ref,
                                      repository,
                                      agent,
                                    );
                                await ref
                                    .read(socialActionsProvider)
                                    .addAgentToConversation(
                                      conversationId: response.conversationId,
                                      agentId: serverAgent.agentId,
                                    );
                              }
                              if (!context.mounted) return;
                              ref.invalidate(conversationsProvider);
                              ref.invalidate(
                                conversationMembersProvider(
                                  response.conversationId,
                                ),
                              );
                              ref.invalidate(
                                conversationAgentMembersProvider(
                                  response.conversationId,
                                ),
                              );
                              Navigator.of(context).pop();
                              if (!mounted) return;
                              unawaited(
                                rootContext.push(
                                  '/social/chat/${response.conversationId}',
                                  extra: SocialChatRouteExtra(
                                    title: title,
                                    kind: ConversationKind.group,
                                  ),
                                ),
                              );
                            } catch (error) {
                              if (!mounted || !rootContext.mounted) return;
                              showSocialFeedbackError(
                                rootContext,
                                '创建群聊失败',
                                error,
                              );
                            }
                          },
                        ),
                      ),
                    ],
                  ),
                ],
              ),
            );
          },
        );
      },
    );
    titleController.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final conversationsAsync = ref.watch(conversationsProvider);
    final shadTheme = ShadTheme.of(context);

    return Scaffold(
      backgroundColor: shadTheme.colorScheme.background,
      appBar: widget.showAppBar
          ? AppBar(
              title: const Text('消息'),
              surfaceTintColor: Colors.transparent,
              actions: <Widget>[
                IconButton(
                  tooltip: '发起会话',
                  icon: const Icon(LucideIcons.messageSquarePlus),
                  onPressed: _showStartConversationSheet,
                ),
              ],
            )
          : null,
      body: RefreshIndicator(
        onRefresh: _refreshAll,
        child: conversationsAsync.when(
          loading: () => _MessagesScrollView(
            showHeader: !widget.showAppBar,
            onCreate: _showStartConversationSheet,
            children: const <Widget>[_ConversationListSkeleton()],
          ),
          error: (error, _) => _MessagesScrollView(
            showHeader: !widget.showAppBar,
            onCreate: _showStartConversationSheet,
            children: <Widget>[
              _MessagesInlineState(
                icon: LucideIcons.circleAlert,
                title: '会话加载失败',
                subtitle: error.toString(),
              ),
            ],
          ),
          data: (response) => _MessagesScrollView(
            showHeader: !widget.showAppBar,
            onCreate: _showStartConversationSheet,
            children: response.conversations.isEmpty
                ? const <Widget>[
                    _MessagesInlineState(
                      icon: LucideIcons.messagesSquare,
                      title: '还没有会话',
                      subtitle: '点右上角发起私聊或群聊。',
                    ),
                  ]
                : <Widget>[
                    for (
                      var index = 0;
                      index < response.conversations.length;
                      index++
                    ) ...<Widget>[
                      _ConversationTile(
                        conversation: response.conversations[index],
                      ),
                      if (index < response.conversations.length - 1)
                        const Divider(height: 1, indent: 72),
                    ],
                  ],
          ),
        ),
      ),
    );
  }
}

class _MessagesScrollView extends StatelessWidget {
  const _MessagesScrollView({
    required this.showHeader,
    required this.onCreate,
    required this.children,
  });

  final bool showHeader;
  final VoidCallback onCreate;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    return ListView(
      physics: const AlwaysScrollableScrollPhysics(),
      padding: EdgeInsets.fromLTRB(
        0,
        showHeader ? MediaQuery.of(context).padding.top + 12 : 6,
        0,
        28,
      ),
      children: <Widget>[
        if (showHeader) ...<Widget>[
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 0, 12, 8),
            child: Row(
              children: <Widget>[
                Expanded(
                  child: Text(
                    '消息',
                    style: Theme.of(context).textTheme.headlineLarge?.copyWith(
                      fontWeight: FontWeight.w800,
                      letterSpacing: 0,
                    ),
                  ),
                ),
                Tooltip(
                  message: '发起会话',
                  child: ShadIconButton.ghost(
                    icon: const Icon(LucideIcons.messageSquarePlus),
                    iconSize: 21,
                    width: 40,
                    height: 40,
                    onPressed: onCreate,
                  ),
                ),
              ],
            ),
          ),
        ],
        ...children,
      ],
    );
  }
}

class _ConversationTile extends StatelessWidget {
  const _ConversationTile({required this.conversation});

  final ConversationSummary conversation;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isGroup = conversation.kind == ConversationKind.group;
    final subtitle = conversation.lastMessagePreview?.trim();
    final unread = conversation.unreadCount;
    final mentionUnread = conversation.unreadMentionCount;

    return Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: () => context.push(
          '/social/chat/${conversation.conversationId}',
          extra: SocialChatRouteExtra(
            title: conversation.title,
            kind: conversation.kind,
          ),
        ),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 11, 16, 11),
          child: Row(
            children: <Widget>[
              CircleAvatar(
                radius: 22,
                backgroundColor: isGroup
                    ? theme.colorScheme.tertiaryContainer
                    : theme.colorScheme.primaryContainer,
                child: Icon(
                  isGroup ? LucideIcons.usersRound : LucideIcons.userRound,
                  size: 21,
                  color: isGroup
                      ? theme.colorScheme.onTertiaryContainer
                      : theme.colorScheme.onPrimaryContainer,
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
                            conversation.title,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: theme.textTheme.titleMedium?.copyWith(
                              fontWeight: FontWeight.w700,
                            ),
                          ),
                        ),
                        const SizedBox(width: 8),
                        Text(
                          _formatConversationTime(
                            conversation.lastMessageAtMs.toInt(),
                          ),
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
                            subtitle == null || subtitle.isEmpty
                                ? isGroup
                                      ? '${conversation.memberCount} 位成员'
                                      : '还没有消息'
                                : subtitle,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: theme.textTheme.bodyMedium?.copyWith(
                              color: theme.colorScheme.onSurfaceVariant,
                            ),
                          ),
                        ),
                        if (unread > 0) ...<Widget>[
                          const SizedBox(width: 8),
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
  }
}

class _StartConversationSheet extends StatelessWidget {
  const _StartConversationSheet({
    required this.friendsAsync,
    required this.agents,
    required this.onOpenFriend,
    required this.onOpenAgent,
    required this.onCreateGroup,
  });

  final AsyncValue<FriendsResponse> friendsAsync;
  final List<AgentProfile> agents;
  final ValueChanged<FriendSummary> onOpenFriend;
  final ValueChanged<AgentProfile> onOpenAgent;
  final void Function(List<FriendSummary> friends, List<AgentProfile> agents)
  onCreateGroup;

  @override
  Widget build(BuildContext context) {
    final friends =
        friendsAsync.asData?.value.friends ?? const <FriendSummary>[];
    return Padding(
      padding: EdgeInsets.only(
        left: 16,
        right: 16,
        bottom: MediaQuery.of(context).viewInsets.bottom + 16,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Text('发起会话', style: ShadTheme.of(context).textTheme.h4),
          const SizedBox(height: 12),
          Flexible(
            child: ListView(
              shrinkWrap: true,
              children: <Widget>[
                _ComposerActionTile(
                  icon: LucideIcons.usersRound,
                  title: '新建群聊',
                  subtitle: friends.isNotEmpty || agents.isNotEmpty
                      ? '从 ${friends.length} 位好友和 ${agents.length} 个 Agent 中选择成员'
                      : '先添加好友或创建 Agent',
                  onTap: () => onCreateGroup(friends, agents),
                ),
                const Divider(height: 1),
                const _SheetGroupLabel(label: '私聊'),
                friendsAsync.when(
                  loading: () => const _SheetLoadingRow(),
                  error: (error, _) => _SheetMessage(text: error.toString()),
                  data: (response) => response.friends.isEmpty
                      ? const _SheetMessage(text: '还没有好友')
                      : Column(
                          children: <Widget>[
                            for (final friend in response.friends)
                              _FriendComposerTile(
                                friend: friend,
                                onTap: () => onOpenFriend(friend),
                              ),
                          ],
                        ),
                ),
                if (agents.isNotEmpty) ...<Widget>[
                  const Divider(height: 1),
                  const _SheetGroupLabel(label: 'Agents'),
                  for (final agent in agents)
                    _AgentComposerTile(
                      agent: agent,
                      onTap: () => onOpenAgent(agent),
                    ),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _ComposerActionTile extends StatelessWidget {
  const _ComposerActionTile({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onTap,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: CircleAvatar(
        radius: 20,
        backgroundColor: Theme.of(context).colorScheme.secondaryContainer,
        child: Icon(
          icon,
          size: 20,
          color: Theme.of(context).colorScheme.onSecondaryContainer,
        ),
      ),
      title: Text(title),
      subtitle: Text(subtitle),
      trailing: const Icon(LucideIcons.chevronRight, size: 18),
      onTap: onTap,
    );
  }
}

class _FriendComposerTile extends StatelessWidget {
  const _FriendComposerTile({required this.friend, required this.onTap});

  final FriendSummary friend;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final initial = friend.displayName.trim().isEmpty
        ? '?'
        : friend.displayName.trim().substring(0, 1).toUpperCase();
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: CircleAvatar(child: Text(initial)),
      title: Text(friend.displayName),
      subtitle: Text('@${friend.minosId}'),
      onTap: onTap,
    );
  }
}

class _AgentComposerTile extends StatelessWidget {
  const _AgentComposerTile({required this.agent, required this.onTap});

  final AgentProfile agent;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: CircleAvatar(
        backgroundColor: Theme.of(context).colorScheme.primaryContainer,
        child: Icon(
          LucideIcons.bot,
          color: Theme.of(context).colorScheme.onPrimaryContainer,
        ),
      ),
      title: Text(agent.name),
      subtitle: Text('${agent.runtimeAgent.name} · ${agent.model}'),
      onTap: onTap,
    );
  }
}

class _SheetGroupLabel extends StatelessWidget {
  const _SheetGroupLabel({required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(0, 12, 0, 6),
      child: Text(
        label,
        style: ShadTheme.of(context).textTheme.small.copyWith(
          color: ShadTheme.of(context).colorScheme.mutedForeground,
          fontWeight: FontWeight.w700,
        ),
      ),
    );
  }
}

class _SheetLoadingRow extends StatelessWidget {
  const _SheetLoadingRow();

  @override
  Widget build(BuildContext context) {
    return const Padding(
      padding: EdgeInsets.symmetric(vertical: 16),
      child: Center(child: ShadProgress()),
    );
  }
}

class _SheetMessage extends StatelessWidget {
  const _SheetMessage({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 12),
      child: Text(
        text,
        style: Theme.of(context).textTheme.bodyMedium?.copyWith(
          color: Theme.of(context).colorScheme.onSurfaceVariant,
        ),
      ),
    );
  }
}

class _ConversationListSkeleton extends StatelessWidget {
  const _ConversationListSkeleton();

  @override
  Widget build(BuildContext context) {
    return Column(
      children: List.generate(
        6,
        (index) => const Padding(
          padding: EdgeInsets.fromLTRB(16, 12, 16, 12),
          child: Row(
            children: <Widget>[
              ShimmerBox(width: 44, height: 44, circular: true),
              SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: <Widget>[
                    ShimmerBox(width: 160, height: 14),
                    SizedBox(height: 8),
                    ShimmerBox(width: double.infinity, height: 12),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _MessagesInlineState extends StatelessWidget {
  const _MessagesInlineState({
    required this.icon,
    required this.title,
    required this.subtitle,
  });

  final IconData icon;
  final String title;
  final String subtitle;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 96, 24, 24),
      child: Column(
        children: <Widget>[
          Icon(icon, size: 34, color: theme.colorScheme.outline),
          const SizedBox(height: 10),
          Text(
            title,
            style: theme.textTheme.titleMedium?.copyWith(
              fontWeight: FontWeight.w700,
            ),
          ),
          const SizedBox(height: 4),
          Text(
            subtitle,
            textAlign: TextAlign.center,
            style: theme.textTheme.bodyMedium?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ],
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
    final theme = Theme.of(context);
    final bg = highlighted
        ? const Color(0xFFF59E0B)
        : theme.colorScheme.primary;
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
            color: Colors.white,
            fontWeight: FontWeight.w700,
          ),
        ),
      ),
    );
  }
}

String _formatConversationTime(int ms) {
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
