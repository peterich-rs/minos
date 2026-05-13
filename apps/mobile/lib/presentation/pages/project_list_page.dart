import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/project_providers.dart';
import 'package:minos/presentation/router.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Full-screen project list — the app's home after login.
/// Tapping a project navigates to [ProjectDetailPage] (Discord-style).
/// Swipe-back from the detail page returns here.
class ProjectListPage extends ConsumerWidget {
  const ProjectListPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final projectsAsync = ref.watch(projectListProvider);

    return Scaffold(
      backgroundColor: _scaffoldBg(context),
      body: SafeArea(
        bottom: false,
        child: Column(
          children: [
            _Header(onAdd: () => _showCreateDialog(context, ref)),
            Expanded(
              child: projectsAsync.when(
                data: (projects) => projects.isEmpty
                    ? _EmptyState(onAdd: () => _showCreateDialog(context, ref))
                    : _ProjectGrid(projects: projects),
                loading: () => const Center(child: CircularProgressIndicator()),
                error: (e, _) => Center(
                  child: Text(
                    '加载失败: $e',
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  void _showCreateDialog(BuildContext context, WidgetRef ref) {
    final nameController = TextEditingController();
    final slugController = TextEditingController();

    showDialog(
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
                // Auto-generate slug from name
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
                helperText: '存储在 .minos/workspaces/ 下',
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
    );
  }

  static String _slugify(String input) {
    return input
        .toLowerCase()
        .replaceAll(RegExp(r'[^a-z0-9\u4e00-\u9fff]+'), '-')
        .replaceAll(RegExp(r'^-+|-+$'), '');
  }
}

// ─────────────────────────── Header ───────────────────────────

class _Header extends StatelessWidget {
  const _Header({required this.onAdd});
  final VoidCallback onAdd;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
      child: Row(
        children: [
          Text(
            '项目',
            style: Theme.of(
              context,
            ).textTheme.headlineMedium?.copyWith(fontWeight: FontWeight.bold),
          ),
          const Spacer(),
          IconButton(
            icon: const Icon(LucideIcons.plus, size: 22),
            onPressed: onAdd,
          ),
        ],
      ),
    );
  }
}

// ─────────────────────────── Empty State ───────────────────────────

class _EmptyState extends StatelessWidget {
  const _EmptyState({required this.onAdd});
  final VoidCallback onAdd;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            LucideIcons.folderOpen,
            size: 64,
            color: Theme.of(context).colorScheme.outline,
          ),
          const SizedBox(height: 16),
          Text(
            '还没有项目',
            style: Theme.of(context).textTheme.titleMedium?.copyWith(
              color: Theme.of(context).colorScheme.outline,
            ),
          ),
          const SizedBox(height: 8),
          Text(
            '创建一个项目来开始使用 AI 助手',
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(
              color: Theme.of(context).colorScheme.outline,
            ),
          ),
          const SizedBox(height: 24),
          FilledButton.icon(
            onPressed: onAdd,
            icon: const Icon(LucideIcons.plus, size: 16),
            label: const Text('新建项目'),
          ),
        ],
      ),
    );
  }
}

// ─────────────────────────── Project Grid ───────────────────────────

class _ProjectGrid extends ConsumerWidget {
  const _ProjectGrid({required this.projects});
  final List<ProjectSummary> projects;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return RefreshIndicator(
      onRefresh: () => ref.read(projectListProvider.notifier).refresh(),
      child: ListView.separated(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
        itemCount: projects.length,
        separatorBuilder: (_, _) => const SizedBox(height: 8),
        itemBuilder: (context, index) {
          final project = projects[index];
          return _ProjectCard(project: project);
        },
      ),
    );
  }
}

class _ProjectCard extends ConsumerWidget {
  const _ProjectCard({required this.project});
  final ProjectSummary project;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final colorScheme = Theme.of(context).colorScheme;

    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: BorderSide(color: colorScheme.outlineVariant.withOpacity(0.3)),
      ),
      child: InkWell(
        borderRadius: BorderRadius.circular(12),
        onTap: () {
          ref.read(selectedProjectProvider.notifier).select(project.projectId);
          context.push(
            '/project/${project.projectId}',
            extra: ProjectDetailRouteExtra(projectName: project.name),
          );
        },
        onLongPress: () => _showProjectMenu(context, ref),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Row(
            children: [
              Container(
                width: 48,
                height: 48,
                decoration: BoxDecoration(
                  color: _projectColor(project.projectId).withOpacity(0.15),
                  borderRadius: BorderRadius.circular(12),
                ),
                child: Center(
                  child: Text(
                    project.name.isNotEmpty
                        ? project.name[0].toUpperCase()
                        : '?',
                    style: TextStyle(
                      fontSize: 20,
                      fontWeight: FontWeight.bold,
                      color: _projectColor(project.projectId),
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 14),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      project.name,
                      style: Theme.of(context).textTheme.titleSmall?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    const SizedBox(height: 4),
                    Text(
                      '${project.threadCount} 个会话 · ${project.workspaceSlug}',
                      style: Theme.of(context).textTheme.bodySmall?.copyWith(
                        color: colorScheme.outline,
                      ),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ],
                ),
              ),
              Icon(
                LucideIcons.chevronRight,
                size: 18,
                color: colorScheme.outline,
              ),
            ],
          ),
        ),
      ),
    );
  }

  void _showProjectMenu(BuildContext context, WidgetRef ref) {
    showModalBottomSheet(
      context: context,
      builder: (ctx) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: const Icon(LucideIcons.pencil),
              title: const Text('重命名'),
              onTap: () {
                Navigator.pop(ctx);
                _showRenameDialog(context, ref);
              },
            ),
            ListTile(
              leading: Icon(
                LucideIcons.trash2,
                color: Theme.of(context).colorScheme.error,
              ),
              title: Text(
                '删除',
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
              onTap: () {
                Navigator.pop(ctx);
                _confirmDelete(context, ref);
              },
            ),
          ],
        ),
      ),
    );
  }

  void _showRenameDialog(BuildContext context, WidgetRef ref) {
    final controller = TextEditingController(text: project.name);
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('重命名项目'),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: const InputDecoration(labelText: '项目名称'),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () async {
              final name = controller.text.trim();
              if (name.isEmpty) return;
              Navigator.pop(ctx);
              await ref
                  .read(projectListProvider.notifier)
                  .updateProject(projectId: project.projectId, name: name);
            },
            child: const Text('确定'),
          ),
        ],
      ),
    );
  }

  void _confirmDelete(BuildContext context, WidgetRef ref) {
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('删除项目'),
        content: Text('确定要删除「${project.name}」吗？项目内的会话不会被删除。'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('取消'),
          ),
          FilledButton(
            style: FilledButton.styleFrom(
              backgroundColor: Theme.of(context).colorScheme.error,
            ),
            onPressed: () async {
              Navigator.pop(ctx);
              await ref
                  .read(projectListProvider.notifier)
                  .deleteProject(project.projectId);
            },
            child: const Text('删除'),
          ),
        ],
      ),
    );
  }

  Color _projectColor(String id) {
    final colors = [
      Colors.blue,
      Colors.purple,
      Colors.teal,
      Colors.orange,
      Colors.pink,
      Colors.indigo,
      Colors.green,
      Colors.amber,
    ];
    return colors[id.hashCode.abs() % colors.length];
  }
}

Color _scaffoldBg(BuildContext context) {
  final brightness = Theme.of(context).brightness;
  return brightness == Brightness.dark
      ? const Color(0xFF0F0F0F)
      : const Color(0xFFF8F8F8);
}
