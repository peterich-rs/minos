// ignore_for_file: unused_element

import 'dart:async';

import 'package:flutter/cupertino.dart' hide ConnectionState;
import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/preferred_agent_provider.dart';
import 'package:minos/application/project_providers.dart';
import 'package:minos/application/runtime_actions.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/application/thread_commands.dart';
import 'package:minos/application/ui_state_providers.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/core/widgets/shimmer_box.dart';
import 'package:minos/ui/features/agents/views/agents_hub_page.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:minos/ui/features/social/views/social_hub_page.dart';
import 'package:minos/ui/features/social/widgets/social_management_sections.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

const String _appVersion = '1.0.0';

/// iOS-flat shell: three tabs (Messages / Partners / Profile) with a sticky
/// large title per tab, a thin top divider, and a Material 3
/// [NavigationBar] themed to match.
class AppShellPage extends ConsumerWidget {
  const AppShellPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final socialUnreadCount = ref.watch(socialUnreadCountProvider);
    final tabIndex = ref.watch(shellTabIndexProvider);
    return Scaffold(
      backgroundColor: _scaffoldBg(context),
      body: IndexedStack(
        index: tabIndex,
        children: const <Widget>[_MessagesTab(), _AgentsTab(), _ProfileTab()],
      ),
      bottomNavigationBar: _BottomNav(
        index: tabIndex,
        unreadCount: socialUnreadCount,
        onChanged: (i) => ref.read(shellTabIndexProvider.notifier).select(i),
      ),
    );
  }
}

// ─────────────────────────── Messages tab ───────────────────────────

class _MessagesTab extends ConsumerWidget {
  const _MessagesTab();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return const SocialHubPage(showAppBar: false);
  }

  void _showCreateProjectDialog(BuildContext context, WidgetRef ref) {
    final nameController = TextEditingController();
    final slugController = TextEditingController();

    unawaited(
      showDialog<void>(
        context: context,
        builder: (ctx) => AlertDialog(
          title: const Text('新建项目'),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextField(
                controller: nameController,
                decoration: const InputDecoration(
                  labelText: '项目名称',
                  hintText: '例如: My App',
                ),
                autofocus: true,
                onChanged: (value) {
                  if (slugController.text.isEmpty ||
                      slugController.text ==
                          _slugify(
                            nameController.text.substring(
                              0,
                              nameController.text.length > 1
                                  ? nameController.text.length - 1
                                  : 0,
                            ),
                          )) {
                    slugController.text = _slugify(value);
                  }
                },
              ),
              const SizedBox(height: 12),
              TextField(
                controller: slugController,
                decoration: const InputDecoration(
                  labelText: 'Workspace 文件夹名',
                  hintText: '例如: my-app',
                ),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(ctx),
              child: const Text('取消'),
            ),
            FilledButton(
              onPressed: () async {
                final name = nameController.text.trim();
                final slug = slugController.text.trim();
                if (name.isEmpty || slug.isEmpty) return;
                Navigator.pop(ctx);
                await ref
                    .read(projectListProvider.notifier)
                    .createProject(name: name, workspaceSlug: slug);
              },
              child: const Text('创建'),
            ),
          ],
        ),
      ),
    );
  }

  static String _slugify(String input) {
    return input
        .toLowerCase()
        .replaceAll(RegExp(r'[^a-z0-9\u4e00-\u9fff]+'), '-')
        .replaceAll(RegExp(r'^-+|-+$'), '');
  }
}

class _DiscordProjectsPane extends ConsumerStatefulWidget {
  const _DiscordProjectsPane({required this.projectsAsync});

  final AsyncValue<List<ProjectSummary>> projectsAsync;

  @override
  ConsumerState<_DiscordProjectsPane> createState() =>
      _DiscordProjectsPaneState();
}

class _DiscordProjectsPaneState extends ConsumerState<_DiscordProjectsPane> {
  @override
  Widget build(BuildContext context) {
    final collapsedProjectIds = ref.watch(collapsedProjectIdsProvider);
    return widget.projectsAsync.when(
      loading: () => const _MessagesListSkeleton(),
      error: (error, _) => _CupertinoRefreshScrollView(
        onRefresh: () async {
          try {
            await ref.read(projectListProvider.notifier).refresh();
          } catch (error) {
            if (context.mounted) {
              _showRefreshError(context, '项目刷新失败', error);
            }
          }
        },
        slivers: <Widget>[
          SliverFillRemaining(
            hasScrollBody: false,
            child: _InlineErrorState(
              title: '项目暂时不可用',
              description: error.toString(),
            ),
          ),
        ],
      ),
      data: (projects) => _CupertinoRefreshScrollView(
        onRefresh: () async {
          try {
            await _refreshProjects(projects);
          } catch (error) {
            if (context.mounted) {
              _showRefreshError(context, '项目刷新失败', error);
            }
          }
        },
        slivers: <Widget>[
          if (projects.isEmpty)
            const SliverFillRemaining(
              hasScrollBody: false,
              child: Padding(
                padding: EdgeInsets.fromLTRB(24, 80, 24, 24),
                child: _EmptyState(
                  icon: LucideIcons.folderOpen,
                  title: '还没有项目',
                  subtitle: '创建一个项目后，项目内的 thread 会作为群聊出现在这里。',
                ),
              ),
            )
          else
            SliverPadding(
              padding: const EdgeInsets.fromLTRB(12, 4, 12, 24),
              sliver: SliverList(
                delegate: SliverChildBuilderDelegate((context, index) {
                  final project = projects[index];
                  return Padding(
                    padding: const EdgeInsets.only(bottom: 10),
                    child: _ProjectThreadGroup(
                      project: project,
                      collapsed: collapsedProjectIds.contains(
                        project.projectId,
                      ),
                      onToggle: () => _toggleProject(project.projectId),
                      onNewThread: () =>
                          _showCreateThreadDialog(context, ref, project),
                    ),
                  );
                }, childCount: projects.length),
              ),
            ),
        ],
      ),
    );
  }

  void _toggleProject(String projectId) {
    ref.read(collapsedProjectIdsProvider.notifier).toggle(projectId);
  }

  Future<void> _refreshProjects(List<ProjectSummary> fallbackProjects) async {
    await ref.read(projectListProvider.notifier).refresh();
    final refreshed = ref.read(projectListProvider);
    final projects = refreshed.hasValue
        ? refreshed.requireValue
        : fallbackProjects;
    await Future.wait(<Future<void>>[
      for (final project in projects)
        ref
            .read(projectSessionsProvider(project.projectId).notifier)
            .refresh()
            .catchError((_) {}),
    ]);
  }

  Future<void> _showCreateThreadDialog(
    BuildContext context,
    WidgetRef ref,
    ProjectSummary project,
  ) async {
    final preferredProfile = ref.read(preferredAgentProfileProvider);
    final agentName = preferredProfile?.runtimeAgent ?? AgentName.codex;
    final displayName = preferredProfile?.name ?? _agentLabel(agentName);
    final promptController = TextEditingController();
    final prompt = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('新建对话 · ${project.name}'),
        content: TextField(
          controller: promptController,
          autofocus: true,
          maxLines: 3,
          decoration: InputDecoration(
            hintText: '$displayName 要处理什么？',
            border: const OutlineInputBorder(),
          ),
        ),
        actions: <Widget>[
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, promptController.text.trim()),
            child: const Text('开始'),
          ),
        ],
      ),
    );
    promptController.dispose();
    if (prompt == null || prompt.isEmpty) return;

    try {
      // In the refactored architecture, mobile sends messages directly.
      // The host auto-creates a session when session_id is empty.
      await ref
          .read(threadCommandsProvider)
          .sendUserMessage(sessionId: '', text: prompt);
      ref.read(selectedProjectProvider.notifier).select(project.projectId);
      ref.invalidate(projectListProvider);
      ref.invalidate(projectSessionsProvider(project.projectId));
      if (!context.mounted) return;
      unawaited(
        context.push(
          AppRoutes.newThread,
          extra: ThreadRouteExtra(agent: agentName),
        ),
      );
    } catch (error) {
      if (!context.mounted) return;
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text('启动失败: $error')));
    }
  }
}

class _ProjectThreadGroup extends ConsumerWidget {
  const _ProjectThreadGroup({
    required this.project,
    required this.collapsed,
    required this.onToggle,
    required this.onNewThread,
  });

  final ProjectSummary project;
  final bool collapsed;
  final VoidCallback onToggle;
  final VoidCallback onNewThread;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final threadsAsync = collapsed
        ? null
        : ref.watch(projectSessionsProvider(project.projectId));

    return DecoratedBox(
      decoration: BoxDecoration(
        color: theme.colorScheme.surfaceContainerLow,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.35),
        ),
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(8),
        child: Column(
          children: <Widget>[
            _ProjectGroupHeader(
              project: project,
              collapsed: collapsed,
              onTap: onToggle,
              onNewThread: onNewThread,
            ),
            if (!collapsed) ...<Widget>[
              const _RowDivider(indent: 64),
              if (threadsAsync == null)
                const SizedBox.shrink()
              else
                threadsAsync.when(
                  loading: () => const _ProjectThreadSkeletonList(),
                  error: (error, _) => _ProjectThreadsError(
                    error: error,
                    onRetry: () => ref
                        .read(
                          projectSessionsProvider(project.projectId).notifier,
                        )
                        .refresh(),
                  ),
                  data: (sessions) => sessions.isEmpty
                      ? _EmptyProjectThreads(onNewThread: onNewThread)
                      : _ProjectThreadList(
                          projectId: project.projectId,
                          sessions: sessions,
                        ),
                ),
            ],
          ],
        ),
      ),
    );
  }
}

class _ProjectGroupHeader extends StatelessWidget {
  const _ProjectGroupHeader({
    required this.project,
    required this.collapsed,
    required this.onTap,
    required this.onNewThread,
  });

  final ProjectSummary project;
  final bool collapsed;
  final VoidCallback onTap;
  final VoidCallback onNewThread;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final projectColor = _projectColor(project.projectId);
    final workspaceLabel = _projectWorkspaceLabel(project);
    return Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(12, 12, 8, 12),
          child: Row(
            children: <Widget>[
              Icon(
                collapsed ? LucideIcons.chevronRight : LucideIcons.chevronDown,
                size: 18,
                color: theme.colorScheme.outline,
              ),
              const SizedBox(width: 8),
              _ProjectAvatar(project: project, color: projectColor),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: <Widget>[
                    Text(
                      project.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.titleSmall?.copyWith(
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                    const SizedBox(height: 2),
                    Text(
                      '${project.threadCount} 个对话 · $workspaceLabel',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
              Tooltip(
                message: '新建对话',
                child: ShadIconButton.ghost(
                  icon: const Icon(LucideIcons.messageSquarePlus),
                  iconSize: 18,
                  width: 36,
                  height: 36,
                  onPressed: onNewThread,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _ProjectAvatar extends StatelessWidget {
  const _ProjectAvatar({required this.project, required this.color});

  final ProjectSummary project;
  final Color color;

  @override
  Widget build(BuildContext context) {
    final isDark = Theme.of(context).brightness == Brightness.dark;
    return Container(
      width: 36,
      height: 36,
      decoration: BoxDecoration(
        color: color.withValues(alpha: isDark ? 0.30 : 0.16),
        borderRadius: BorderRadius.circular(8),
      ),
      alignment: Alignment.center,
      child: Text(
        project.name.isNotEmpty ? project.name[0].toUpperCase() : '?',
        style: TextStyle(
          color: color,
          fontWeight: FontWeight.w800,
          fontSize: 16,
        ),
      ),
    );
  }
}

class _ProjectThreadList extends StatelessWidget {
  const _ProjectThreadList({required this.projectId, required this.sessions});

  final String projectId;
  final List<SessionSummary> sessions;

  @override
  Widget build(BuildContext context) {
    return Column(
      children: <Widget>[
        const _ThreadCategoryHeader(),
        for (var index = 0; index < sessions.length; index++) ...<Widget>[
          _ProjectThreadTile(projectId: projectId, thread: sessions[index]),
          if (index < sessions.length - 1) const _RowDivider(indent: 64),
        ],
      ],
    );
  }
}

class _ThreadCategoryHeader extends StatelessWidget {
  const _ThreadCategoryHeader();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
      child: Row(
        children: <Widget>[
          Icon(
            LucideIcons.messagesSquare,
            size: 14,
            color: theme.colorScheme.outline,
          ),
          const SizedBox(width: 6),
          Text(
            '群聊',
            style: theme.textTheme.labelSmall?.copyWith(
              color: theme.colorScheme.outline,
              fontWeight: FontWeight.w700,
              letterSpacing: 0,
            ),
          ),
        ],
      ),
    );
  }
}

class _ProjectThreadTile extends ConsumerWidget {
  const _ProjectThreadTile({required this.projectId, required this.thread});

  final String projectId;
  final SessionSummary thread;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final ended = thread.endedAtMs != null;
    return Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: () {
          ref.read(selectedProjectProvider.notifier).select(projectId);
          unawaited(
            context.push(
              '/thread/${thread.sessionId}',
              extra: ThreadRouteExtra(agent: thread.agent),
            ),
          );
        },
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 10, 10, 10),
          child: Row(
            children: <Widget>[
              SizedBox(
                width: 36,
                height: 36,
                child: Stack(
                  clipBehavior: Clip.none,
                  children: <Widget>[
                    _AgentAvatar(agent: thread.agent, size: 32),
                    Positioned(
                      right: -1,
                      bottom: -1,
                      child: Container(
                        width: 14,
                        height: 14,
                        decoration: BoxDecoration(
                          color: theme.colorScheme.surfaceContainerLow,
                          shape: BoxShape.circle,
                        ),
                        alignment: Alignment.center,
                        child: Icon(
                          LucideIcons.usersRound,
                          size: 9,
                          color: ended
                              ? theme.colorScheme.outline
                              : theme.colorScheme.primary,
                        ),
                      ),
                    ),
                  ],
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
                            _threadTitle(thread),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: theme.textTheme.bodyMedium?.copyWith(
                              fontWeight: FontWeight.w700,
                              color: ended
                                  ? theme.colorScheme.onSurfaceVariant
                                  : theme.colorScheme.onSurface,
                            ),
                          ),
                        ),
                        const SizedBox(width: 8),
                        Text(
                          _formatRelativeTimestamp(thread.lastTsMs.toInt()),
                          style: theme.textTheme.labelSmall?.copyWith(
                            color: theme.colorScheme.outline,
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 3),
                    Text(
                      '群聊会话 · ${thread.messageCount} 条消息',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              Icon(
                ended ? LucideIcons.lock : LucideIcons.chevronRight,
                size: 16,
                color: theme.colorScheme.outline,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _EmptyProjectThreads extends StatelessWidget {
  const _EmptyProjectThreads({required this.onNewThread});

  final VoidCallback onNewThread;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 18, 16, 20),
      child: Column(
        children: <Widget>[
          Icon(
            LucideIcons.messagesSquare,
            size: 30,
            color: theme.colorScheme.outline,
          ),
          const SizedBox(height: 8),
          Text(
            '还没有对话',
            style: theme.textTheme.bodyMedium?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
              fontWeight: FontWeight.w600,
            ),
          ),
          const SizedBox(height: 12),
          ShadButton.outline(
            onPressed: onNewThread,
            child: const Row(
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                Icon(LucideIcons.messageSquarePlus, size: 16),
                SizedBox(width: 8),
                Text('新建对话'),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _ProjectThreadsError extends StatelessWidget {
  const _ProjectThreadsError({required this.error, required this.onRetry});

  final Object error;
  final Future<void> Function() onRetry;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Row(
            children: <Widget>[
              Icon(
                LucideIcons.circleAlert,
                size: 16,
                color: theme.colorScheme.error,
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  '群聊加载失败',
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: theme.colorScheme.error,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
              TextButton(onPressed: onRetry, child: const Text('重试')),
            ],
          ),
          const SizedBox(height: 4),
          Text(
            error.toString(),
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

class _ProjectThreadSkeletonList extends StatelessWidget {
  const _ProjectThreadSkeletonList();

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 14, 16, 16),
      child: Column(
        children: <Widget>[
          for (var index = 0; index < 3; index++) ...<Widget>[
            const Row(
              children: <Widget>[
                ShimmerBox(width: 32, height: 32, circular: true),
                SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: <Widget>[
                      ShimmerBox(width: 160, height: 13),
                      SizedBox(height: 7),
                      ShimmerBox(width: 110, height: 11),
                    ],
                  ),
                ),
              ],
            ),
            if (index < 2) const SizedBox(height: 16),
          ],
        ],
      ),
    );
  }
}

String _threadTitle(SessionSummary thread) {
  final title = thread.title?.trim();
  if (title != null && title.isNotEmpty) return title;
  final ts = DateTime.fromMillisecondsSinceEpoch(thread.lastTsMs.toInt());
  return '${_agentLabel(thread.agent)} · ${ts.month}/${ts.day} ${_two(ts.hour)}:${_two(ts.minute)}';
}

String _formatRelativeTimestamp(int ms) {
  final now = DateTime.now();
  final ts = DateTime.fromMillisecondsSinceEpoch(ms).toLocal();
  final diff = now.difference(ts);

  if (diff.inSeconds < 60) return '刚刚';
  if (diff.inMinutes < 60) return '${diff.inMinutes} 分钟前';

  final today = DateTime(now.year, now.month, now.day);
  final tsDay = DateTime(ts.year, ts.month, ts.day);
  if (tsDay == today) return '${_two(ts.hour)}:${_two(ts.minute)}';

  final yesterday = today.subtract(const Duration(days: 1));
  if (tsDay == yesterday) return '昨天';

  if (ts.year == now.year) return '${_two(ts.month)}-${_two(ts.day)}';
  return '${ts.year}-${_two(ts.month)}-${_two(ts.day)}';
}

String _two(int n) => n.toString().padLeft(2, '0');

String _projectWorkspaceLabel(ProjectSummary project) {
  final path = project.workspacePath?.trim();
  return path == null || path.isEmpty ? project.workspaceSlug : path;
}

Color _projectColor(String id) {
  const colors = <Color>[
    Color(0xFF2563EB),
    Color(0xFF7C3AED),
    Color(0xFF0F766E),
    Color(0xFFEA580C),
    Color(0xFFDB2777),
    Color(0xFF4F46E5),
    Color(0xFF16A34A),
    Color(0xFFD97706),
  ];
  return colors[id.hashCode.abs() % colors.length];
}

class _MessagesListSkeleton extends StatelessWidget {
  const _MessagesListSkeleton();

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.only(top: 8),
      physics: const NeverScrollableScrollPhysics(),
      children: List.generate(
        6,
        (index) => const Padding(
          padding: EdgeInsets.symmetric(horizontal: 16, vertical: 14),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              ShimmerBox(width: 48, height: 48, circular: true),
              SizedBox(width: 14),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    ShimmerBox(width: 120, height: 14),
                    SizedBox(height: 8),
                    ShimmerBox(width: double.infinity, height: 12),
                    SizedBox(height: 6),
                    ShimmerBox(width: 180, height: 12),
                  ],
                ),
              ),
              SizedBox(width: 12),
              ShimmerBox(width: 40, height: 12),
            ],
          ),
        ),
      ),
    );
  }
}

// ─────────────────────────── Partners tab ───────────────────────────

class _AgentsTab extends ConsumerWidget {
  const _AgentsTab();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return const AgentsHubTab();
  }
}

class _RuntimeProfilePage extends ConsumerWidget {
  const _RuntimeProfilePage();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final connection = ref.watch(connectionStateProvider).asData?.value;
    final agents = ref.watch(runtimeAgentDescriptorsProvider);
    final preferredAgent = ref.watch(preferredAgentProvider);
    final session = ref.watch(activeSessionControllerProvider);
    final currentSessionAgent = _sessionAgent(session);

    return Scaffold(
      backgroundColor: _scaffoldBg(context),
      body: SafeArea(
        bottom: false,
        child: Column(
          children: <Widget>[
            _LargeTitleBar(
              title: '设备主页',
              subtitle: 'Agent runtime',
              trailing: _CircleIconButton(
                icon: CupertinoIcons.xmark,
                onTap: () => Navigator.of(context).pop(),
                tooltip: '关闭',
              ),
            ),
            Expanded(
              child: _CupertinoRefreshScrollView(
                onRefresh: () async {
                  ref.invalidate(runtimeAgentDescriptorsProvider);
                  await ref.read(runtimeAgentDescriptorsProvider.future);
                },
                slivers: <Widget>[
                  SliverToBoxAdapter(
                    child: Padding(
                      padding: const EdgeInsets.fromLTRB(16, 8, 16, 28),
                      child: Column(
                        children: <Widget>[
                          _RuntimeProfileHeader(connection: connection),
                          const SizedBox(height: 14),
                          Row(
                            children: <Widget>[
                              Expanded(
                                child: FilledButton.icon(
                                  onPressed: () => _openNewChat(context, ref),
                                  icon: const Icon(CupertinoIcons.bubble_left),
                                  label: const Text('发起会话'),
                                ),
                              ),
                              const SizedBox(width: 10),
                              Expanded(
                                child: OutlinedButton.icon(
                                  onPressed: () =>
                                      _confirmForgetActiveMac(context, ref),
                                  icon: const Icon(CupertinoIcons.delete),
                                  label: const Text('删除伙伴'),
                                ),
                              ),
                            ],
                          ),
                          const SizedBox(height: 18),
                          agents.when(
                            loading: () => const _PartnerLoadingCard(),
                            error: (error, _) => _PartnerEmptyCard(
                              icon: CupertinoIcons.exclamationmark_triangle,
                              title: 'Agent 检测失败',
                              subtitle: error.toString(),
                            ),
                            data: (items) {
                              if (items.isEmpty) {
                                return const _PartnerEmptyCard(
                                  icon: CupertinoIcons.command,
                                  title: '没有检测到 Agent CLI',
                                  subtitle: 'runtime detect 未返回可展示的 CLI。',
                                );
                              }
                              return _GroupedCard(
                                children: <Widget>[
                                  for (
                                    var i = 0;
                                    i < items.length;
                                    i++
                                  ) ...<Widget>[
                                    if (i > 0) const _RowDivider(indent: 56),
                                    _RuntimeAgentRow(
                                      descriptor: items[i],
                                      selected: preferredAgent == items[i].name,
                                      active:
                                          currentSessionAgent == items[i].name,
                                      onTap: _agentAvailable(items[i])
                                          ? () {}
                                          : null,
                                    ),
                                  ],
                                ],
                              );
                            },
                          ),
                          const SizedBox(height: 12),
                          const _GroupedFooter(
                            text: '选择默认 Agent 只影响新建会话；已有 thread 会保持当前运行上下文。',
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  void _openNewChat(BuildContext context, WidgetRef ref) {
    ref.read(activeSessionControllerProvider.notifier).reset();
    unawaited(context.push(AppRoutes.agentStart));
  }
}

class _CupertinoRefreshScrollView extends StatelessWidget {
  const _CupertinoRefreshScrollView({
    required this.onRefresh,
    required this.slivers,
  });

  final Future<void> Function() onRefresh;
  final List<Widget> slivers;

  @override
  Widget build(BuildContext context) {
    return CustomScrollView(
      physics: const BouncingScrollPhysics(
        parent: AlwaysScrollableScrollPhysics(),
      ),
      slivers: <Widget>[
        CupertinoSliverRefreshControl(onRefresh: onRefresh),
        ...slivers,
      ],
    );
  }
}

class _SectionLabel extends StatelessWidget {
  const _SectionLabel(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(4, 4, 4, 8),
      child: Text(
        text,
        style: Theme.of(context).textTheme.labelMedium?.copyWith(
          color: Theme.of(context).colorScheme.onSurfaceVariant,
          fontWeight: FontWeight.w700,
        ),
      ),
    );
  }
}

class _PartnerLoadingCard extends StatelessWidget {
  const _PartnerLoadingCard();

  @override
  Widget build(BuildContext context) {
    return const _GroupedCard(
      children: <Widget>[
        Padding(
          padding: EdgeInsets.all(18),
          child: Center(child: CupertinoActivityIndicator()),
        ),
      ],
    );
  }
}

class _PartnerEmptyCard extends StatelessWidget {
  const _PartnerEmptyCard({
    required this.icon,
    required this.title,
    required this.subtitle,
  });

  final IconData icon;
  final String title;
  final String subtitle;

  @override
  Widget build(BuildContext context) {
    return _GroupedCard(
      children: <Widget>[
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 20, 18, 20),
          child: _EmptyState(icon: icon, title: title, subtitle: subtitle),
        ),
      ],
    );
  }
}

/// One row per paired Mac in the Partners tab. Tap = set as active routing
/// target; long-press shows a delete confirm. The row mirrors the visual
/// shape of the legacy single-peer `_RuntimePartnerRow`: avatar +
/// name/connection-line. The active Mac gets a checkmark on the right.
///
/// UI gap (deviceshell-MVP): no swipe-to-forget gesture yet — long-press is
/// the only delete affordance. The Add Partner row + RefreshIndicator on
/// the parent ListView cover the other interactions.
class _MacRow extends StatelessWidget {
  const _MacRow({
    required this.mac,
    required this.isActive,
    required this.connection,
    required this.onTap,
    required this.onForget,
  });

  final HostSummaryDto mac;
  final bool isActive;
  final ConnectionState? connection;
  final VoidCallback onTap;
  final VoidCallback onForget;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final name = mac.hostDisplayName.trim().isEmpty
        ? 'Agent Runtime'
        : mac.hostDisplayName.trim();
    final runtimeLabel = mac.online ? 'Runtime 在线' : 'Runtime 离线';
    return InkWell(
      onTap: onTap,
      onLongPress: onForget,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        child: Row(
          children: <Widget>[
            const _RuntimeAvatar(size: 42),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    name,
                    style: theme.textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: 3),
                  if (isActive)
                    Text(
                      '$runtimeLabel · 当前路由 · App ${_connectionStatusText(connection)}',
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    )
                  else
                    Text(
                      '$runtimeLabel · 点击切换为当前路由目标',
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: 8),
            if (isActive)
              Icon(
                CupertinoIcons.check_mark,
                size: 20,
                color: theme.colorScheme.primary,
              )
            else
              Icon(
                CupertinoIcons.chevron_right,
                size: 16,
                color: theme.colorScheme.outline,
              ),
          ],
        ),
      ),
    );
  }
}

/// Trailing row in the partners list: tap to enter [PairingPage]. Mirrors
/// the iOS-flat "Add Account" affordance.
class _AddPartnerRow extends StatelessWidget {
  const _AddPartnerRow({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        child: Row(
          children: <Widget>[
            Container(
              width: 42,
              height: 42,
              decoration: BoxDecoration(
                color: theme.colorScheme.primary.withValues(alpha: 0.12),
                borderRadius: BorderRadius.circular(12),
              ),
              alignment: Alignment.center,
              child: Icon(
                CupertinoIcons.qrcode_viewfinder,
                color: theme.colorScheme.primary,
                size: 22,
              ),
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    '添加伙伴',
                    style: theme.textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                      color: theme.colorScheme.primary,
                    ),
                  ),
                  const SizedBox(height: 3),
                  Text(
                    '扫描 runtime 二维码',
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
            Icon(
              CupertinoIcons.chevron_right,
              size: 16,
              color: theme.colorScheme.outline,
            ),
          ],
        ),
      ),
    );
  }
}

class _RuntimeProfileHeader extends ConsumerWidget {
  const _RuntimeProfileHeader({required this.connection});

  final ConnectionState? connection;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final name = _resolvePeerDisplayName(ref);
    return Container(
      padding: const EdgeInsets.all(18),
      decoration: BoxDecoration(
        color: theme.colorScheme.surface,
        borderRadius: BorderRadius.circular(18),
      ),
      child: Row(
        children: <Widget>[
          const _RuntimeAvatar(size: 62),
          const SizedBox(width: 16),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(
                  name,
                  style: theme.textTheme.titleLarge?.copyWith(
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const SizedBox(height: 5),
                _ConnectionLine(state: connection),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// Resolves a display name for the *active* Mac partner. After ADR-0020
/// the Mac name comes from the server's `account_mac_pairings` row,
/// surfaced via `pairedMacsProvider`. Falls back to the cached
/// `peerDisplayName` (kept for legacy startup transitions) and finally
/// a generic label so the row never renders empty.
String _resolvePeerDisplayName(WidgetRef ref) {
  const fallback = 'Agent Runtime';
  final activeId = ref.watch(activeMacProvider).asData?.value;
  final macs = ref.watch(pairedMacsProvider).asData?.value;
  if (activeId != null && macs != null) {
    final match = macs.where((m) => m.hostDeviceId == activeId).toList();
    if (match.isNotEmpty) {
      final trimmed = match.first.hostDisplayName.trim();
      if (trimmed.isNotEmpty) return trimmed;
    }
  }
  final cached = ref.watch(peerDisplayNameProvider).asData?.value;
  if (cached == null) return fallback;
  final trimmed = cached.trim();
  return trimmed.isEmpty ? fallback : trimmed;
}

class _RuntimeAvatar extends StatelessWidget {
  const _RuntimeAvatar({required this.size});

  final double size;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(size * 0.28),
        color: isDark ? const Color(0xFF1E3A8A) : const Color(0xFFDBEAFE),
      ),
      alignment: Alignment.center,
      child: Icon(
        CupertinoIcons.desktopcomputer,
        color: isDark ? const Color(0xFF60A5FA) : const Color(0xFF2563EB),
        size: size * 0.48,
      ),
    );
  }
}

class _RuntimeAgentRow extends StatelessWidget {
  const _RuntimeAgentRow({
    required this.descriptor,
    required this.selected,
    required this.active,
    required this.onTap,
  });

  final AgentDescriptor descriptor;
  final bool selected;
  final bool active;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final available = _agentAvailable(descriptor);
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        child: Row(
          children: <Widget>[
            _AgentAvatar(agent: descriptor.name, size: 36),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Row(
                    children: <Widget>[
                      Text(
                        _agentLabel(descriptor.name),
                        style: theme.textTheme.titleMedium?.copyWith(
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      if (active) ...<Widget>[
                        const SizedBox(width: 8),
                        const _RunningDot(),
                      ],
                    ],
                  ),
                  const SizedBox(height: 2),
                  Text(
                    _agentDescriptorLine(descriptor),
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: available
                          ? theme.colorScheme.onSurfaceVariant
                          : theme.colorScheme.error,
                      height: 1.35,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 8),
            if (selected)
              Icon(
                CupertinoIcons.check_mark,
                size: 20,
                color: theme.colorScheme.primary,
              )
            else
              const SizedBox(width: 20),
          ],
        ),
      ),
    );
  }
}

class _RunningDot extends StatelessWidget {
  const _RunningDot();

  @override
  Widget build(BuildContext context) {
    final isDark = Theme.of(context).brightness == Brightness.dark;
    return Container(
      width: 6,
      height: 6,
      decoration: BoxDecoration(
        color: isDark ? const Color(0xFF22C55E) : const Color(0xFF16A34A),
        shape: BoxShape.circle,
      ),
    );
  }
}

// ─────────────────────────── Profile tab ───────────────────────────

class _ProfileTab extends ConsumerWidget {
  const _ProfileTab();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final authState = ref.watch(authControllerProvider);
    final email = authState is AuthAuthenticated
        ? authState.account.email
        : '—';
    final connection = ref.watch(connectionStateProvider).asData?.value;
    final theme = Theme.of(context);

    return SafeArea(
      bottom: false,
      child: Column(
        children: <Widget>[
          const _LargeTitleBar(title: '我的'),
          Expanded(
            child: RefreshIndicator(
              onRefresh: () async {
                ref.invalidate(socialProfileProvider);
                await Future.wait<void>(<Future<void>>[
                  ref.read(socialProfileProvider.future).then<void>((_) {}),
                  ref.read(friendRequestsProvider.notifier).refresh(),
                  ref.read(friendsProvider.notifier).refresh(),
                ]);
              },
              child: ListView(
                padding: const EdgeInsets.fromLTRB(16, 8, 16, 28),
                children: <Widget>[
                  _AccountHeader(email: email, connection: connection),
                  const SizedBox(height: 16),
                  const SocialProfileSection(),
                  const SizedBox(height: 16),
                  const FriendSearchSection(),
                  const SizedBox(height: 16),
                  const FriendRequestsSection(),
                  const SizedBox(height: 16),
                  _GroupedCard(
                    children: <Widget>[
                      _SettingsRow(
                        icon: CupertinoIcons.ant,
                        title: 'Devtool',
                        subtitle: '日志与请求追踪',
                        onTap: () => context.push(AppRoutes.logViewer),
                      ),
                      const _RowDivider(indent: 56),
                      _SettingsRow(
                        icon: CupertinoIcons.qrcode_viewfinder,
                        title: '添加伙伴',
                        subtitle: '扫描二维码添加 runtime 设备',
                        onTap: () => context.push(AppRoutes.pairing),
                      ),
                    ],
                  ),
                  const SizedBox(height: 16),
                  _GroupedCard(
                    children: <Widget>[
                      _SettingsRow(
                        icon: CupertinoIcons.square_arrow_right,
                        title: '退出登录',
                        destructive: true,
                        onTap: () => _confirmLogout(context, ref),
                      ),
                    ],
                  ),
                  const SizedBox(height: 24),
                  Center(
                    child: Text(
                      'Minos · v$_appVersion',
                      style: theme.textTheme.labelSmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _confirmLogout(BuildContext context, WidgetRef ref) async {
    final confirmed = await showCupertinoDialog<bool>(
      context: context,
      builder: (ctx) => CupertinoAlertDialog(
        title: const Text('退出登录'),
        content: const Text('当前账户会话会被清除，确认继续？'),
        actions: <Widget>[
          CupertinoDialogAction(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: const Text('取消'),
          ),
          CupertinoDialogAction(
            isDestructiveAction: true,
            onPressed: () => Navigator.of(ctx).pop(true),
            child: const Text('退出'),
          ),
        ],
      ),
    );
    if (confirmed != true) return;
    await ref.read(authControllerProvider.notifier).logout();
  }
}

class _AccountHeader extends StatelessWidget {
  const _AccountHeader({required this.email, required this.connection});

  final String email;
  final ConnectionState? connection;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    final initial = email.isEmpty || email == '—'
        ? '?'
        : email.substring(0, 1).toUpperCase();
    return Container(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 16),
      decoration: BoxDecoration(
        color: theme.colorScheme.surfaceContainerLow,
        borderRadius: BorderRadius.circular(16),
      ),
      child: Row(
        children: <Widget>[
          Container(
            width: 56,
            height: 56,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: isDark ? const Color(0xFF1E3A8A) : const Color(0xFFDBEAFE),
            ),
            alignment: Alignment.center,
            child: Text(
              initial,
              style: TextStyle(
                color: isDark
                    ? const Color(0xFF60A5FA)
                    : const Color(0xFF2563EB),
                fontSize: 22,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.5,
              ),
            ),
          ),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(
                  email,
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.w600,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
                const SizedBox(height: 4),
                _ConnectionLine(state: connection),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _ConnectionLine extends StatelessWidget {
  const _ConnectionLine({required this.state});

  final ConnectionState? state;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    final (label, color) = switch (state) {
      ConnectionState_Connected() => (
        '实时连接在线',
        isDark ? const Color(0xFF22C55E) : const Color(0xFF16A34A),
      ),
      ConnectionState_Reconnecting(:final attempt) => (
        '实时连接重连中 #$attempt',
        isDark ? const Color(0xFFEAB308) : const Color(0xFFCA8A04),
      ),
      ConnectionState_Pairing() => (
        '实时连接初始化中',
        isDark ? const Color(0xFFEAB308) : const Color(0xFFCA8A04),
      ),
      _ => (
        '实时连接离线',
        isDark ? const Color(0xFFEF4444) : const Color(0xFFDC2626),
      ),
    };
    return Row(
      children: <Widget>[
        Container(
          width: 6,
          height: 6,
          decoration: BoxDecoration(color: color, shape: BoxShape.circle),
        ),
        const SizedBox(width: 6),
        Text(
          label,
          style: theme.textTheme.bodySmall?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
      ],
    );
  }
}

String _connectionStatusText(ConnectionState? state) {
  return switch (state) {
    ConnectionState_Connected() => '在线',
    ConnectionState_Reconnecting(:final attempt) => '重连中 #$attempt',
    ConnectionState_Pairing() => '初始化中',
    _ => '离线',
  };
}

class _SettingsRow extends StatelessWidget {
  const _SettingsRow({
    required this.icon,
    required this.title,
    required this.onTap,
    this.subtitle,
    this.destructive = false,
  });

  final IconData icon;
  final String title;
  final String? subtitle;
  final VoidCallback onTap;
  final bool destructive;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tint = destructive
        ? const Color(0xFFFF3B30)
        : theme.colorScheme.onSurface;
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        child: Row(
          children: <Widget>[
            Icon(icon, size: 22, color: tint),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    title,
                    style: theme.textTheme.bodyLarge?.copyWith(color: tint),
                  ),
                  if (subtitle != null) ...<Widget>[
                    const SizedBox(height: 2),
                    Text(
                      subtitle!,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ],
              ),
            ),
            if (!destructive)
              Icon(
                CupertinoIcons.chevron_right,
                size: 16,
                color: theme.colorScheme.outline,
              ),
          ],
        ),
      ),
    );
  }
}

// ─────────────────────────── Reusable shell ───────────────────────────

class _LargeTitleBar extends StatelessWidget {
  const _LargeTitleBar({required this.title, this.subtitle, this.trailing});

  final String title;
  final String? subtitle;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(20, 12, 12, 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: <Widget>[
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(
                  title,
                  style: theme.textTheme.headlineLarge?.copyWith(
                    fontWeight: FontWeight.w700,
                    letterSpacing: 0,
                  ),
                ),
                if (subtitle != null) ...<Widget>[
                  const SizedBox(height: 2),
                  Text(
                    subtitle!,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ],
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}

class _CircleIconButton extends StatelessWidget {
  const _CircleIconButton({
    required this.icon,
    required this.onTap,
    this.tooltip,
  });

  final IconData icon;
  final VoidCallback onTap;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: tooltip ?? '',
      child: ShadIconButton.ghost(
        icon: Icon(icon),
        iconSize: 21,
        width: 40,
        height: 40,
        onPressed: onTap,
      ),
    );
  }
}

class _ShellIcon extends StatelessWidget {
  const _ShellIcon(this.icon, {required this.selected});

  final IconData icon;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return Icon(
      icon,
      size: 21,
      color: selected
          ? shadTheme.colorScheme.foreground
          : shadTheme.colorScheme.mutedForeground,
    );
  }
}

class _ShellLabel extends StatelessWidget {
  const _ShellLabel(this.text, {required this.selected});

  final String text;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return Text(
      text,
      style: shadTheme.textTheme.muted.copyWith(
        fontWeight: selected ? FontWeight.w700 : FontWeight.w500,
        color: selected
            ? shadTheme.colorScheme.foreground
            : shadTheme.colorScheme.mutedForeground,
      ),
    );
  }
}

class _NavCountBadge extends StatelessWidget {
  const _NavCountBadge({required this.count});

  final int count;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: shadTheme.colorScheme.primary,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
        child: Text(
          count > 99 ? '99+' : '$count',
          style: shadTheme.textTheme.small.copyWith(
            color: shadTheme.colorScheme.primaryForeground,
            fontWeight: FontWeight.w700,
          ),
        ),
      ),
    );
  }
}

class _OfflineBanner extends StatelessWidget {
  const _OfflineBanner({required this.state});

  final ConnectionState? state;

  String _label() {
    return switch (state) {
      ConnectionState_Reconnecting(:final attempt) =>
        '当前设备与服务器的实时连接正在重连 #$attempt',
      ConnectionState_Pairing() => '当前设备正在建立实时连接',
      ConnectionState_Disconnected() => '当前设备与服务器的实时连接已断开',
      _ => '当前设备与服务器的实时连接已断开',
    };
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    return Container(
      margin: const EdgeInsets.fromLTRB(16, 4, 16, 8),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: isDark ? const Color(0xFF422006) : const Color(0xFFFEF3C7),
        borderRadius: BorderRadius.circular(10),
        border: Border.all(
          color: isDark ? const Color(0xFF854D0E) : const Color(0xFFFDE68A),
        ),
      ),
      child: Row(
        children: <Widget>[
          Icon(
            LucideIcons.wifiOff,
            size: 16,
            color: isDark ? const Color(0xFFFBBF24) : const Color(0xFFD97706),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              _label(),
              style: TextStyle(
                color: isDark
                    ? const Color(0xFFFDE68A)
                    : const Color(0xFF92400E),
                fontSize: 13,
                fontWeight: FontWeight.w500,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _EmptyState extends StatelessWidget {
  const _EmptyState({
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
    return Column(
      children: <Widget>[
        Icon(icon, size: 44, color: theme.colorScheme.outline),
        const SizedBox(height: 14),
        Text(
          title,
          style: theme.textTheme.titleMedium?.copyWith(
            fontWeight: FontWeight.w600,
          ),
        ),
        const SizedBox(height: 6),
        Text(
          subtitle,
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyMedium?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
            height: 1.4,
          ),
        ),
      ],
    );
  }
}

class _InlineErrorState extends StatelessWidget {
  const _InlineErrorState({required this.title, required this.description});

  final String title;
  final String description;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Icon(
              LucideIcons.circleAlert,
              size: 36,
              color: shadTheme.colorScheme.mutedForeground,
            ),
            const SizedBox(height: 12),
            Text(title, style: shadTheme.textTheme.h4),
            const SizedBox(height: 6),
            Text(
              description,
              textAlign: TextAlign.center,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
              style: shadTheme.textTheme.muted,
            ),
          ],
        ),
      ),
    );
  }
}

class _GroupedCard extends StatelessWidget {
  const _GroupedCard({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        color: theme.colorScheme.surfaceContainerLow,
        borderRadius: BorderRadius.circular(16),
      ),
      child: Column(children: children),
    );
  }
}

class _GroupedFooter extends StatelessWidget {
  const _GroupedFooter({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 4, 16, 4),
      child: Text(
        text,
        style: theme.textTheme.labelSmall?.copyWith(
          color: theme.colorScheme.onSurfaceVariant,
          height: 1.4,
        ),
      ),
    );
  }
}

class _RowDivider extends StatelessWidget {
  const _RowDivider({this.indent = 0});

  final double indent;

  @override
  Widget build(BuildContext context) {
    return Divider(
      height: 0.5,
      thickness: 0.5,
      indent: indent,
      color: Theme.of(context).dividerColor.withValues(alpha: 0.5),
    );
  }
}

/// Forget the Mac currently set as the routing target. Surfaces an error
/// dialog when no Mac is active (post-ADR-0020 forget needs an explicit
/// `host_device_id`). Used from `_RuntimeProfilePage`'s "delete partner"
/// button.
Future<void> _confirmForgetActiveMac(
  BuildContext context,
  WidgetRef ref,
) async {
  final activeId = ref.read(activeMacProvider).asData?.value;
  final macs = ref.read(pairedMacsProvider).asData?.value ?? const [];
  final mac = activeId == null
      ? null
      : macs.where((m) => m.hostDeviceId == activeId).toList();
  if (mac == null || mac.isEmpty) {
    await showCupertinoDialog<void>(
      context: context,
      builder: (ctx) => CupertinoAlertDialog(
        title: const Text('未选中设备伙伴'),
        content: const Text('请回到伙伴列表选择要删除的设备。'),
        actions: <Widget>[
          CupertinoDialogAction(
            onPressed: () => Navigator.of(ctx).pop(),
            child: const Text('好'),
          ),
        ],
      ),
    );
    return;
  }
  await _confirmForgetMac(context, ref, mac.first);
}

/// Forget a specific paired Mac. Shared between the row-level long-press
/// and the runtime-profile delete button.
Future<void> _confirmForgetMac(
  BuildContext context,
  WidgetRef ref,
  HostSummaryDto mac,
) async {
  final name = mac.hostDisplayName.trim().isEmpty
      ? 'Agent Runtime'
      : mac.hostDisplayName.trim();
  final confirmed = await showCupertinoDialog<bool>(
    context: context,
    builder: (ctx) => CupertinoAlertDialog(
      title: const Text('删除设备伙伴'),
      content: Text('删除「$name」后再次使用需要重新扫码配对。'),
      actions: <Widget>[
        CupertinoDialogAction(
          onPressed: () => Navigator.of(ctx).pop(false),
          child: const Text('取消'),
        ),
        CupertinoDialogAction(
          isDestructiveAction: true,
          onPressed: () => Navigator.of(ctx).pop(true),
          child: const Text('删除'),
        ),
      ],
    ),
  );
  if (confirmed != true) return;
  await ref.read(runtimeActionsProvider).forgetHost(mac.hostDeviceId);
  try {
    await ref.read(pairedMacsProvider.notifier).refresh();
  } catch (_) {}
  ref.invalidate(runtimeAgentDescriptorsProvider);
  await ref.read(activeMacProvider.notifier).refresh();
  if (context.mounted && Navigator.of(context).canPop()) {
    Navigator.of(context).pop();
  }
}

class _AgentAvatar extends StatelessWidget {
  const _AgentAvatar({required this.agent, this.size = 32});

  final AgentName agent;
  final double size;

  @override
  Widget build(BuildContext context) {
    final isDark = Theme.of(context).brightness == Brightness.dark;
    final (label, bgColor, fgColor) = switch (agent) {
      AgentName.codex => (
        'C',
        isDark ? const Color(0xFF14532D) : const Color(0xFFDCFCE7),
        isDark ? const Color(0xFF4ADE80) : const Color(0xFF16A34A),
      ),
      AgentName.claude => (
        'A',
        isDark ? const Color(0xFF7C2D12) : const Color(0xFFFFEDD5),
        isDark ? const Color(0xFFFB923C) : const Color(0xFFEA580C),
      ),
      AgentName.gemini => (
        'G',
        isDark ? const Color(0xFF164E63) : const Color(0xFFCFFAFE),
        isDark ? const Color(0xFF22D3EE) : const Color(0xFF0891B2),
      ),
      AgentName.opencode => (
        'O',
        isDark ? const Color(0xFF1E293B) : const Color(0xFFF1F5F9),
        isDark ? const Color(0xFF94A3B8) : const Color(0xFF475569),
      ),
      AgentName.grok => (
        'X',
        isDark ? const Color(0xFF78350F) : const Color(0xFFFEF3C7),
        isDark ? const Color(0xFFFBBF24) : const Color(0xFFB45309),
      ),
    };
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(shape: BoxShape.circle, color: bgColor),
      alignment: Alignment.center,
      child: Text(
        label,
        style: TextStyle(
          color: fgColor,
          fontWeight: FontWeight.w700,
          fontSize: size * 0.42,
          letterSpacing: 0.3,
        ),
      ),
    );
  }
}

class _BottomNav extends StatelessWidget {
  const _BottomNav({
    required this.index,
    required this.unreadCount,
    required this.onChanged,
  });

  final int index;
  final int unreadCount;
  final ValueChanged<int> onChanged;

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    return Material(
      color: shadTheme.colorScheme.background,
      shape: Border(
        top: BorderSide(color: shadTheme.colorScheme.border, width: 1),
      ),
      child: SafeArea(
        top: false,
        child: SizedBox(
          height: 56,
          child: Row(
            children: <Widget>[
              _NavItem(
                icon: LucideIcons.messageCircle,
                activeIcon: LucideIcons.messagesSquare,
                label: '消息',
                badgeCount: unreadCount,
                selected: index == 0,
                onTap: () => onChanged(0),
              ),
              _NavItem(
                icon: LucideIcons.bot,
                activeIcon: LucideIcons.botMessageSquare,
                label: '伙伴',
                selected: index == 1,
                onTap: () => onChanged(1),
              ),
              _NavItem(
                icon: LucideIcons.userRound,
                activeIcon: LucideIcons.circleUserRound,
                label: '我的',
                selected: index == 2,
                onTap: () => onChanged(2),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _NavItem extends StatelessWidget {
  const _NavItem({
    required this.icon,
    required this.activeIcon,
    required this.label,
    this.badgeCount = 0,
    required this.selected,
    required this.onTap,
  });

  final IconData icon;
  final IconData activeIcon;
  final String label;
  final int badgeCount;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Expanded(
      child: ShadButton.ghost(
        onPressed: onTap,
        height: 54,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Stack(
              clipBehavior: Clip.none,
              children: <Widget>[
                _ShellIcon(selected ? activeIcon : icon, selected: selected),
                if (badgeCount > 0)
                  Positioned(
                    right: -10,
                    top: -7,
                    child: _NavCountBadge(count: badgeCount),
                  ),
              ],
            ),
            const SizedBox(height: 3),
            _ShellLabel(label, selected: selected),
          ],
        ),
      ),
    );
  }
}

// ─────────────────────────── helpers ───────────────────────────

Color _scaffoldBg(BuildContext context) {
  return ShadTheme.of(context).colorScheme.background;
}

void _showRefreshError(BuildContext context, String title, Object error) {
  showLoggedErrorToast(
    context,
    target: 'app_shell',
    title: title,
    error: error,
  );
}

String _agentLabel(AgentName agent) {
  return switch (agent) {
    AgentName.codex => 'Codex',
    AgentName.claude => 'Claude',
    AgentName.gemini => 'Gemini',
    AgentName.opencode => 'OpenCode',
    AgentName.grok => 'Grok',
  };
}

bool _agentAvailable(AgentDescriptor descriptor) {
  return switch (descriptor.status) {
    AgentStatus_Ok() => true,
    _ => false,
  };
}

String _avatarInitial(String? value) {
  final trimmed = value?.trim() ?? '';
  if (trimmed.isEmpty) return 'C';
  return trimmed.substring(0, 1).toUpperCase();
}

String _agentDescriptorLine(AgentDescriptor descriptor) {
  final version = descriptor.version;
  final path = descriptor.path;
  final suffix = <String>[
    if (version != null && version.isNotEmpty) version,
    if (path != null && path.isNotEmpty) path,
  ].join(' · ');
  final detail = suffix.isEmpty ? '' : ' · $suffix';
  return switch (descriptor.status) {
    AgentStatus_Ok() => '可用$detail',
    AgentStatus_Missing() => '未安装',
    AgentStatus_Error(:final reason) => '检测异常: $reason',
  };
}

AgentName? _sessionAgent(ActiveSession session) {
  return switch (session) {
    SessionSending(:final agent) => agent,
    SessionStreaming(:final agent) => agent,
    SessionAwaitingInput(:final agent) => agent,
    SessionSuspended(:final agent) => agent,
    _ => null,
  };
}
