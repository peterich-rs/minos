import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/flutter_log.dart';
import 'package:minos/application/project_providers.dart';
import 'package:minos/application/thread_commands.dart';
import 'package:minos/application/ui_state_providers.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/features/chat/views/thread_view_page.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

/// Discord-style project detail page.
/// Shows a sidebar-like thread/channel list on the left with the project name
/// at the top. Tapping a session opens the chat view.
/// Swipe-back (iOS gesture) returns to the project list.
class ProjectDetailPage extends ConsumerWidget {
  const ProjectDetailPage({
    super.key,
    required this.projectId,
    required this.projectName,
  });

  final String projectId;
  final String projectName;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final threadsAsync = ref.watch(projectSessionsProvider(projectId));
    final selectedThreadId = ref.watch(
      selectedProjectThreadProvider(projectId),
    );
    final isWide = MediaQuery.of(context).size.width > 600;

    if (isWide) {
      // Tablet/landscape: side-by-side layout
      return Scaffold(
        backgroundColor: _scaffoldBg(context),
        body: Row(
          children: [
            SizedBox(
              width: 280,
              child: _ChannelSidebar(
                projectId: projectId,
                projectName: projectName,
                threadsAsync: threadsAsync,
                selectedThreadId: selectedThreadId,
                onThreadSelected: (id) => ref
                    .read(selectedProjectThreadProvider(projectId).notifier)
                    .select(id),
                onNewThread: () => _startNewThread(context, ref),
                onDeleteThread: (id) => _deleteThread(context, ref, id),
              ),
            ),
            const VerticalDivider(width: 1),
            Expanded(
              child: selectedThreadId != null
                  ? ThreadViewPage(sessionId: selectedThreadId)
                  : const _NoThreadSelected(),
            ),
          ],
        ),
      );
    }

    // Phone: full-screen channel list, push to thread view
    return Scaffold(
      backgroundColor: _scaffoldBg(context),
      body: _ChannelSidebar(
        projectId: projectId,
        projectName: projectName,
        threadsAsync: threadsAsync,
        selectedThreadId: selectedThreadId,
        onThreadSelected: (id) {
          ref
              .read(selectedProjectThreadProvider(projectId).notifier)
              .select(id);
          unawaited(context.push('/thread/$id'));
        },
        onNewThread: () => _startNewThread(context, ref),
        onDeleteThread: (id) => _deleteThread(context, ref, id),
      ),
    );
  }

  Future<void> _startNewThread(BuildContext context, WidgetRef ref) async {
    final preferredProfile = ref.read(preferredAgentProfileProvider);

    // Show a simple prompt dialog
    final promptController = TextEditingController();
    final displayName = preferredProfile?.name ?? 'Codex';

    final result = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('新建会话 ($displayName)'),
        content: TextField(
          controller: promptController,
          autofocus: true,
          maxLines: 3,
          decoration: const InputDecoration(
            hintText: '输入你的问题或任务...',
            border: OutlineInputBorder(),
          ),
        ),
        actions: [
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

    if (result == null || result.isEmpty) return;
    if (!context.mounted) return;
    logFlutterInfo(
      'project_detail',
      'start project thread requested projectId=$projectId promptLength=${result.length}',
    );

    // Start agent in project — in the refactored architecture, mobile
    // sends messages directly via sendUserMessage. The host auto-creates
    // a session when session_id is empty.
    try {
      await ref
          .read(threadCommandsProvider)
          .sendUserMessage(sessionId: '', text: result);

      // Refresh the session list
      ref.invalidate(projectSessionsProvider(projectId));

      logFlutterInfo(
        'project_detail',
        'project thread dispatch accepted projectId=$projectId',
      );

      if (!context.mounted) return;

      // Navigate to the new thread view (session ID will arrive via events)
      unawaited(context.push(AppRoutes.newThread));
    } catch (error) {
      if (!context.mounted) return;
      showLoggedErrorToast(
        context,
        target: 'project_detail',
        title: '启动会话失败',
        error: error,
      );
    }
  }

  Future<void> _deleteThread(
    BuildContext context,
    WidgetRef ref,
    String sessionId,
  ) async {
    try {
      await ref.read(threadCommandsProvider).deleteThread(sessionId: sessionId);
      ref.invalidate(projectSessionsProvider(projectId));
      if (ref.read(selectedProjectThreadProvider(projectId)) == sessionId) {
        ref
            .read(selectedProjectThreadProvider(projectId).notifier)
            .select(null);
      }
      logFlutterInfo(
        'project_detail',
        'project thread deleted projectId=$projectId sessionId=$sessionId',
      );
      if (!context.mounted) return;
      ShadToaster.maybeOf(context)?.show(const ShadToast(title: Text('会话已删除')));
    } catch (error) {
      if (!context.mounted) return;
      showLoggedErrorToast(
        context,
        target: 'project_detail',
        title: '删除会话失败',
        error: error,
      );
    }
  }
}

// ─────────────────────────── Channel Sidebar ───────────────────────────

class _ChannelSidebar extends ConsumerWidget {
  const _ChannelSidebar({
    required this.projectId,
    required this.projectName,
    required this.threadsAsync,
    required this.selectedThreadId,
    required this.onThreadSelected,
    required this.onNewThread,
    required this.onDeleteThread,
  });

  final String projectId;
  final String projectName;
  final AsyncValue<List<SessionSummary>> threadsAsync;
  final String? selectedThreadId;
  final ValueChanged<String> onThreadSelected;
  final VoidCallback onNewThread;
  final Future<void> Function(String sessionId) onDeleteThread;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final colorScheme = Theme.of(context).colorScheme;

    return SafeArea(
      bottom: false,
      child: Column(
        crossAxisAlignment: .start,
        children: [
          // Project header (Discord-style server name bar)
          Container(
            padding: const .fromLTRB(16, 12, 8, 12),
            decoration: BoxDecoration(
              border: Border(
                bottom: BorderSide(
                  color: colorScheme.outlineVariant.withValues(alpha: 0.3),
                ),
              ),
            ),
            child: Row(
              children: [
                GestureDetector(
                  onTap: () => Navigator.of(context).maybePop(),
                  child: Icon(
                    LucideIcons.chevronLeft,
                    size: 20,
                    color: colorScheme.onSurface,
                  ),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    projectName,
                    style: Theme.of(
                      context,
                    ).textTheme.titleMedium?.copyWith(fontWeight: .bold),
                    maxLines: 1,
                    overflow: .ellipsis,
                  ),
                ),
                IconButton(
                  icon: const Icon(LucideIcons.plus, size: 20),
                  onPressed: onNewThread,
                  tooltip: '新建会话',
                ),
              ],
            ),
          ),

          // Channel category header
          Padding(
            padding: const .fromLTRB(16, 16, 16, 8),
            child: Row(
              children: [
                Icon(LucideIcons.hash, size: 14, color: colorScheme.outline),
                const SizedBox(width: 6),
                Text(
                  '会话',
                  style: Theme.of(context).textTheme.labelSmall?.copyWith(
                    color: colorScheme.outline,
                    fontWeight: .w600,
                    letterSpacing: 0.5,
                  ),
                ),
              ],
            ),
          ),

          // Session list
          Expanded(
            child: threadsAsync.when(
              data: (sessions) => sessions.isEmpty
                  ? _EmptyThreads(onNewThread: onNewThread)
                  : RefreshIndicator(
                      onRefresh: () => ref
                          .read(projectSessionsProvider(projectId).notifier)
                          .refresh(),
                      child: ListView.builder(
                        padding: const .symmetric(horizontal: 8),
                        itemCount: sessions.length,
                        itemBuilder: (context, index) {
                          final thread = sessions[index];
                          final isSelected =
                              thread.sessionId == selectedThreadId;
                          return Dismissible(
                            key: ValueKey(thread.sessionId),
                            direction: DismissDirection.endToStart,
                            confirmDismiss: (_) async {
                              return showDialog<bool>(
                                context: context,
                                builder: (dialogContext) => AlertDialog(
                                  title: const Text('删除会话'),
                                  content: Text(
                                    '确定要删除「${_threadTitle(thread)}」吗？',
                                  ),
                                  actions: <Widget>[
                                    TextButton(
                                      onPressed: () => Navigator.of(
                                        dialogContext,
                                      ).pop(false),
                                      child: const Text('取消'),
                                    ),
                                    TextButton(
                                      onPressed: () =>
                                          Navigator.of(dialogContext).pop(true),
                                      child: const Text('删除'),
                                    ),
                                  ],
                                ),
                              );
                            },
                            onDismissed: (_) => onDeleteThread(thread.sessionId),
                            background: Container(
                              margin: const .symmetric(vertical: 1),
                              padding: const .only(right: 16),
                              alignment: .centerRight,
                              decoration: BoxDecoration(
                                color: Theme.of(
                                  context,
                                ).colorScheme.errorContainer,
                                borderRadius: .circular(6),
                              ),
                              child: Icon(
                                LucideIcons.trash2,
                                size: 16,
                                color: Theme.of(context).colorScheme.error,
                              ),
                            ),
                            child: _ThreadChannelTile(
                              thread: thread,
                              isSelected: isSelected,
                              onTap: () => onThreadSelected(thread.sessionId),
                            ),
                          );
                        },
                      ),
                    ),
              loading: () => const Center(child: CircularProgressIndicator()),
              error: (e, _) => Center(
                child: Padding(
                  padding: const .all(16),
                  child: Text(
                    '加载失败: $e',
                    style: TextStyle(color: colorScheme.error),
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

// ─────────────────────────── Thread Channel Tile ───────────────────────────

class _ThreadChannelTile extends StatelessWidget {
  const _ThreadChannelTile({
    required this.thread,
    required this.isSelected,
    required this.onTap,
  });

  final SessionSummary thread;
  final bool isSelected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final isEnded = thread.endedAtMs != null;

    return Padding(
      padding: const .symmetric(vertical: 1),
      child: Material(
        color: isSelected
            ? colorScheme.primaryContainer.withValues(alpha: 0.4)
            : Colors.transparent,
        borderRadius: .circular(6),
        child: InkWell(
          borderRadius: .circular(6),
          onTap: onTap,
          child: Padding(
            padding: const .symmetric(horizontal: 10, vertical: 8),
            child: Row(
              children: [
                Icon(
                  isEnded ? LucideIcons.messageSquare : LucideIcons.hash,
                  size: 16,
                  color: isSelected ? colorScheme.primary : colorScheme.outline,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    _threadTitle(thread),
                    style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                      fontWeight: isSelected ? .w600 : .normal,
                      color: isEnded
                          ? colorScheme.outline
                          : colorScheme.onSurface,
                    ),
                    maxLines: 1,
                    overflow: .ellipsis,
                  ),
                ),
                if (!isEnded)
                  Container(
                    width: 8,
                    height: 8,
                    decoration: BoxDecoration(
                      color: _agentColor(thread.agent),
                      shape: BoxShape.circle,
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Color _agentColor(AgentName agent) {
    return switch (agent) {
      AgentName.codex => Colors.green,
      AgentName.claude => Colors.orange,
      AgentName.gemini => Colors.blue,
      AgentName.opencode => Colors.blueGrey,
      AgentName.grok => Colors.amber,
    };
  }
}

String _threadTitle(SessionSummary thread) {
  if (thread.title != null && thread.title!.isNotEmpty) {
    return thread.title!;
  }
  final agent = switch (thread.agent) {
    AgentName.codex => 'Codex',
    AgentName.claude => 'Claude',
    AgentName.gemini => 'Gemini',
    AgentName.opencode => 'OpenCode',
    AgentName.grok => 'Grok',
  };
  final time = DateTime.fromMillisecondsSinceEpoch(thread.lastTsMs);
  return '$agent · ${time.month}/${time.day} ${time.hour}:${time.minute.toString().padLeft(2, '0')}';
}

// ─────────────────────────── Empty States ───────────────────────────

class _EmptyThreads extends StatelessWidget {
  const _EmptyThreads({required this.onNewThread});
  final VoidCallback onNewThread;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: .min,
        children: [
          Icon(
            LucideIcons.messageSquarePlus,
            size: 48,
            color: Theme.of(context).colorScheme.outline,
          ),
          const SizedBox(height: 12),
          Text(
            '还没有会话',
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(
              color: Theme.of(context).colorScheme.outline,
            ),
          ),
          const SizedBox(height: 16),
          TextButton.icon(
            onPressed: onNewThread,
            icon: const Icon(LucideIcons.plus, size: 16),
            label: const Text('新建会话'),
          ),
        ],
      ),
    );
  }
}

class _NoThreadSelected extends StatelessWidget {
  const _NoThreadSelected();

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: .min,
        children: [
          Icon(
            LucideIcons.messageSquare,
            size: 48,
            color: Theme.of(context).colorScheme.outline,
          ),
          const SizedBox(height: 12),
          Text(
            '选择一个会话开始',
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(
              color: Theme.of(context).colorScheme.outline,
            ),
          ),
        ],
      ),
    );
  }
}

Color _scaffoldBg(BuildContext context) {
  final brightness = Theme.of(context).brightness;
  return brightness == Brightness.dark
      ? const Color(0xFF1A1A1D)
      : const Color(0xFFF2F3F5);
}
