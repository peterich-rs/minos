import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/presentation/pages/agents_hub_page.dart'
    show AgentEditorSheet;
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/src/rust/api/minos.dart';

class AgentStartPage extends ConsumerStatefulWidget {
  const AgentStartPage({super.key});

  @override
  ConsumerState<AgentStartPage> createState() => _AgentStartPageState();
}

class _AgentStartPageState extends ConsumerState<AgentStartPage> {
  String? _selectedProfileId;
  String _selectedWorkspace = '';

  @override
  Widget build(BuildContext context) {
    final profilesAsync = ref.watch(agentProfilesControllerProvider);
    final hosts =
        ref.watch(pairedMacsProvider).asData?.value ?? const <HostSummaryDto>[];
    final descriptors =
        ref.watch(runtimeAgentDescriptorsProvider).asData?.value ??
        const <AgentDescriptor>[];
    final activeHostId = ref.watch(activeMacProvider).asData?.value;
    final shadTheme = ShadTheme.of(context);

    return Scaffold(
      backgroundColor: shadTheme.colorScheme.background,
      appBar: AppBar(
        title: const Text('开始 Agent 对话'),
        surfaceTintColor: Colors.transparent,
      ),
      body: profilesAsync.when(
        loading: () => const Center(child: ShadProgress()),
        error: (error, _) => _AgentStartError(
          error: error,
          onRetry: () => ref.invalidate(agentProfilesControllerProvider),
        ),
        data: (workspaceState) {
          final orderedProfiles = _orderedProfiles(
            workspaceState.profiles,
            workspaceState.preferredProfileId,
          );
          final selectedProfile = _resolveSelectedProfile(
            orderedProfiles,
            workspaceState.preferredProfileId,
          );
          final effectiveHostId = selectedProfile?.hostDeviceId ?? activeHostId;
          final hostSkillsAsync = effectiveHostId == null
              ? const AsyncValue<List<HostSkillsEntry>>.data(
                  <HostSkillsEntry>[],
                )
              : ref.watch(hostSkillsProvider(effectiveHostId));
          final workspaceOptions = _workspaceOptions(
            hostSkillsAsync.asData?.value ?? const <HostSkillsEntry>[],
          );
          final selectedWorkspace =
              workspaceOptions.contains(_selectedWorkspace)
              ? _selectedWorkspace
              : '';

          return SafeArea(
            bottom: false,
            child: ListView(
              padding: const EdgeInsets.fromLTRB(16, 16, 16, 24),
              children: <Widget>[
                Text(
                  '先选择一个现有 Agent，再决定这次会话使用的工作区。',
                  style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                    color: Theme.of(context).colorScheme.onSurfaceVariant,
                  ),
                ),
                const SizedBox(height: 16),
                _PickerSection(
                  title: 'Agent',
                  trailing: ShadButton.ghost(
                    onPressed: () => _openEditor(
                      context,
                      hosts: hosts,
                      descriptors: descriptors,
                    ),
                    child: const Text('创建 Agent'),
                  ),
                  child: orderedProfiles.isEmpty
                      ? _EmptySelectionState(
                          title: '还没有 Agent',
                          description: '先创建一个 Agent profile，之后这里就可以直接开始新对话。',
                          actionLabel: '创建 Agent',
                          onAction: () => _openEditor(
                            context,
                            hosts: hosts,
                            descriptors: descriptors,
                          ),
                        )
                      : Column(
                          children: <Widget>[
                            for (final profile in orderedProfiles)
                              RadioListTile<String>(
                                value: profile.id,
                                groupValue: selectedProfile?.id,
                                onChanged: (value) {
                                  if (value == null) return;
                                  setState(() {
                                    _selectedProfileId = value;
                                    _selectedWorkspace = '';
                                  });
                                },
                                title: Text(profile.name),
                                subtitle: Text(
                                  _profileSubtitle(profile, hosts),
                                  maxLines: 2,
                                  overflow: TextOverflow.ellipsis,
                                ),
                                contentPadding: const EdgeInsets.symmetric(
                                  horizontal: 12,
                                ),
                              ),
                          ],
                        ),
                ),
                const SizedBox(height: 16),
                _PickerSection(
                  title: '工作区',
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: <Widget>[
                      ListTile(
                        contentPadding: const EdgeInsets.symmetric(
                          horizontal: 12,
                          vertical: 4,
                        ),
                        title: Text(_workspaceTitle(selectedWorkspace)),
                        subtitle: Text(
                          selectedProfile == null
                              ? '先选择 Agent 后再挑工作区。'
                              : _workspaceSubtitle(
                                  selectedWorkspace,
                                  effectiveHostId: effectiveHostId,
                                  hosts: hosts,
                                ),
                        ),
                        trailing: const Icon(Icons.chevron_right),
                        onTap: selectedProfile == null
                            ? null
                            : () async {
                                final picked = await _showWorkspacePicker(
                                  context,
                                  options: workspaceOptions,
                                  currentValue: selectedWorkspace,
                                );
                                if (picked == null || !mounted) return;
                                setState(() => _selectedWorkspace = picked);
                              },
                      ),
                      if (hostSkillsAsync.isLoading)
                        const Padding(
                          padding: EdgeInsets.fromLTRB(16, 0, 16, 12),
                          child: LinearProgressIndicator(minHeight: 2),
                        )
                      else if (hostSkillsAsync.hasError)
                        Padding(
                          padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
                          child: Text(
                            '工作区列表暂时不可用，将继续使用默认工作区。',
                            style: Theme.of(context).textTheme.bodySmall
                                ?.copyWith(
                                  color: Theme.of(
                                    context,
                                  ).colorScheme.onSurfaceVariant,
                                ),
                          ),
                        )
                      else
                        Padding(
                          padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
                          child: Text(
                            '默认工作区会保持当前行为；其它选项来自 runtime 返回的技能扫描目录。',
                            style: Theme.of(context).textTheme.bodySmall
                                ?.copyWith(
                                  color: Theme.of(
                                    context,
                                  ).colorScheme.onSurfaceVariant,
                                ),
                          ),
                        ),
                    ],
                  ),
                ),
                const SizedBox(height: 20),
                ShadButton(
                  onPressed: selectedProfile == null
                      ? null
                      : () {
                          Navigator.of(context).push(
                            MaterialPageRoute<void>(
                              builder: (_) => ThreadViewPage(
                                agentProfileId: selectedProfile.id,
                                startupWorkspace: selectedWorkspace.isEmpty
                                    ? null
                                    : selectedWorkspace,
                              ),
                            ),
                          );
                        },
                  child: const Text('继续'),
                ),
              ],
            ),
          );
        },
      ),
    );
  }

  AgentProfile? _resolveSelectedProfile(
    List<AgentProfile> orderedProfiles,
    String? preferredProfileId,
  ) {
    if (orderedProfiles.isEmpty) {
      return null;
    }
    for (final profile in orderedProfiles) {
      if (profile.id == _selectedProfileId) {
        return profile;
      }
    }
    if (preferredProfileId != null) {
      for (final profile in orderedProfiles) {
        if (profile.id == preferredProfileId) {
          return profile;
        }
      }
    }
    return orderedProfiles.first;
  }

  Future<void> _openEditor(
    BuildContext context, {
    required List<HostSummaryDto> hosts,
    required List<AgentDescriptor> descriptors,
  }) {
    return showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      backgroundColor: Theme.of(context).colorScheme.surface,
      builder: (_) => AgentEditorSheet(hosts: hosts, descriptors: descriptors),
    );
  }
}

class _AgentStartError extends StatelessWidget {
  const _AgentStartError({required this.error, required this.onRetry});

  final Object error;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Text('加载 Agent 失败: $error', textAlign: TextAlign.center),
            const SizedBox(height: 12),
            ShadButton(onPressed: onRetry, child: const Text('重试')),
          ],
        ),
      ),
    );
  }
}

class _PickerSection extends StatelessWidget {
  const _PickerSection({
    required this.title,
    required this.child,
    this.trailing,
  });

  final String title;
  final Widget child;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Row(
          children: <Widget>[
            Expanded(
              child: Text(
                title,
                style: Theme.of(
                  context,
                ).textTheme.titleSmall?.copyWith(fontWeight: FontWeight.w700),
              ),
            ),
            if (trailing != null) trailing!,
          ],
        ),
        const SizedBox(height: 8),
        DecoratedBox(
          decoration: BoxDecoration(
            color: Theme.of(context).colorScheme.surfaceContainerLow,
            borderRadius: BorderRadius.circular(16),
          ),
          child: child,
        ),
      ],
    );
  }
}

class _EmptySelectionState extends StatelessWidget {
  const _EmptySelectionState({
    required this.title,
    required this.description,
    required this.actionLabel,
    required this.onAction,
  });

  final String title;
  final String description;
  final String actionLabel;
  final VoidCallback onAction;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Text(
            title,
            style: Theme.of(
              context,
            ).textTheme.titleMedium?.copyWith(fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 6),
          Text(
            description,
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: 12),
          ShadButton(onPressed: onAction, child: Text(actionLabel)),
        ],
      ),
    );
  }
}

List<AgentProfile> _orderedProfiles(
  List<AgentProfile> profiles,
  String? preferredProfileId,
) {
  final ordered = [...profiles]
    ..sort((left, right) {
      if (left.id == preferredProfileId) return -1;
      if (right.id == preferredProfileId) return 1;
      return right.updatedAtMs.compareTo(left.updatedAtMs);
    });
  return ordered;
}

String _profileSubtitle(AgentProfile profile, List<HostSummaryDto> hosts) {
  final hostLabel =
      _hostLabelForId(hosts, profile.hostDeviceId) ??
      profile.hostDisplayName ??
      '跟随当前 runtime';
  final detail = profile.description.trim().isEmpty
      ? '${_runtimeLabel(profile.runtimeAgent)} · ${profile.model}'
      : profile.description.trim();
  return '$hostLabel · $detail';
}

Future<String?> _showWorkspacePicker(
  BuildContext context, {
  required List<String> options,
  required String currentValue,
}) {
  return showModalBottomSheet<String>(
    context: context,
    useSafeArea: true,
    showDragHandle: true,
    backgroundColor: Theme.of(context).colorScheme.surface,
    builder: (sheetContext) {
      return Padding(
        padding: const EdgeInsets.fromLTRB(16, 4, 16, 24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Text(
              '选择工作区',
              style: Theme.of(
                sheetContext,
              ).textTheme.titleMedium?.copyWith(fontWeight: FontWeight.w800),
            ),
            const SizedBox(height: 12),
            Flexible(
              child: ListView.separated(
                shrinkWrap: true,
                itemCount: options.length,
                separatorBuilder: (_, _) => const Divider(height: 1),
                itemBuilder: (_, index) {
                  final option = options[index];
                  final selected = option == currentValue;
                  return ListTile(
                    contentPadding: const EdgeInsets.symmetric(
                      horizontal: 4,
                      vertical: 2,
                    ),
                    title: Text(_workspaceTitle(option)),
                    subtitle: Text(option.isEmpty ? '保持当前默认工作区行为' : option),
                    trailing: selected ? const Icon(Icons.check) : null,
                    onTap: () => Navigator.of(sheetContext).pop(option),
                  );
                },
              ),
            ),
          ],
        ),
      );
    },
  );
}

List<String> _workspaceOptions(List<HostSkillsEntry> entries) {
  final options = <String>{''};
  for (final entry in entries) {
    final cwd = entry.cwd.trim();
    if (cwd.isNotEmpty) {
      options.add(cwd);
    }
  }
  final sorted = options.toList(growable: false)..sort();
  if (sorted.isNotEmpty && sorted.first != '') {
    return <String>['', ...sorted.where((value) => value.isNotEmpty)];
  }
  return sorted;
}

String _workspaceTitle(String workspace) {
  if (workspace.isEmpty) {
    return '默认工作区';
  }
  final segments = workspace.split('/');
  return segments.isEmpty ? workspace : segments.last;
}

String _workspaceSubtitle(
  String workspace, {
  required String? effectiveHostId,
  required List<HostSummaryDto> hosts,
}) {
  final hostLabel = _hostLabelForId(hosts, effectiveHostId) ?? '当前 runtime';
  if (workspace.isEmpty) {
    return '$hostLabel · 保持当前默认工作区行为';
  }
  return '$hostLabel · $workspace';
}

String? _hostLabelForId(List<HostSummaryDto> hosts, String? hostDeviceId) {
  if (hostDeviceId == null) {
    return null;
  }
  for (final host in hosts) {
    if (host.hostDeviceId != hostDeviceId) {
      continue;
    }
    final displayName = host.hostDisplayName.trim();
    return displayName.isEmpty ? host.hostDeviceId : displayName;
  }
  return null;
}

String _runtimeLabel(AgentName agent) {
  return switch (agent) {
    AgentName.codex => 'Codex',
    AgentName.claude => 'Claude',
    AgentName.gemini => 'Gemini',
  };
}
