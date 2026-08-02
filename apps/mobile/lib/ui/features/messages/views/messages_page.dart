import 'dart:async';

import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/agent_conversation_actions.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/social_actions.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/core/widgets/minos_button.dart';
import 'package:minos/ui/core/widgets/minos_empty_state.dart';
import 'package:minos/ui/core/widgets/minos_page_header.dart';
import 'package:minos/ui/core/widgets/minos_progress.dart';
import 'package:minos/ui/core/widgets/minos_text_field.dart';
import 'package:minos/ui/core/widgets/minos_toast.dart';
import 'package:minos/ui/core/widgets/shimmer_box.dart';
import 'package:minos/ui/features/messages/widgets/conversation_tile.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:minos/ui/theme/theme.dart';

/// Golden-path Messages inbox: all conversations by last activity.
class MessagesPage extends ConsumerStatefulWidget {
  const MessagesPage({super.key});

  @override
  ConsumerState<MessagesPage> createState() => _MessagesPageState();
}

class _MessagesPageState extends ConsumerState<MessagesPage> {
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
      showLoggedErrorToast(
        context,
        target: 'messages_page',
        title: '打开聊天失败',
        error: error,
      );
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
      showLoggedErrorToast(
        context,
        target: 'messages_page',
        title: '打开 Agent 私信失败',
        error: error,
      );
    }
  }

  Future<void> _showStartConversationSheet() async {
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      useSafeArea: true,
      backgroundColor: context.minosColors.surface,
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
      showMinosToast(context, title: '至少需要 1 位好友或 Agent 才能创建群聊');
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
      backgroundColor: context.minosColors.surface,
      builder: (context) {
        return StatefulBuilder(
          builder: (context, setSheetState) {
            final colors = context.minosColors;
            final theme = Theme.of(context);
            return Padding(
              padding: EdgeInsets.only(
                left: MinosSpacing.pageX,
                right: MinosSpacing.pageX,
                top: MinosSpacing.sm,
                bottom:
                    MediaQuery.of(context).viewInsets.bottom + MinosSpacing.lg,
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text('新建群聊', style: theme.textTheme.titleLarge),
                  const SizedBox(height: MinosSpacing.md),
                  MinosTextField(
                    controller: titleController,
                    placeholder: '群聊名称',
                  ),
                  const SizedBox(height: MinosSpacing.md),
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
                            secondary: Icon(
                              CupertinoIcons.gear_alt_fill,
                              size: 20,
                              color: colors.accent,
                            ),
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
                  const SizedBox(height: MinosSpacing.md),
                  Row(
                    children: <Widget>[
                      Expanded(
                        child: MinosButton.outline(
                          onPressed: () => Navigator.of(context).pop(),
                          child: const Text('取消'),
                        ),
                      ),
                      const SizedBox(width: MinosSpacing.md),
                      Expanded(
                        child: MinosButton(
                          onPressed: () async {
                            final title = titleController.text.trim();
                            if (title.isEmpty) {
                              showMinosToast(rootContext, title: '请输入群聊名称');
                              return;
                            }
                            if (selectedIds.isEmpty &&
                                selectedAgentIds.isEmpty) {
                              showMinosToast(
                                rootContext,
                                title: '请选择好友或 Agent',
                              );
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
                              showLoggedErrorToast(
                                rootContext,
                                target: 'messages_page',
                                title: '创建群聊失败',
                                error: error,
                              );
                            }
                          },
                          child: const Text('创建'),
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

  Future<bool> _confirmDeleteConversation(
    ConversationSummary conversation,
  ) async {
    final confirmed = await showCupertinoDialog<bool>(
      context: context,
      builder: (dialogContext) => CupertinoAlertDialog(
        title: const Text('删除会话'),
        content: Text(
          '确定要删除「${conversation.title}」吗？聊天记录会被清除；如果 Agent 仍在执行，会先尝试停止任务。',
        ),
        actions: <Widget>[
          CupertinoDialogAction(
            onPressed: () => Navigator.of(dialogContext).pop(false),
            child: const Text('取消'),
          ),
          CupertinoDialogAction(
            isDestructiveAction: true,
            onPressed: () => Navigator.of(dialogContext).pop(true),
            child: const Text('删除'),
          ),
        ],
      ),
    );
    return confirmed == true;
  }

  Future<void> _deleteConversation(ConversationSummary conversation) async {
    try {
      await ref
          .read(conversationsProvider.notifier)
          .deleteConversation(conversation.conversationId);
      if (!mounted) return;
      showMinosToast(context, title: '会话已删除');
    } catch (error) {
      if (!mounted) return;
      showLoggedErrorToast(
        context,
        target: 'messages_page',
        title: '删除会话失败',
        error: error,
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final conversationsAsync = ref.watch(conversationsProvider);
    final colors = context.minosColors;

    return ColoredBox(
      color: colors.canvas,
      child: SafeArea(
        bottom: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            MinosPageHeader(
              title: '消息',
              subtitle: '按最近活跃排序的全部对话',
              trailing: IconButton(
                tooltip: '发起会话',
                onPressed: _showStartConversationSheet,
                icon: Icon(CupertinoIcons.square_pencil, color: colors.accent),
              ),
            ),
            Expanded(
              child: RefreshIndicator(
                onRefresh: _refreshAll,
                child: conversationsAsync.when(
                  loading: () => const _MessagesSkeleton(),
                  error: (error, _) => ListView(
                    physics: const AlwaysScrollableScrollPhysics(
                      parent: BouncingScrollPhysics(),
                    ),
                    children: <Widget>[
                      MinosEmptyState(
                        icon: CupertinoIcons.exclamationmark_triangle,
                        title: '会话加载失败',
                        subtitle: error.toString(),
                        actionLabel: '重试',
                        onAction: _refreshAll,
                      ),
                    ],
                  ),
                  data: (response) {
                    final conversations = response.conversations;
                    if (conversations.isEmpty) {
                      return ListView(
                        physics: const AlwaysScrollableScrollPhysics(
                          parent: BouncingScrollPhysics(),
                        ),
                        children: <Widget>[
                          MinosEmptyState(
                            icon: CupertinoIcons.chat_bubble_2,
                            title: '还没有会话',
                            subtitle: '点右上角发起私聊、群聊或与 Agent 对话。',
                            actionLabel: '发起会话',
                            onAction: _showStartConversationSheet,
                          ),
                        ],
                      );
                    }

                    return ListView.builder(
                      physics: const AlwaysScrollableScrollPhysics(
                        parent: BouncingScrollPhysics(),
                      ),
                      padding: const EdgeInsets.only(
                        bottom: MinosSpacing.pageBottom,
                      ),
                      itemCount: conversations.length,
                      itemBuilder: (context, index) {
                        final conversation = conversations[index];
                        return ConversationTile(
                          conversation: conversation,
                          onConfirmDelete: _confirmDeleteConversation,
                          onDelete: _deleteConversation,
                        );
                      },
                    );
                  },
                ),
              ),
            ),
          ],
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
    final colors = context.minosColors;
    final theme = Theme.of(context);
    return Padding(
      padding: EdgeInsets.only(
        left: MinosSpacing.pageX,
        right: MinosSpacing.pageX,
        bottom: MediaQuery.of(context).viewInsets.bottom + MinosSpacing.lg,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Text('发起会话', style: theme.textTheme.titleLarge),
          const SizedBox(height: MinosSpacing.md),
          Flexible(
            child: ListView(
              shrinkWrap: true,
              children: <Widget>[
                ListTile(
                  contentPadding: EdgeInsets.zero,
                  leading: CircleAvatar(
                    radius: 20,
                    backgroundColor: colors.accentSoft,
                    child: Icon(
                      CupertinoIcons.person_2_fill,
                      size: 20,
                      color: colors.accent,
                    ),
                  ),
                  title: const Text('新建群聊'),
                  subtitle: Text(
                    friends.isNotEmpty || agents.isNotEmpty
                        ? '从 ${friends.length} 位好友和 ${agents.length} 个 Agent 中选择成员'
                        : '先添加好友或创建 Agent',
                  ),
                  trailing: Icon(
                    CupertinoIcons.chevron_right,
                    size: 18,
                    color: colors.textTertiary,
                  ),
                  onTap: () => onCreateGroup(friends, agents),
                ),
                Divider(height: 1, color: colors.borderSubtle),
                const _SheetGroupLabel(label: '私聊'),
                friendsAsync.when(
                  loading: () => const Padding(
                    padding: EdgeInsets.symmetric(vertical: MinosSpacing.lg),
                    child: Center(child: MinosProgress()),
                  ),
                  error: (error, _) => Padding(
                    padding: const EdgeInsets.symmetric(
                      vertical: MinosSpacing.md,
                    ),
                    child: Text(
                      error.toString(),
                      style: theme.textTheme.bodyMedium?.copyWith(
                        color: colors.textSecondary,
                      ),
                    ),
                  ),
                  data: (response) => response.friends.isEmpty
                      ? Padding(
                          padding: const EdgeInsets.symmetric(
                            vertical: MinosSpacing.md,
                          ),
                          child: Text(
                            '还没有好友',
                            style: theme.textTheme.bodyMedium?.copyWith(
                              color: colors.textSecondary,
                            ),
                          ),
                        )
                      : Column(
                          children: <Widget>[
                            for (final friend in response.friends)
                              ListTile(
                                contentPadding: EdgeInsets.zero,
                                leading: CircleAvatar(
                                  backgroundColor: colors.surfaceMuted,
                                  child: Text(
                                    friend.displayName.trim().isEmpty
                                        ? '?'
                                        : friend.displayName
                                              .trim()
                                              .substring(0, 1)
                                              .toUpperCase(),
                                  ),
                                ),
                                title: Text(friend.displayName),
                                subtitle: Text('@${friend.minosId}'),
                                onTap: () => onOpenFriend(friend),
                              ),
                          ],
                        ),
                ),
                if (agents.isNotEmpty) ...<Widget>[
                  Divider(height: 1, color: colors.borderSubtle),
                  const _SheetGroupLabel(label: 'Agents'),
                  for (final agent in agents)
                    ListTile(
                      contentPadding: EdgeInsets.zero,
                      leading: CircleAvatar(
                        backgroundColor: colors.accentSoft,
                        child: Icon(
                          CupertinoIcons.gear_alt_fill,
                          color: colors.accent,
                        ),
                      ),
                      title: Text(agent.name),
                      subtitle: Text(
                        '${agent.runtimeAgent.name} · ${agent.model}',
                      ),
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

class _SheetGroupLabel extends StatelessWidget {
  const _SheetGroupLabel({required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        0,
        MinosSpacing.md,
        0,
        MinosSpacing.sm,
      ),
      child: Text(
        label,
        style: Theme.of(context).textTheme.labelMedium?.copyWith(
          color: colors.textSecondary,
          fontWeight: FontWeight.w700,
        ),
      ),
    );
  }
}

class _MessagesSkeleton extends StatelessWidget {
  const _MessagesSkeleton();

  @override
  Widget build(BuildContext context) {
    return ListView(
      physics: const AlwaysScrollableScrollPhysics(
        parent: BouncingScrollPhysics(),
      ),
      children: List.generate(
        6,
        (index) => const Padding(
          padding: EdgeInsets.fromLTRB(
            MinosSpacing.pageX,
            MinosSpacing.md,
            MinosSpacing.pageX,
            MinosSpacing.md,
          ),
          child: Row(
            children: <Widget>[
              ShimmerBox(width: 48, height: 48, circular: true),
              SizedBox(width: MinosSpacing.md),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: <Widget>[
                    ShimmerBox(width: 160, height: 14),
                    SizedBox(height: MinosSpacing.sm),
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
