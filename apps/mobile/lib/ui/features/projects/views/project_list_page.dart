import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/project_providers.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/features/shell/router.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

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
    unawaited(
      showDialog<void>(
        context: context,
        builder: (_) => const _CreateProjectDialog(),
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

class _CreateProjectDialog extends ConsumerStatefulWidget {
  const _CreateProjectDialog();

  @override
  ConsumerState<_CreateProjectDialog> createState() =>
      _CreateProjectDialogState();
}

class _CreateProjectDialogState extends ConsumerState<_CreateProjectDialog> {
  final TextEditingController _nameController = TextEditingController();
  final TextEditingController _slugController = TextEditingController();
  final TextEditingController _workspacePathController =
      TextEditingController();

  bool _slugEdited = false;
  bool _creating = false;

  @override
  void initState() {
    super.initState();
    _nameController.addListener(_refresh);
    _slugController.addListener(_refresh);
  }

  @override
  void dispose() {
    _nameController.dispose();
    _slugController.dispose();
    _workspacePathController.dispose();
    super.dispose();
  }

  void _refresh() {
    if (mounted) {
      setState(() {});
    }
  }

  @override
  Widget build(BuildContext context) {
    final activeHostId = ref.watch(activeMacProvider).asData?.value;
    final workspacesAsync = ref.watch(hostWorkspacesProvider(activeHostId));
    final canCreate =
        _nameController.text.trim().isNotEmpty &&
        _slugController.text.trim().isNotEmpty &&
        !_creating;

    return AlertDialog(
      title: const Text('新建项目'),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextField(
                controller: _nameController,
                decoration: const InputDecoration(
                  labelText: '项目名称',
                  hintText: '例如: My App',
                ),
                autofocus: true,
                onChanged: _onNameChanged,
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _slugController,
                decoration: const InputDecoration(
                  labelText: 'Workspace 标识',
                  hintText: '例如: my-app',
                ),
                onChanged: (_) => _slugEdited = true,
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _workspacePathController,
                decoration: const InputDecoration(
                  labelText: 'Host 工作区路径',
                  hintText: '/Users/you/develop/my-app',
                ),
              ),
              const SizedBox(height: 12),
              _WorkspacePickerList(
                workspacesAsync: workspacesAsync,
                onPick: _pickWorkspace,
                onRefresh: () =>
                    ref.invalidate(hostWorkspacesProvider(activeHostId)),
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: _creating ? null : () => Navigator.pop(context),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: canCreate ? _create : null,
          child: _creating
              ? const SizedBox.square(
                  dimension: 18,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Text('创建'),
        ),
      ],
    );
  }

  void _onNameChanged(String value) {
    if (!_slugEdited) {
      _slugController.text = ProjectListPage._slugify(value);
    }
  }

  void _pickWorkspace(HostWorkspaceSummary workspace) {
    _workspacePathController.text = workspace.path;
    if (_nameController.text.trim().isEmpty) {
      _nameController.text = workspace.displayName;
    }
    if (!_slugEdited || _slugController.text.trim().isEmpty) {
      _slugController.text = ProjectListPage._slugify(workspace.displayName);
      _slugEdited = false;
    }
  }

  Future<void> _create() async {
    final name = _nameController.text.trim();
    final slug = _slugController.text.trim();
    final workspacePath = _workspacePathController.text.trim();
    if (name.isEmpty || slug.isEmpty) {
      return;
    }
    setState(() => _creating = true);
    try {
      await ref
          .read(projectListProvider.notifier)
          .createProject(
            name: name,
            workspaceSlug: slug,
            workspacePath: workspacePath.isEmpty ? null : workspacePath,
          );
      if (mounted) {
        Navigator.pop(context);
      }
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text('创建失败: $error')));
        setState(() => _creating = false);
      }
    }
  }
}

class _WorkspacePickerList extends StatelessWidget {
  const _WorkspacePickerList({
    required this.workspacesAsync,
    required this.onPick,
    required this.onRefresh,
  });

  final AsyncValue<ListHostWorkspacesResponse> workspacesAsync;
  final ValueChanged<HostWorkspaceSummary> onPick;
  final VoidCallback onRefresh;

  @override
  Widget build(BuildContext context) {
    return workspacesAsync.when(
      loading: () => const LinearProgressIndicator(minHeight: 2),
      error: (error, _) => Align(
        alignment: Alignment.centerLeft,
        child: TextButton.icon(
          onPressed: onRefresh,
          icon: const Icon(LucideIcons.refreshCw, size: 16),
          label: const Text('重新加载 Host 文件夹'),
        ),
      ),
      data: (response) {
        if (response.workspaces.isEmpty) {
          return Align(
            alignment: Alignment.centerLeft,
            child: TextButton.icon(
              onPressed: onRefresh,
              icon: const Icon(LucideIcons.folderOpen, size: 16),
              label: const Text('Host 文件夹为空'),
            ),
          );
        }
        return ConstrainedBox(
          constraints: const BoxConstraints(maxHeight: 220),
          child: ListView.separated(
            shrinkWrap: true,
            itemCount: response.workspaces.length,
            separatorBuilder: (_, _) => const Divider(height: 1),
            itemBuilder: (context, index) {
              final workspace = response.workspaces[index];
              return ListTile(
                dense: true,
                contentPadding: EdgeInsets.zero,
                leading: Icon(
                  workspace.isGitRepo
                      ? LucideIcons.gitBranch
                      : LucideIcons.folder,
                  size: 18,
                ),
                title: Text(
                  workspace.displayName,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                subtitle: Text(
                  workspace.path,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                onTap: () => onPick(workspace),
              );
            },
          ),
        );
      },
    );
  }
}

// ─────────────────────────── Header ───────────────────────────

class _Header extends StatelessWidget {
  const _Header({required this.onAdd});
  final VoidCallback onAdd;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const .symmetric(horizontal: 20, vertical: 12),
      child: Row(
        children: [
          Text(
            '项目',
            style: Theme.of(
              context,
            ).textTheme.headlineMedium?.copyWith(fontWeight: .bold),
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
        mainAxisSize: .min,
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
        padding: const .symmetric(horizontal: 16, vertical: 8),
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
    final workspaceLabel = _projectWorkspaceLabel(project);

    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: .circular(12),
        side: BorderSide(
          color: colorScheme.outlineVariant.withValues(alpha: 0.3),
        ),
      ),
      child: InkWell(
        borderRadius: .circular(12),
        onTap: () {
          ref.read(selectedProjectProvider.notifier).select(project.projectId);
          unawaited(
            context.push(
              '/project/${project.projectId}',
              extra: ProjectDetailRouteExtra(projectName: project.name),
            ),
          );
        },
        onLongPress: () => _showProjectMenu(context, ref),
        child: Padding(
          padding: const .all(16),
          child: Row(
            children: [
              Container(
                width: 48,
                height: 48,
                decoration: BoxDecoration(
                  color: _projectColor(
                    project.projectId,
                  ).withValues(alpha: 0.15),
                  borderRadius: .circular(12),
                ),
                child: Center(
                  child: Text(
                    project.name.isNotEmpty
                        ? project.name[0].toUpperCase()
                        : '?',
                    style: TextStyle(
                      fontSize: 20,
                      fontWeight: .bold,
                      color: _projectColor(project.projectId),
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 14),
              Expanded(
                child: Column(
                  crossAxisAlignment: .start,
                  children: [
                    Text(
                      project.name,
                      style: Theme.of(
                        context,
                      ).textTheme.titleSmall?.copyWith(fontWeight: .w600),
                      maxLines: 1,
                      overflow: .ellipsis,
                    ),
                    const SizedBox(height: 4),
                    Text(
                      '${project.threadCount} 个会话 · $workspaceLabel',
                      style: Theme.of(context).textTheme.bodySmall?.copyWith(
                        color: colorScheme.outline,
                      ),
                      maxLines: 1,
                      overflow: .ellipsis,
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
    unawaited(
      showModalBottomSheet<void>(
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
      ),
    );
  }

  void _showRenameDialog(BuildContext context, WidgetRef ref) {
    final controller = TextEditingController(text: project.name);
    unawaited(
      showDialog<void>(
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
      ),
    );
  }

  void _confirmDelete(BuildContext context, WidgetRef ref) {
    unawaited(
      showDialog<void>(
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

String _projectWorkspaceLabel(ProjectSummary project) {
  final path = project.workspacePath?.trim();
  return path == null || path.isEmpty ? project.workspaceSlug : path;
}

Color _scaffoldBg(BuildContext context) {
  final brightness = Theme.of(context).brightness;
  return brightness == Brightness.dark
      ? const Color(0xFF0F0F0F)
      : const Color(0xFFF8F8F8);
}
