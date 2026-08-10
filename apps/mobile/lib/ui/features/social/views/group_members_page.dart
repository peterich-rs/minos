import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/application/social_actions.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/core/widgets/minos_button.dart';
import 'package:minos/ui/core/widgets/minos_progress.dart';
import 'package:minos/ui/core/widgets/minos_toast.dart';
import 'package:minos/ui/theme/theme.dart';

/// Page for viewing and managing group conversation members.
/// Supports viewing existing members (users), adding new user members,
/// and adding/removing agents.
class GroupMembersPage extends ConsumerWidget {
  const GroupMembersPage({
    super.key,
    required this.conversationId,
    required this.title,
  });

  final String conversationId;
  final String title;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final colors = context.minosColors;
    final membersAsync = ref.watch(conversationMembersProvider(conversationId));
    final groupAgents = ref.watch(groupAgentsProvider(conversationId));
    final myAccountId = ref
        .watch(socialProfileProvider)
        .asData
        ?.value
        .accountId;

    return Scaffold(
      backgroundColor: colors.canvas,
      appBar: AppBar(
        title: Text('$title · 成员'),
        surfaceTintColor: Colors.transparent,
        actions: <Widget>[
          IconButton(
            icon: const Icon(CupertinoIcons.person_badge_plus),
            tooltip: '添加成员',
            onPressed: () => _showAddMemberSheet(context, ref),
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 28),
        children: <Widget>[
          // --- User members section ---
          const _SectionHeader(title: '成员'),
          membersAsync.when(
            loading: () => const Padding(
              padding: EdgeInsets.all(16),
              child: Center(child: MinosProgress()),
            ),
            error: (error, _) => Padding(
              padding: const EdgeInsets.all(16),
              child: Text('加载失败: $error'),
            ),
            data: (members) => _UserMembersList(
              conversationId: conversationId,
              members: members,
              myAccountId: myAccountId,
            ),
          ),
          const SizedBox(height: 24),
          // --- Agent members section ---
          _SectionHeader(
            title: 'Agents',
            trailing: MinosButton.ghost(
              onPressed: () => _showAddAgentSheet(context, ref),
              child: const Row(
                mainAxisSize: MainAxisSize.min,
                children: <Widget>[
                  Icon(CupertinoIcons.gear_alt_fill, size: 16),
                  SizedBox(width: 4),
                  Text('添加 Agent'),
                ],
              ),
            ),
          ),
          if (groupAgents.isEmpty)
            const Padding(
              padding: EdgeInsets.all(16),
              child: Text('暂无 Agent 成员，点击上方按钮添加'),
            )
          else
            _AgentMembersList(
              conversationId: conversationId,
              agents: groupAgents,
            ),
        ],
      ),
    );
  }

  Future<void> _showAddMemberSheet(BuildContext context, WidgetRef ref) async {
    final friendsAsync = await ref.read(friendsProvider.future);
    final existingMembers =
        ref.read(conversationMembersProvider(conversationId)).asData?.value ??
        const <UserSummary>[];
    final existingIds = existingMembers
        .map((member) => member.accountId)
        .toSet();
    final available = friendsAsync.friends
        .where((friend) => !existingIds.contains(friend.accountId))
        .toList();

    if (!context.mounted) return;

    if (available.isEmpty) {
      showMinosToast(context, title: '所有好友都已在群中');
      return;
    }

    await showModalBottomSheet<void>(
      context: context,
      useSafeArea: true,
      builder: (sheetContext) {
        final theme = Theme.of(sheetContext);
        return SafeArea(
          child: ListView(
            shrinkWrap: true,
            children: <Widget>[
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
                child: Text('添加成员到群聊', style: theme.textTheme.titleLarge),
              ),
              for (final friend in available)
                ListTile(
                  leading: const Icon(CupertinoIcons.person),
                  title: Text(friend.displayName),
                  subtitle: Text('@${friend.minosId}'),
                  onTap: () async {
                    Navigator.of(sheetContext).pop();
                    try {
                      await ref
                          .read(socialActionsProvider)
                          .addGroupMember(
                            conversationId: conversationId,
                            memberAccountId: friend.accountId,
                          );
                      ref.invalidate(
                        conversationParticipantsProvider(conversationId),
                      );
                      if (context.mounted) {
                        showMinosToast(
                          context,
                          title: '已添加 ${friend.displayName}',
                        );
                      }
                    } catch (error) {
                      if (context.mounted) {
                        showLoggedErrorToast(
                          context,
                          target: 'group_members',
                          title: '添加成员失败',
                          error: error,
                        );
                      }
                    }
                  },
                ),
            ],
          ),
        );
      },
    );
  }

  Future<void> _showAddAgentSheet(BuildContext context, WidgetRef ref) async {
    final workspaceState = ref
        .read(agentProfilesControllerProvider)
        .asData
        ?.value;
    if (workspaceState == null || workspaceState.profiles.isEmpty) {
      if (context.mounted) {
        showMinosToast(context, title: '请先创建一个 Agent');
      }
      return;
    }

    final existingAgents = ref.read(groupAgentsProvider(conversationId));
    final existingAgentIds = existingAgents
        .map((agent) => agent.agentId)
        .toSet();
    final available = workspaceState.profiles
        .where((profile) => !existingAgentIds.contains(profile.agentId))
        .toList();

    if (available.isEmpty) {
      if (context.mounted) {
        showMinosToast(context, title: '所有 Agent 都已在群中');
      }
      return;
    }

    if (!context.mounted) return;

    await showModalBottomSheet<void>(
      context: context,
      useSafeArea: true,
      builder: (sheetContext) {
        final theme = Theme.of(sheetContext);
        return SafeArea(
          child: ListView(
            shrinkWrap: true,
            children: <Widget>[
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
                child: Text('添加 Agent 到群聊', style: theme.textTheme.titleLarge),
              ),
              for (final profile in available)
                ListTile(
                  leading: const Icon(CupertinoIcons.gear_alt_fill),
                  title: Text(profile.name),
                  subtitle: Text(
                    '${profile.runtimeAgent.name} · ${profile.model}',
                  ),
                  onTap: () async {
                    Navigator.of(sheetContext).pop();
                    try {
                      await ref
                          .read(socialActionsProvider)
                          .addAgentToConversation(
                            conversationId: conversationId,
                            agentId: profile.agentId,
                          );
                      ref.invalidate(
                        conversationParticipantsProvider(conversationId),
                      );
                      if (context.mounted) {
                        showMinosToast(
                          context,
                          title: '已添加 Agent: ${profile.name}',
                        );
                      }
                    } catch (error) {
                      if (context.mounted) {
                        showLoggedErrorToast(
                          context,
                          target: 'group_members',
                          title: '添加 Agent 失败',
                          error: error,
                        );
                      }
                    }
                  },
                ),
            ],
          ),
        );
      },
    );
  }
}

class _SectionHeader extends StatelessWidget {
  const _SectionHeader({required this.title, this.trailing});

  final String title;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(4, 0, 4, 8),
      child: Row(
        children: <Widget>[
          Expanded(
            child: Text(
              title,
              style: theme.textTheme.bodySmall?.copyWith(
                color: colors.textSecondary,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}

class _UserMembersList extends ConsumerWidget {
  const _UserMembersList({
    required this.conversationId,
    required this.members,
    required this.myAccountId,
  });

  final String conversationId;
  final List<UserSummary> members;
  final String? myAccountId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (members.isEmpty) {
      return const Padding(padding: EdgeInsets.all(16), child: Text('暂无成员'));
    }
    return Column(
      children: <Widget>[
        for (final member in members)
          ListTile(
            leading: CircleAvatar(
              radius: 18,
              child: Text(
                member.displayName.isNotEmpty
                    ? member.displayName[0].toUpperCase()
                    : '?',
              ),
            ),
            title: Text(member.displayName),
            subtitle: Text('@${member.minosId}'),
            trailing: member.accountId == myAccountId
                ? null
                : IconButton(
                    icon: const Icon(
                      CupertinoIcons.person_badge_minus,
                      size: 18,
                    ),
                    tooltip: '移除成员',
                    onPressed: () => _removeMember(context, ref, member),
                  ),
          ),
      ],
    );
  }

  Future<void> _removeMember(
    BuildContext context,
    WidgetRef ref,
    UserSummary member,
  ) async {
    final colors = context.minosColors;
    final confirmed = await showCupertinoDialog<bool>(
      context: context,
      builder: (dialogContext) => CupertinoAlertDialog(
        title: const Text('移除成员？'),
        content: Text('确定要将 ${member.displayName} 从群聊中移除吗？'),
        actions: <Widget>[
          CupertinoDialogAction(
            onPressed: () => Navigator.of(dialogContext).pop(false),
            child: Text('取消', style: TextStyle(color: colors.textSecondary)),
          ),
          CupertinoDialogAction(
            isDestructiveAction: true,
            onPressed: () => Navigator.of(dialogContext).pop(true),
            child: const Text('移除'),
          ),
        ],
      ),
    );
    if (confirmed != true) {
      return;
    }
    try {
      await ref
          .read(socialActionsProvider)
          .removeGroupMember(
            conversationId: conversationId,
            memberAccountId: member.accountId,
          );
      ref.invalidate(conversationParticipantsProvider(conversationId));
      ref.invalidate(conversationsProvider);
    } catch (error) {
      if (!context.mounted) return;
      showLoggedErrorToast(
        context,
        target: 'group_members',
        title: '移除成员失败',
        error: error,
      );
    }
  }
}

class _AgentMembersList extends ConsumerWidget {
  const _AgentMembersList({required this.conversationId, required this.agents});

  final String conversationId;
  final List<AgentProfile> agents;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final colors = context.minosColors;
    return Column(
      children: <Widget>[
        for (final agent in agents)
          ListTile(
            leading: CircleAvatar(
              radius: 18,
              backgroundColor: colors.accent,
              child: Icon(
                CupertinoIcons.gear_alt_fill,
                size: 18,
                color: colors.textOnAccent,
              ),
            ),
            title: Text(agent.name),
            subtitle: Text('${agent.runtimeAgent.name} · @${agent.agentId}'),
            trailing: IconButton(
              icon: const Icon(CupertinoIcons.person_badge_minus, size: 18),
              tooltip: '移除 Agent',
              onPressed: () async {
                final confirmed = await showCupertinoDialog<bool>(
                  context: context,
                  builder: (dialogContext) => CupertinoAlertDialog(
                    title: const Text('移除 Agent？'),
                    content: Text('确定要将 ${agent.name} 从群聊中移除吗？'),
                    actions: <Widget>[
                      CupertinoDialogAction(
                        onPressed: () => Navigator.of(dialogContext).pop(false),
                        child: Text(
                          '取消',
                          style: TextStyle(color: colors.textSecondary),
                        ),
                      ),
                      CupertinoDialogAction(
                        isDestructiveAction: true,
                        onPressed: () => Navigator.of(dialogContext).pop(true),
                        child: const Text('移除'),
                      ),
                    ],
                  ),
                );
                if (confirmed == true) {
                  try {
                    await ref
                        .read(socialActionsProvider)
                        .removeAgentFromConversation(
                          conversationId: conversationId,
                          agentId: agent.agentId,
                        );
                    ref.invalidate(
                      conversationParticipantsProvider(conversationId),
                    );
                  } catch (error) {
                    if (context.mounted) {
                      showLoggedErrorToast(
                        context,
                        target: 'group_members',
                        title: '移除 Agent 失败',
                        error: error,
                      );
                    }
                  }
                }
              },
            ),
          ),
      ],
    );
  }
}
