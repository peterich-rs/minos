import 'package:flutter/cupertino.dart' hide ConnectionState;
import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/auth_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/presentation/pages/pairing_page.dart';
import 'package:minos/presentation/pages/thread_view_page.dart';
import 'package:minos/presentation/widgets/shimmer_box.dart';
import 'package:minos/src/rust/api/minos.dart'
    as minos_api
    show ConnectionState;
import 'package:minos/src/rust/api/minos.dart' hide ConnectionState;

class AgentsHubTab extends ConsumerWidget {
  const AgentsHubTab({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profilesAsync = ref.watch(agentProfilesControllerProvider);
    final preferredProfile = ref.watch(preferredAgentProfileProvider);
    final runtimeDescriptors = ref.watch(runtimeAgentDescriptorsProvider);
    final pairedHosts = ref.watch(pairedMacsProvider);
    final hosts = pairedHosts.asData?.value ?? const <HostSummaryDto>[];
    final activeHostId = ref.watch(activeMacProvider).asData?.value;
    final connection = ref.watch(connectionStateProvider).asData?.value;
    final authState = ref.watch(authControllerProvider);

    return SafeArea(
      bottom: false,
      child: RefreshIndicator(
        onRefresh: () async {
          try {
            await ref.read(pairedMacsProvider.notifier).refresh();
            ref.invalidate(runtimeAgentDescriptorsProvider);
            await ref.read(runtimeAgentDescriptorsProvider.future);
          } catch (error) {
            if (context.mounted) {
              _showRefreshError(context, '成员刷新失败', error);
            }
          }
        },
        child: ListView(
          padding: const EdgeInsets.only(top: 12, bottom: 28),
          children: <Widget>[
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 4),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    'Members',
                    style: Theme.of(context).textTheme.headlineLarge?.copyWith(
                      fontWeight: FontWeight.w800,
                      letterSpacing: 0,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    preferredProfile == null
                        ? '像聊天成员一样管理设备、Agent 和你自己。'
                        : '默认 Agent：${preferredProfile.name}',
                    style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                      color: Theme.of(context).colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 16),
            _HostRuntimeCard(
              pairedHosts: pairedHosts,
              activeHostId: activeHostId,
              connection: connection,
            ),
            const SizedBox(height: 16),
            profilesAsync.when(
              loading: () => _SectionCard(
                title: 'AGENTS 0',
                trailing: _SectionActionButton(
                  tooltip: '新建 Agent',
                  onPressed: () => _openEditor(
                    context,
                    ref,
                    hosts: hosts,
                    descriptors:
                        runtimeDescriptors.asData?.value ??
                        const <AgentDescriptor>[],
                  ),
                ),
                child: const _AgentListSkeleton(),
              ),
              error: (error, _) => _SectionCard(
                title: 'AGENTS 0',
                trailing: _SectionActionButton(
                  tooltip: '新建 Agent',
                  onPressed: () => _openEditor(
                    context,
                    ref,
                    hosts: hosts,
                    descriptors:
                        runtimeDescriptors.asData?.value ??
                        const <AgentDescriptor>[],
                  ),
                ),
                child: Padding(
                  padding: const EdgeInsets.all(18),
                  child: Text('加载 Agent 失败: $error'),
                ),
              ),
              data: (state) => _ProfilesSection(
                state: state,
                preferredProfileId: preferredProfile?.id,
                hosts: hosts,
                onAdd: () => _openEditor(
                  context,
                  ref,
                  hosts: hosts,
                  descriptors:
                      runtimeDescriptors.asData?.value ??
                      const <AgentDescriptor>[],
                ),
              ),
            ),
            const SizedBox(height: 16),
            _HumansSection(authState: authState),
          ],
        ),
      ),
    );
  }

  Future<void> _openEditor(
    BuildContext context,
    WidgetRef ref, {
    required List<HostSummaryDto> hosts,
    required List<AgentDescriptor> descriptors,
    AgentProfile? profile,
  }) {
    return showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      backgroundColor: Theme.of(context).colorScheme.surface,
      builder: (_) => AgentEditorSheet(
        profile: profile,
        hosts: hosts,
        descriptors: descriptors,
      ),
    );
  }
}

class _SectionActionButton extends StatelessWidget {
  const _SectionActionButton({required this.tooltip, required this.onPressed});

  final String tooltip;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return ShadIconButton.ghost(
      icon: const Icon(LucideIcons.plus, size: 18),
      onPressed: onPressed,
      width: 38,
      height: 38,
    );
  }
}

class _AgentListSkeleton extends StatelessWidget {
  const _AgentListSkeleton();

  @override
  Widget build(BuildContext context) {
    return Column(
      children: List.generate(
        2,
        (index) => Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const ShimmerBox(width: 42, height: 42, circular: true),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    ShimmerBox(width: 80, height: 12),
                    SizedBox(height: 8),
                    ShimmerBox(width: 120, height: 16),
                    SizedBox(height: 8),
                    ShimmerBox(width: double.infinity, height: 14),
                    SizedBox(height: 4),
                    ShimmerBox(width: 200, height: 14),
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

class _DeviceListSkeleton extends StatelessWidget {
  const _DeviceListSkeleton();

  @override
  Widget build(BuildContext context) {
    return Column(
      children: List.generate(
        1,
        (index) => Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
          child: Row(
            children: [
              const ShimmerBox(width: 42, height: 42, circular: true),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    ShimmerBox(width: 100, height: 16),
                    SizedBox(height: 8),
                    ShimmerBox(width: 140, height: 14),
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

class _HumanListSkeleton extends StatelessWidget {
  const _HumanListSkeleton();

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      child: Row(
        children: [
          const ShimmerBox(width: 42, height: 42, circular: true),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: const [
                ShimmerBox(width: 120, height: 16),
                SizedBox(height: 8),
                ShimmerBox(width: 180, height: 14),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _CompactErrorPanel extends StatelessWidget {
  const _CompactErrorPanel({
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
    final shadTheme = ShadTheme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Row(
          children: <Widget>[
            Icon(
              LucideIcons.circleAlert,
              size: 18,
              color: shadTheme.colorScheme.mutedForeground,
            ),
            const SizedBox(width: 8),
            Expanded(child: Text(title, style: shadTheme.textTheme.small)),
          ],
        ),
        const SizedBox(height: 6),
        Text(
          description,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          style: shadTheme.textTheme.muted,
        ),
        const SizedBox(height: 10),
        ShadButton.outline(onPressed: onAction, child: Text(actionLabel)),
      ],
    );
  }
}

class _MemberStatusDot extends StatelessWidget {
  const _MemberStatusDot({required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      width: 10,
      height: 10,
      decoration: BoxDecoration(
        color: color,
        shape: BoxShape.circle,
        border: Border.all(color: theme.colorScheme.surface, width: 1.5),
        boxShadow: [
          BoxShadow(color: color.withValues(alpha: 0.4), blurRadius: 4),
        ],
      ),
    );
  }
}

class _DeviceAvatar extends StatelessWidget {
  const _DeviceAvatar();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    return Container(
      width: 42,
      height: 42,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(14),
        color: isDark ? const Color(0xFF1E3A8A) : const Color(0xFFDBEAFE),
      ),
      alignment: Alignment.center,
      child: Icon(
        CupertinoIcons.desktopcomputer,
        color: isDark ? const Color(0xFF60A5FA) : const Color(0xFF2563EB),
        size: 20,
      ),
    );
  }
}

class _HumanAvatar extends StatelessWidget {
  const _HumanAvatar({required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    return Container(
      width: 42,
      height: 42,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(14),
        color: isDark ? const Color(0xFF4C1D95) : const Color(0xFFEDE9FE),
      ),
      alignment: Alignment.center,
      child: Text(
        label,
        style: theme.textTheme.titleMedium?.copyWith(
          fontWeight: FontWeight.w800,
          color: isDark ? const Color(0xFFA78BFA) : const Color(0xFF7C3AED),
        ),
      ),
    );
  }
}

String _resolvedProfileHostLabel(
  AgentProfile profile,
  List<HostSummaryDto> hosts,
) {
  final resolved = _hostLabelForId(hosts, profile.hostDeviceId);
  if (resolved != null && resolved.trim().isNotEmpty) return resolved;
  final fallback = profile.hostDisplayName?.trim();
  if (fallback != null && fallback.isNotEmpty) return fallback;
  return '未绑定 runtime';
}

Color _connectionColor(
  minos_api.ConnectionState? state,
  bool isActive,
  bool isDark,
) {
  if (!isActive) {
    return isDark ? const Color(0xFF3F3F46) : const Color(0xFFD4D4D8);
  }
  return switch (state) {
    ConnectionState_Connected() =>
      isDark ? const Color(0xFF22C55E) : const Color(0xFF16A34A),
    ConnectionState_Reconnecting() || ConnectionState_Pairing() =>
      isDark ? const Color(0xFFEAB308) : const Color(0xFFCA8A04),
    _ => isDark ? const Color(0xFF52525B) : const Color(0xFFA1A1AA),
  };
}

String _connectionLabel(minos_api.ConnectionState? state) {
  return switch (state) {
    ConnectionState_Connected() => '在线',
    ConnectionState_Reconnecting() => '重连中',
    ConnectionState_Pairing() => '配对中',
    _ => '离线',
  };
}

String _humanDisplayName(String email) {
  final trimmed = email.trim();
  if (trimmed.isEmpty) return 'You';
  final at = trimmed.indexOf('@');
  return at <= 0 ? trimmed : trimmed.substring(0, at);
}

void _showRefreshError(BuildContext context, String title, Object error) {
  ShadToaster.maybeOf(context)?.show(
    ShadToast.destructive(
      title: Text(title),
      description: Text(error.toString()),
    ),
  );
}

class AgentProfilePage extends ConsumerWidget {
  const AgentProfilePage({super.key, required this.profileId});

  final String profileId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final workspace = ref.watch(agentProfilesControllerProvider).asData?.value;
    final profile = workspace?.profileById(profileId);
    final preferredId = workspace?.preferredProfileId;
    final hosts =
        ref.watch(pairedMacsProvider).asData?.value ?? const <HostSummaryDto>[];
    final activeHostId = ref.watch(activeMacProvider).asData?.value;
    final effectiveHostId = profile?.hostDeviceId ?? activeHostId;
    final effectiveHostLabel =
        profile?.hostDisplayName ??
        _hostLabelForId(hosts, effectiveHostId) ??
        '未选择 runtime';
    final descriptors =
        ref.watch(runtimeAgentDescriptorsProvider).asData?.value ??
        const <AgentDescriptor>[];
    if (profile == null) {
      return Scaffold(
        appBar: AppBar(title: const Text('Agent')),
        body: const Center(child: Text('Agent 不存在或已被删除。')),
      );
    }

    return Scaffold(
      appBar: AppBar(
        title: Text(profile.name),
        actions: <Widget>[
          IconButton(
            tooltip: '编辑',
            onPressed: () => showModalBottomSheet<void>(
              context: context,
              isScrollControlled: true,
              useSafeArea: true,
              backgroundColor: Theme.of(context).colorScheme.surface,
              builder: (_) => AgentEditorSheet(
                profile: profile,
                hosts: hosts,
                descriptors: descriptors,
              ),
            ),
            icon: const Icon(CupertinoIcons.pencil),
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.only(top: 12, bottom: 28),
        children: <Widget>[
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: _ProfileHero(
              profile: profile,
              isPreferred: preferredId == profile.id,
            ),
          ),
          const SizedBox(height: 16),
          _SectionCard(
            title: 'Profile',
            child: Column(
              children: <Widget>[
                _DetailRow(label: 'Display Name', value: profile.name),
                const Divider(height: 1),
                _DetailRow(
                  label: 'Description',
                  value: profile.description.isEmpty
                      ? 'No description'
                      : profile.description,
                ),
                const Divider(height: 1),
                _DetailRow(
                  label: 'Computer',
                  value: profile.hostDisplayName ?? '跟随当前 runtime',
                ),
              ],
            ),
          ),
          const SizedBox(height: 16),
          _SectionCard(
            title: 'Runtime Configuration',
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Wrap(
                spacing: 10,
                runSpacing: 10,
                children: <Widget>[
                  _BadgeChip(label: _runtimeLabel(profile.runtimeAgent)),
                  _BadgeChip(label: profile.model),
                  _BadgeChip(label: _reasoningLabel(profile.reasoningEffort)),
                ],
              ),
            ),
          ),
          const SizedBox(height: 16),
          _SectionCard(
            title: 'Environment Variables',
            child: profile.environmentVariables.isEmpty
                ? const Padding(
                    padding: EdgeInsets.all(16),
                    child: Text('No environment variables configured.'),
                  )
                : Column(
                    children: <Widget>[
                      for (
                        var i = 0;
                        i < profile.environmentVariables.length;
                        i++
                      ) ...<Widget>[
                        if (i > 0) const Divider(height: 1),
                        _DetailRow(
                          label: profile.environmentVariables[i].key,
                          value: profile.environmentVariables[i].value,
                        ),
                      ],
                    ],
                  ),
          ),
          const SizedBox(height: 16),
          _HostSkillsSection(
            hostDeviceId: effectiveHostId,
            hostLabel: effectiveHostLabel,
            skillsAsync: effectiveHostId == null
                ? null
                : ref.watch(hostSkillsProvider(effectiveHostId)),
          ),
          const SizedBox(height: 18),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: FilledButton.icon(
              onPressed: () {
                ref.read(activeSessionControllerProvider.notifier).reset();
                Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => ThreadViewPage(agentProfileId: profile.id),
                  ),
                );
              },
              icon: const Icon(CupertinoIcons.bubble_left_bubble_right_fill),
              label: const Text('从这个 Agent 发起对话'),
            ),
          ),
          if (preferredId != profile.id) ...<Widget>[
            const SizedBox(height: 10),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16),
              child: OutlinedButton.icon(
                onPressed: () => ref
                    .read(agentProfilesControllerProvider.notifier)
                    .setPreferredProfile(profile.id),
                icon: const Icon(CupertinoIcons.star),
                label: const Text('设为默认 Agent'),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class AgentEditorSheet extends ConsumerStatefulWidget {
  const AgentEditorSheet({
    super.key,
    required this.hosts,
    required this.descriptors,
    this.profile,
  });

  final AgentProfile? profile;
  final List<HostSummaryDto> hosts;
  final List<AgentDescriptor> descriptors;

  @override
  ConsumerState<AgentEditorSheet> createState() => _AgentEditorSheetState();
}

class _AgentEditorSheetState extends ConsumerState<AgentEditorSheet> {
  late final TextEditingController _nameController;
  late final TextEditingController _descriptionController;
  late List<AgentEnvironmentVariable> _envVars;
  late AgentName _runtimeAgent;
  late String _model;
  late AgentReasoningEffort _reasoningEffort;
  String? _hostDeviceId;
  String? _hostDisplayName;
  bool _showAdvanced = false;

  @override
  void initState() {
    super.initState();
    final draft = widget.profile == null
        ? _seedDraft(widget.hosts)
        : AgentProfileDraft.fromProfile(widget.profile!);
    _nameController = TextEditingController(text: draft.name);
    _nameController.addListener(_handleDraftChanged);
    _descriptionController = TextEditingController(text: draft.description);
    _envVars = List<AgentEnvironmentVariable>.from(draft.environmentVariables);
    _runtimeAgent = draft.runtimeAgent;
    _model = draft.model;
    _reasoningEffort = draft.reasoningEffort;
    _hostDeviceId = draft.hostDeviceId;
    _hostDisplayName = draft.hostDisplayName;
  }

  void _handleDraftChanged() {
    if (!mounted) return;
    setState(() {});
  }

  @override
  void dispose() {
    _nameController.removeListener(_handleDraftChanged);
    _nameController.dispose();
    _descriptionController.dispose();
    super.dispose();
  }

  AgentProfileDraft _seedDraft(List<HostSummaryDto> hosts) {
    final activeId = ref.read(activeMacProvider).asData?.value;
    HostSummaryDto? host;
    for (final candidate in hosts) {
      if (candidate.hostDeviceId == activeId) {
        host = candidate;
        break;
      }
    }
    host ??= hosts.isEmpty ? null : hosts.first;
    return AgentProfileDraft(
      name: 'new-agent',
      description: '',
      runtimeAgent: _preferredRuntime(widget.descriptors),
      model: _defaultModel(_preferredRuntime(widget.descriptors)),
      reasoningEffort: AgentReasoningEffort.medium,
      environmentVariables: const <AgentEnvironmentVariable>[],
      hostDeviceId: host?.hostDeviceId,
      hostDisplayName: host?.hostDisplayName,
    );
  }

  String get _hostSelectionLabel {
    final explicit = _hostDisplayName?.trim();
    if (explicit != null && explicit.isNotEmpty) return explicit;
    final fromList = _hostLabelForId(widget.hosts, _hostDeviceId);
    if (fromList != null && fromList.trim().isNotEmpty) return fromList;
    return '跟随当前 runtime';
  }

  Future<T?> _showPickerSheet<T>({
    required String title,
    required List<_PickerOption<T>> options,
    required T? currentValue,
  }) {
    return showModalBottomSheet<T>(
      context: context,
      useSafeArea: true,
      showDragHandle: true,
      backgroundColor: Theme.of(context).colorScheme.surface,
      builder: (sheetContext) {
        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                Text(
                  title,
                  style: Theme.of(sheetContext).textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.w800,
                  ),
                ),
                const SizedBox(height: 12),
                Flexible(
                  child: ListView.separated(
                    shrinkWrap: true,
                    itemCount: options.length,
                    separatorBuilder: (_, _) => const Divider(height: 1),
                    itemBuilder: (_, index) {
                      final option = options[index];
                      final selected = option.value == currentValue;
                      return ListTile(
                        contentPadding: const EdgeInsets.symmetric(
                          horizontal: 4,
                          vertical: 2,
                        ),
                        title: Text(option.title),
                        subtitle: option.subtitle == null
                            ? null
                            : Text(option.subtitle!),
                        trailing: selected
                            ? const Icon(CupertinoIcons.check_mark)
                            : null,
                        onTap: () =>
                            Navigator.of(sheetContext).pop(option.value),
                      );
                    },
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }

  Future<void> _pickHost() async {
    final hostOptions = <_PickerOption<String?>>[
      const _PickerOption<String?>(
        value: null,
        title: '跟随当前 runtime',
        subtitle: '始终使用当前激活设备',
      ),
      for (final host in widget.hosts)
        _PickerOption<String?>(
          value: host.hostDeviceId,
          title:
              _hostLabelForId(widget.hosts, host.hostDeviceId) ??
              host.hostDeviceId,
          subtitle: host.hostDeviceId,
        ),
    ];
    final selected = await _showPickerSheet<String?>(
      title: '选择运行设备',
      options: hostOptions,
      currentValue: _hostDeviceId,
    );
    if (!mounted) return;
    HostSummaryDto? selectedHost;
    for (final host in widget.hosts) {
      if (host.hostDeviceId == selected) {
        selectedHost = host;
        break;
      }
    }
    setState(() {
      _hostDeviceId = selected;
      _hostDisplayName = selectedHost?.hostDisplayName;
    });
  }

  Future<void> _pickRuntime(List<AgentName> runtimeOptions) async {
    final selected = await _showPickerSheet<AgentName>(
      title: '选择 Runtime',
      options: <_PickerOption<AgentName>>[
        for (final agent in runtimeOptions)
          _PickerOption<AgentName>(
            value: agent,
            title: _runtimeLabel(agent),
            subtitle: _defaultModel(agent),
          ),
      ],
      currentValue: _runtimeAgent,
    );
    if (selected == null || !mounted) return;
    setState(() {
      _runtimeAgent = selected;
      _model = _defaultModel(selected);
    });
  }

  Future<void> _pickModel(List<String> models) async {
    final selected = await _showPickerSheet<String>(
      title: '选择模型',
      options: <_PickerOption<String>>[
        for (final model in models)
          _PickerOption<String>(value: model, title: model),
      ],
      currentValue: _model,
    );
    if (selected == null || !mounted) return;
    setState(() => _model = selected);
  }

  Future<void> _saveProfile() async {
    final draft = AgentProfileDraft(
      name: _nameController.text,
      description: _descriptionController.text,
      runtimeAgent: _runtimeAgent,
      model: _model,
      reasoningEffort: _reasoningEffort,
      environmentVariables: _envVars,
      hostDeviceId: _hostDeviceId,
      hostDisplayName: _hostDisplayName,
    );
    final controller = ref.read(agentProfilesControllerProvider.notifier);
    if (widget.profile == null) {
      final created = await controller.createProfile(draft);
      await controller.setPreferredProfile(created.id);
    } else {
      await controller.updateProfile(widget.profile!, draft);
    }
    if (mounted) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    final isEditing = widget.profile != null;
    final hosts = widget.hosts;
    final runtimeOptions = _runtimeOptions(widget.descriptors);
    final models = _modelOptions(_runtimeAgent);
    if (!models.contains(_model)) {
      _model = models.first;
    }
    final canSave = _nameController.text.trim().isNotEmpty;
    final media = MediaQuery.of(context);
    final navigator = Navigator.of(context);

    return AnimatedPadding(
      duration: const Duration(milliseconds: 160),
      curve: Curves.easeOut,
      padding: EdgeInsets.only(bottom: media.viewInsets.bottom),
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: media.size.height * 0.9),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(20, 10, 20, 20),
          child: Column(
            children: <Widget>[
              Container(
                width: 42,
                height: 5,
                decoration: BoxDecoration(
                  color: Theme.of(context).colorScheme.outlineVariant,
                  borderRadius: BorderRadius.circular(999),
                ),
              ),
              const SizedBox(height: 16),
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: <Widget>[
                        Text(
                          isEditing ? '编辑 Agent' : '创建 Agent',
                          style: Theme.of(context).textTheme.headlineSmall
                              ?.copyWith(
                                fontWeight: FontWeight.w800,
                                letterSpacing: -0.4,
                              ),
                        ),
                        const SizedBox(height: 4),
                        Text(
                          '用移动端常见的表单交互配置名称、运行设备和模型。',
                          style: Theme.of(context).textTheme.bodyMedium
                              ?.copyWith(
                                color: Theme.of(
                                  context,
                                ).colorScheme.onSurfaceVariant,
                              ),
                        ),
                      ],
                    ),
                  ),
                  IconButton(
                    onPressed: () => navigator.pop(),
                    icon: const Icon(CupertinoIcons.xmark),
                  ),
                ],
              ),
              const SizedBox(height: 16),
              Expanded(
                child: ListView(
                  children: <Widget>[
                    _EditorSection(
                      title: '基本信息',
                      child: Padding(
                        padding: const EdgeInsets.all(14),
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: <Widget>[
                            Text(
                              '名称',
                              style: Theme.of(context).textTheme.labelLarge,
                            ),
                            const SizedBox(height: 8),
                            ShadInput(
                              controller: _nameController,
                              placeholder: const Text('例如：codex'),
                              padding: const EdgeInsets.symmetric(
                                horizontal: 14,
                                vertical: 14,
                              ),
                            ),
                            const SizedBox(height: 14),
                            Text(
                              '描述',
                              style: Theme.of(context).textTheme.labelLarge,
                            ),
                            const SizedBox(height: 8),
                            ShadInput(
                              controller: _descriptionController,
                              minLines: 3,
                              maxLines: 5,
                              placeholder: const Text('简短描述这个 Agent 擅长什么。'),
                              padding: const EdgeInsets.all(14),
                            ),
                          ],
                        ),
                      ),
                    ),
                    const SizedBox(height: 14),
                    _EditorSection(
                      title: '运行配置',
                      child: Column(
                        children: <Widget>[
                          _EditorPickerTile(
                            label: '运行设备',
                            value: _hostSelectionLabel,
                            onTap: _pickHost,
                          ),
                          const Divider(height: 1),
                          _EditorPickerTile(
                            label: 'Runtime',
                            value: _runtimeLabel(_runtimeAgent),
                            onTap: () => _pickRuntime(runtimeOptions),
                          ),
                          const Divider(height: 1),
                          _EditorPickerTile(
                            label: 'Model',
                            value: _model,
                            onTap: () => _pickModel(models),
                          ),
                        ],
                      ),
                    ),
                    if (hosts.isEmpty) ...<Widget>[
                      const SizedBox(height: 12),
                      _EditorHintCard(
                        message: '还没有连接 runtime。创建后也可以稍后绑定，但推荐先扫码连接一台设备。',
                        actionLabel: '去扫码连接',
                        onPressed: () {
                          navigator.pop();
                          navigator.push(
                            MaterialPageRoute<void>(
                              builder: (_) => const PairingPage(),
                            ),
                          );
                        },
                      ),
                    ],
                    const SizedBox(height: 14),
                    _EditorSection(
                      title: '推理强度',
                      child: Padding(
                        padding: const EdgeInsets.all(14),
                        child:
                            CupertinoSlidingSegmentedControl<
                              AgentReasoningEffort
                            >(
                              groupValue: _reasoningEffort,
                              children: const <AgentReasoningEffort, Widget>{
                                AgentReasoningEffort.low: Padding(
                                  padding: EdgeInsets.symmetric(horizontal: 8),
                                  child: Text('低'),
                                ),
                                AgentReasoningEffort.medium: Padding(
                                  padding: EdgeInsets.symmetric(horizontal: 8),
                                  child: Text('中'),
                                ),
                                AgentReasoningEffort.high: Padding(
                                  padding: EdgeInsets.symmetric(horizontal: 8),
                                  child: Text('高'),
                                ),
                              },
                              onValueChanged: (value) {
                                if (value == null) return;
                                setState(() => _reasoningEffort = value);
                              },
                            ),
                      ),
                    ),
                    const SizedBox(height: 14),
                    _EditorSection(
                      title: '高级设置',
                      child: Column(
                        children: <Widget>[
                          InkWell(
                            onTap: () =>
                                setState(() => _showAdvanced = !_showAdvanced),
                            child: Padding(
                              padding: const EdgeInsets.fromLTRB(
                                14,
                                14,
                                14,
                                14,
                              ),
                              child: Row(
                                children: <Widget>[
                                  Expanded(
                                    child: Text(
                                      '环境变量',
                                      style: Theme.of(context)
                                          .textTheme
                                          .titleSmall
                                          ?.copyWith(
                                            fontWeight: FontWeight.w700,
                                          ),
                                    ),
                                  ),
                                  Icon(
                                    _showAdvanced
                                        ? CupertinoIcons.chevron_down
                                        : CupertinoIcons.chevron_right,
                                    size: 18,
                                  ),
                                ],
                              ),
                            ),
                          ),
                          if (_showAdvanced) ...<Widget>[
                            const Divider(height: 1),
                            Padding(
                              padding: const EdgeInsets.fromLTRB(14, 12, 14, 0),
                              child: Text(
                                '这些变量只保存在当前手机上的 Agent profile 中。',
                                style: Theme.of(context).textTheme.bodySmall
                                    ?.copyWith(
                                      color: Theme.of(
                                        context,
                                      ).colorScheme.onSurfaceVariant,
                                    ),
                              ),
                            ),
                            for (var i = 0; i < _envVars.length; i++)
                              Padding(
                                padding: const EdgeInsets.fromLTRB(
                                  14,
                                  12,
                                  14,
                                  0,
                                ),
                                child: Container(
                                  padding: const EdgeInsets.all(12),
                                  decoration: BoxDecoration(
                                    color: Theme.of(
                                      context,
                                    ).colorScheme.surfaceContainerLowest,
                                    borderRadius: BorderRadius.circular(16),
                                    border: Border.all(
                                      color: Theme.of(
                                        context,
                                      ).colorScheme.outlineVariant,
                                    ),
                                  ),
                                  child: Row(
                                    children: <Widget>[
                                      Expanded(
                                        child: TextFormField(
                                          initialValue: _envVars[i].key,
                                          decoration: const InputDecoration(
                                            hintText: 'KEY',
                                            border: InputBorder.none,
                                            isDense: true,
                                          ),
                                          onChanged: (value) {
                                            _envVars[i] = _envVars[i].copyWith(
                                              key: value,
                                            );
                                          },
                                        ),
                                      ),
                                      const SizedBox(width: 8),
                                      Expanded(
                                        child: TextFormField(
                                          initialValue: _envVars[i].value,
                                          decoration: const InputDecoration(
                                            hintText: 'VALUE',
                                            border: InputBorder.none,
                                            isDense: true,
                                          ),
                                          onChanged: (value) {
                                            _envVars[i] = _envVars[i].copyWith(
                                              value: value,
                                            );
                                          },
                                        ),
                                      ),
                                      IconButton(
                                        onPressed: () => setState(
                                          () => _envVars.removeAt(i),
                                        ),
                                        icon: const Icon(
                                          CupertinoIcons.minus_circle_fill,
                                        ),
                                      ),
                                    ],
                                  ),
                                ),
                              ),
                            Padding(
                              padding: const EdgeInsets.fromLTRB(14, 8, 14, 14),
                              child: Align(
                                alignment: Alignment.centerLeft,
                                child: TextButton.icon(
                                  onPressed: () => setState(
                                    () => _envVars.add(
                                      const AgentEnvironmentVariable(
                                        key: '',
                                        value: '',
                                      ),
                                    ),
                                  ),
                                  icon: const Icon(CupertinoIcons.add),
                                  label: const Text('添加变量'),
                                ),
                              ),
                            ),
                          ],
                        ],
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 14),
              Row(
                children: <Widget>[
                  Expanded(
                    child: ShadButton.outline(
                      onPressed: () => navigator.pop(),
                      child: const Text('取消'),
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: ShadButton(
                      onPressed: canSave ? _saveProfile : null,
                      child: Text(isEditing ? '保存' : '创建'),
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _PickerOption<T> {
  const _PickerOption({
    required this.value,
    required this.title,
    this.subtitle,
  });

  final T value;
  final String title;
  final String? subtitle;
}

class _EditorSection extends StatelessWidget {
  const _EditorSection({required this.title, required this.child});

  final String title;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        color: theme.colorScheme.surface,
        borderRadius: BorderRadius.circular(16),
        border: Border.all(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.5),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Padding(
            padding: const EdgeInsets.fromLTRB(14, 14, 14, 0),
            child: Text(
              title,
              style: theme.textTheme.labelMedium?.copyWith(
                fontWeight: FontWeight.w700,
                color: theme.colorScheme.onSurfaceVariant,
                letterSpacing: 0.6,
              ),
            ),
          ),
          child,
        ],
      ),
    );
  }
}

class _EditorPickerTile extends StatelessWidget {
  const _EditorPickerTile({
    required this.label,
    required this.value,
    required this.onTap,
  });

  final String label;
  final String value;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 2),
      title: Text(label),
      subtitle: Text(value, maxLines: 1, overflow: TextOverflow.ellipsis),
      trailing: const Icon(CupertinoIcons.chevron_right, size: 18),
      onTap: onTap,
    );
  }
}

class _EditorHintCard extends StatelessWidget {
  const _EditorHintCard({
    required this.message,
    required this.actionLabel,
    required this.onPressed,
  });

  final String message;
  final String actionLabel;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: Theme.of(context).colorScheme.surface,
        borderRadius: BorderRadius.circular(20),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Text(message),
          const SizedBox(height: 12),
          ShadButton.secondary(
            onPressed: onPressed,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                const Icon(CupertinoIcons.qrcode_viewfinder, size: 16),
                const SizedBox(width: 8),
                Text(actionLabel),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _ProfilesSection extends ConsumerWidget {
  const _ProfilesSection({
    required this.state,
    required this.preferredProfileId,
    required this.hosts,
    required this.onAdd,
  });

  final AgentWorkspaceState state;
  final String? preferredProfileId;
  final List<HostSummaryDto> hosts;
  final VoidCallback onAdd;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final ordered = [...state.profiles]
      ..sort((left, right) {
        if (left.id == preferredProfileId) return -1;
        if (right.id == preferredProfileId) return 1;
        return right.updatedAtMs.compareTo(left.updatedAtMs);
      });
    return _SectionCard(
      title: 'AGENTS ${ordered.length}',
      trailing: _SectionActionButton(tooltip: '新建 Agent', onPressed: onAdd),
      child: ordered.isEmpty
          ? Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 18),
              child: Text(
                '还没有 Agent。点右上角添加一个，并把它绑定到对应的 runtime。',
                style: Theme.of(context).textTheme.bodyMedium,
              ),
            )
          : Column(
              children: <Widget>[
                for (
                  var index = 0;
                  index < ordered.length;
                  index++
                ) ...<Widget>[
                  if (index > 0) const Divider(height: 1),
                  _AgentProfileTile(
                    profile: ordered[index],
                    hostLabel: _resolvedProfileHostLabel(ordered[index], hosts),
                    isPreferred: ordered[index].id == preferredProfileId,
                    onTap: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) =>
                            AgentProfilePage(profileId: ordered[index].id),
                      ),
                    ),
                    onMakeDefault: () => ref
                        .read(agentProfilesControllerProvider.notifier)
                        .setPreferredProfile(ordered[index].id),
                    onDelete: () {
                      showShadDialog(
                        context: context,
                        builder: (context) => ShadDialog.alert(
                          title: const Text('删除 Agent'),
                          description: const Text('确定要删除这个 Agent 吗？此操作无法撤销。'),
                          actions: [
                            ShadButton.outline(
                              child: const Text('取消'),
                              onPressed: () => Navigator.of(context).pop(),
                            ),
                            ShadButton.destructive(
                              child: const Text('删除'),
                              onPressed: () async {
                                await ref
                                    .read(
                                      agentProfilesControllerProvider.notifier,
                                    )
                                    .deleteProfile(ordered[index].id);
                                if (context.mounted) {
                                  Navigator.of(context).pop();
                                }
                              },
                            ),
                          ],
                        ),
                      );
                    },
                  ),
                ],
              ],
            ),
    );
  }
}

class _HostRuntimeCard extends ConsumerWidget {
  const _HostRuntimeCard({
    required this.pairedHosts,
    required this.activeHostId,
    required this.connection,
  });

  final AsyncValue<List<HostSummaryDto>> pairedHosts;
  final String? activeHostId;
  final minos_api.ConnectionState? connection;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    return _SectionCard(
      title: 'DEVICES ${pairedHosts.asData?.value.length ?? 0}',
      trailing: _SectionActionButton(
        tooltip: '添加设备',
        onPressed: () => Navigator.of(
          context,
        ).push(MaterialPageRoute<void>(builder: (_) => const PairingPage())),
      ),
      child: pairedHosts.when(
        loading: () => const _DeviceListSkeleton(),
        error: (error, _) => Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 18),
          child: _CompactErrorPanel(
            title: '设备暂时不可用',
            description: error.toString(),
            actionLabel: '重试',
            onAction: () async {
              try {
                await ref.read(pairedMacsProvider.notifier).refresh();
              } catch (error) {
                if (context.mounted) {
                  _showRefreshError(context, '设备刷新失败', error);
                }
              }
            },
          ),
        ),
        data: (hosts) {
          if (hosts.isEmpty) {
            return Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 18),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    '还没有连接任何 runtime。先扫码连接设备，再把 Agent 绑定到对应电脑。',
                    style: theme.textTheme.bodyMedium,
                  ),
                  const SizedBox(height: 12),
                  ShadButton(
                    onPressed: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => const PairingPage(),
                      ),
                    ),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: const [
                        Icon(CupertinoIcons.qrcode_viewfinder, size: 16),
                        SizedBox(width: 8),
                        Text('添加 Runtime'),
                      ],
                    ),
                  ),
                ],
              ),
            );
          }

          return Column(
            children: <Widget>[
              for (var index = 0; index < hosts.length; index++) ...<Widget>[
                if (index > 0) const Divider(height: 1),
                _DeviceRosterTile(
                  host: hosts[index],
                  isActive: hosts[index].hostDeviceId == activeHostId,
                  connection: connection,
                  onTap: () => ref
                      .read(activeMacProvider.notifier)
                      .setActive(hosts[index].hostDeviceId),
                  onDelete: () {
                    showShadDialog(
                      context: context,
                      builder: (context) => ShadDialog.alert(
                        title: const Text('移除设备'),
                        description: const Text('确定要移除此设备吗？此操作无法撤销。'),
                        actions: [
                          ShadButton.outline(
                            child: const Text('取消'),
                            onPressed: () => Navigator.of(context).pop(),
                          ),
                          ShadButton.destructive(
                            child: const Text('移除'),
                            onPressed: () async {
                              final core = ref.read(minosCoreProvider);
                              await core.forgetHost(hosts[index].hostDeviceId);
                              try {
                                await ref
                                    .read(pairedMacsProvider.notifier)
                                    .refresh();
                              } catch (_) {}
                              await ref
                                  .read(activeMacProvider.notifier)
                                  .refresh();
                              if (context.mounted) {
                                Navigator.of(context).pop();
                              }
                            },
                          ),
                        ],
                      ),
                    );
                  },
                ),
              ],
            ],
          );
        },
      ),
    );
  }
}

class _AgentProfileTile extends StatelessWidget {
  const _AgentProfileTile({
    required this.profile,
    required this.hostLabel,
    required this.isPreferred,
    required this.onTap,
    required this.onMakeDefault,
    this.onDelete,
  });

  final AgentProfile profile;
  final String hostLabel;
  final bool isPreferred;
  final VoidCallback onTap;
  final VoidCallback onMakeDefault;
  final VoidCallback? onDelete;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final detail = profile.description.trim().isEmpty
        ? '${_runtimeLabel(profile.runtimeAgent)} · ${profile.model}'
        : profile.description.trim();
    final isDark = theme.brightness == Brightness.dark;

    final highlight = isPreferred
        ? theme.colorScheme.primaryContainer.withValues(
            alpha: isDark ? 0.2 : 0.4,
          )
        : Colors.transparent;

    return InkWell(
      onTap: onTap,
      onLongPress: isPreferred ? null : onMakeDefault,
      child: Container(
        color: highlight,
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            _AgentAvatar(agent: profile.runtimeAgent, size: 42),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    hostLabel,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    profile.name,
                    style: theme.textTheme.titleLarge?.copyWith(
                      fontWeight: FontWeight.w800,
                      letterSpacing: 0,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    detail,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 12),
            Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: _MemberStatusDot(
                    color: isPreferred
                        ? (isDark
                              ? const Color(0xFF22C55E)
                              : const Color(0xFF16A34A))
                        : theme.colorScheme.outline,
                  ),
                ),
                if (onDelete != null) ...[
                  const SizedBox(height: 8),
                  ShadButton.ghost(
                    width: 32,
                    height: 32,
                    padding: EdgeInsets.zero,
                    onPressed: onDelete,
                    child: const Icon(CupertinoIcons.trash, size: 16),
                  ),
                ],
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _DeviceRosterTile extends StatelessWidget {
  const _DeviceRosterTile({
    required this.host,
    required this.isActive,
    required this.connection,
    required this.onTap,
    this.onDelete,
  });

  final HostSummaryDto host;
  final bool isActive;
  final minos_api.ConnectionState? connection;
  final VoidCallback onTap;
  final VoidCallback? onDelete;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final title = host.hostDisplayName.trim().isEmpty
        ? host.hostDeviceId
        : host.hostDisplayName.trim();
    final stateLabel = isActive ? _connectionLabel(connection) : '已配对设备';
    final subtitle = title == host.hostDeviceId
        ? stateLabel
        : '${host.hostDeviceId} · $stateLabel';
    final isDark = theme.brightness == Brightness.dark;
    final highlight = isActive
        ? theme.colorScheme.secondaryContainer.withValues(
            alpha: isDark ? 0.2 : 0.4,
          )
        : Colors.transparent;

    return InkWell(
      onTap: onTap,
      child: Container(
        color: highlight,
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        child: Row(
          children: <Widget>[
            const _DeviceAvatar(),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    subtitle,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 12),
            Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                _MemberStatusDot(
                  color: _connectionColor(connection, isActive, isDark),
                ),
                if (onDelete != null) ...[
                  const SizedBox(height: 8),
                  ShadButton.ghost(
                    width: 32,
                    height: 32,
                    padding: EdgeInsets.zero,
                    onPressed: onDelete,
                    child: const Icon(CupertinoIcons.trash, size: 16),
                  ),
                ],
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _HumansSection extends StatelessWidget {
  const _HumansSection({required this.authState});

  final AuthState authState;

  @override
  Widget build(BuildContext context) {
    return _SectionCard(
      title: 'HUMANS ${authState is AuthAuthenticated ? 1 : 0}',
      child: switch (authState) {
        AuthAuthenticated(:final account) => _HumanMemberTile(account: account),
        AuthBootstrapping() => const _HumanListSkeleton(),
        _ => const Padding(
          padding: EdgeInsets.fromLTRB(16, 8, 16, 18),
          child: Text('当前未登录，无法展示人类成员。'),
        ),
      },
    );
  }
}

class _HumanMemberTile extends StatelessWidget {
  const _HumanMemberTile({required this.account});

  final AuthSummary account;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final title = _humanDisplayName(account.email);
    final initial = title.isEmpty ? 'Y' : title.substring(0, 1).toUpperCase();
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      child: Row(
        children: <Widget>[
          _HumanAvatar(label: initial),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(
                  '$title (you)',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const SizedBox(height: 4),
                Text(
                  account.email,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: 12),
          _MemberStatusDot(
            color: theme.brightness == Brightness.dark
                ? const Color(0xFF22C55E)
                : const Color(0xFF16A34A),
          ),
        ],
      ),
    );
  }
}

class _ProfileHero extends StatelessWidget {
  const _ProfileHero({required this.profile, required this.isPreferred});

  final AgentProfile profile;
  final bool isPreferred;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    return Container(
      padding: const EdgeInsets.all(18),
      decoration: BoxDecoration(
        color: theme.colorScheme.surfaceContainerLow,
        borderRadius: BorderRadius.circular(16),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: isDark ? 0.08 : 0.03),
            blurRadius: 10,
            offset: const Offset(0, 2),
          ),
        ],
      ),
      child: Row(
        children: <Widget>[
          _AgentAvatar(agent: profile.runtimeAgent, size: 64),
          const SizedBox(width: 16),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Row(
                  children: <Widget>[
                    Expanded(
                      child: Text(
                        profile.name,
                        style: theme.textTheme.headlineSmall?.copyWith(
                          fontWeight: FontWeight.w800,
                        ),
                      ),
                    ),
                    if (isPreferred)
                      _BadgeChip(
                        label: 'Default',
                        background: theme.colorScheme.primaryContainer,
                      ),
                  ],
                ),
                const SizedBox(height: 6),
                Text(
                  profile.description.isEmpty
                      ? 'General-purpose agent profile'
                      : profile.description,
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
                const SizedBox(height: 10),
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: <Widget>[
                    _BadgeChip(label: _runtimeLabel(profile.runtimeAgent)),
                    _BadgeChip(label: profile.model),
                    if (profile.hostDisplayName != null)
                      _BadgeChip(label: profile.hostDisplayName!),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _DetailRow extends StatelessWidget {
  const _DetailRow({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          SizedBox(
            width: 120,
            child: Text(
              label,
              style: Theme.of(context).textTheme.labelLarge?.copyWith(
                color: Theme.of(context).colorScheme.onSurfaceVariant,
                fontWeight: FontWeight.w700,
              ),
            ),
          ),
          Expanded(
            child: Text(value, style: Theme.of(context).textTheme.bodyLarge),
          ),
        ],
      ),
    );
  }
}

class _HostSkillsSection extends ConsumerWidget {
  const _HostSkillsSection({
    required this.hostDeviceId,
    required this.hostLabel,
    required this.skillsAsync,
  });

  final String? hostDeviceId;
  final String hostLabel;
  final AsyncValue<List<HostSkillsEntry>>? skillsAsync;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final resolvedHostDeviceId = hostDeviceId;
    final resolvedSkillsAsync = skillsAsync;
    if (resolvedHostDeviceId == null || resolvedSkillsAsync == null) {
      return const _SectionCard(
        title: 'Skills',
        child: Padding(
          padding: EdgeInsets.all(16),
          child: Text('先为这个 Agent 绑定一个 runtime host，才能扫描和编辑 host skills。'),
        ),
      );
    }

    return _SectionCard(
      title: 'Skills',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 10, 8, 0),
            child: Row(
              children: <Widget>[
                Expanded(
                  child: Text(
                    'Runtime · $hostLabel',
                    style: theme.textTheme.labelLarge?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ),
                IconButton(
                  tooltip: '重新扫描',
                  onPressed: () =>
                      ref.invalidate(hostSkillsProvider(resolvedHostDeviceId)),
                  icon: const Icon(CupertinoIcons.refresh),
                ),
              ],
            ),
          ),
          resolvedSkillsAsync.when(
            loading: () => const Padding(
              padding: EdgeInsets.all(18),
              child: Center(child: CupertinoActivityIndicator()),
            ),
            error: (error, _) => Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    '读取 host skills 失败: $error',
                    style: theme.textTheme.bodyMedium,
                  ),
                  const SizedBox(height: 10),
                  OutlinedButton.icon(
                    onPressed: () => ref.invalidate(
                      hostSkillsProvider(resolvedHostDeviceId),
                    ),
                    icon: const Icon(CupertinoIcons.refresh),
                    label: const Text('重试'),
                  ),
                ],
              ),
            ),
            data: (entries) {
              final hasSkills = entries.any((entry) => entry.skills.isNotEmpty);
              final hasErrors = entries.any((entry) => entry.errors.isNotEmpty);
              if (!hasSkills && !hasErrors) {
                return const Padding(
                  padding: EdgeInsets.all(16),
                  child: Text('当前 host 上没有扫描到可用 skills。'),
                );
              }
              return Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  for (var i = 0; i < entries.length; i++) ...<Widget>[
                    const Divider(height: 1),
                    Padding(
                      padding: const EdgeInsets.fromLTRB(16, 12, 16, 6),
                      child: Text(
                        entries[i].cwd,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.labelMedium?.copyWith(
                          color: theme.colorScheme.onSurfaceVariant,
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                    ),
                    for (final error in entries[i].errors)
                      _HostSkillErrorTile(error: error),
                    if (entries[i].errors.isNotEmpty &&
                        entries[i].skills.isNotEmpty)
                      const Divider(height: 1),
                    for (
                      var skillIndex = 0;
                      skillIndex < entries[i].skills.length;
                      skillIndex++
                    ) ...<Widget>[
                      if (skillIndex > 0) const Divider(height: 1),
                      _HostSkillTile(
                        hostDeviceId: resolvedHostDeviceId,
                        skill: entries[i].skills[skillIndex],
                      ),
                    ],
                  ],
                ],
              );
            },
          ),
        ],
      ),
    );
  }
}

class _HostSkillTile extends ConsumerWidget {
  const _HostSkillTile({required this.hostDeviceId, required this.skill});

  final String hostDeviceId;
  final HostSkillSummary skill;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final displayName =
        skill.displayName != null && skill.displayName!.trim().isNotEmpty
        ? skill.displayName!
        : skill.name;
    final secondary =
        skill.shortDescription != null &&
            skill.shortDescription!.trim().isNotEmpty
        ? skill.shortDescription!
        : skill.description;

    return SwitchListTile.adaptive(
      value: skill.enabled,
      contentPadding: const EdgeInsets.fromLTRB(16, 4, 12, 8),
      title: Text(displayName),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          const SizedBox(height: 4),
          Text(secondary, maxLines: 2, overflow: TextOverflow.ellipsis),
          const SizedBox(height: 8),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: <Widget>[
              _BadgeChip(label: skill.scope.toUpperCase()),
              if (displayName != skill.name) _BadgeChip(label: skill.name),
            ],
          ),
          const SizedBox(height: 8),
          Text(
            skill.path,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ],
      ),
      onChanged: (enabled) => _setSkillEnabled(context, ref, enabled),
    );
  }

  Future<void> _setSkillEnabled(
    BuildContext context,
    WidgetRef ref,
    bool enabled,
  ) async {
    try {
      await ref
          .read(minosCoreProvider)
          .writeHostSkillConfig(
            hostDeviceId: hostDeviceId,
            path: skill.path,
            enabled: enabled,
          );
      ref.invalidate(hostSkillsProvider(hostDeviceId));
    } catch (error) {
      if (!context.mounted) return;
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text('更新 skill 失败: $error')));
    }
  }
}

class _HostSkillErrorTile extends StatelessWidget {
  const _HostSkillErrorTile({required this.error});

  final HostSkillError error;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return ListTile(
      leading: const Icon(CupertinoIcons.exclamationmark_triangle_fill),
      title: Text(error.message),
      subtitle: Text(
        error.path,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: theme.textTheme.bodySmall?.copyWith(
          color: theme.colorScheme.onSurfaceVariant,
        ),
      ),
    );
  }
}

class _SectionCard extends StatelessWidget {
  const _SectionCard({required this.title, required this.child, this.trailing});

  final String title;
  final Widget child;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 20, 4, 8),
            child: Row(
              children: <Widget>[
                Expanded(
                  child: Text(
                    title,
                    style: theme.textTheme.labelMedium?.copyWith(
                      fontWeight: FontWeight.w700,
                      color: theme.colorScheme.onSurfaceVariant,
                      letterSpacing: 0.8,
                    ),
                  ),
                ),
                ?trailing,
              ],
            ),
          ),
          Container(
            clipBehavior: Clip.antiAlias,
            decoration: BoxDecoration(
              color: theme.colorScheme.surfaceContainerLow,
              borderRadius: BorderRadius.circular(16),
              boxShadow: [
                BoxShadow(
                  color: Colors.black.withValues(alpha: 0.015),
                  blurRadius: 10,
                  offset: const Offset(0, 1),
                ),
              ],
            ),
            child: child,
          ),
        ],
      ),
    );
  }
}

class _BadgeChip extends StatelessWidget {
  const _BadgeChip({this.background, required this.label});

  final Color? background;
  final String label;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    final bg =
        background ??
        (isDark
            ? theme.colorScheme.surfaceContainerHighest
            : theme.colorScheme.surfaceContainerHigh);
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(6),
      ),
      child: Text(
        label,
        style: theme.textTheme.labelSmall?.copyWith(
          fontWeight: FontWeight.w600,
          color: theme.colorScheme.onSurfaceVariant,
        ),
      ),
    );
  }
}

class _AgentAvatar extends StatelessWidget {
  const _AgentAvatar({required this.agent, this.size = 32});

  final AgentName agent;
  final double size;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
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
          fontWeight: FontWeight.w800,
          fontSize: size * 0.42,
        ),
      ),
    );
  }
}

String? _hostLabelForId(List<HostSummaryDto> hosts, String? hostId) {
  if (hostId == null) return null;
  for (final host in hosts) {
    if (host.hostDeviceId == hostId) {
      final trimmed = host.hostDisplayName.trim();
      return trimmed.isEmpty ? host.hostDeviceId : trimmed;
    }
  }
  return hostId;
}

String _runtimeLabel(AgentName agent) {
  return switch (agent) {
    AgentName.codex => 'Codex CLI',
    AgentName.claude => 'Claude CLI',
    AgentName.gemini => 'Gemini CLI',
  };
}

String _reasoningLabel(AgentReasoningEffort value) {
  return switch (value) {
    AgentReasoningEffort.low => 'Low',
    AgentReasoningEffort.medium => 'Medium',
    AgentReasoningEffort.high => 'High',
  };
}

List<AgentName> _runtimeOptions(List<AgentDescriptor> descriptors) {
  final usable = descriptors
      .where((descriptor) => descriptor.status is AgentStatus_Ok)
      .map((descriptor) => descriptor.name)
      .toSet()
      .toList();
  if (usable.isNotEmpty) return usable;
  return AgentName.values;
}

AgentName _preferredRuntime(List<AgentDescriptor> descriptors) {
  return _runtimeOptions(descriptors).first;
}

List<String> _modelOptions(AgentName runtimeAgent) {
  return switch (runtimeAgent) {
    AgentName.codex => const <String>['GPT-5.5', 'GPT-5.1', 'o4-mini'],
    AgentName.claude => const <String>['Claude Opus 4.1', 'Claude Sonnet 4'],
    AgentName.gemini => const <String>['Gemini 2.5 Pro', 'Gemini 2.5 Flash'],
  };
}

String _defaultModel(AgentName runtimeAgent) {
  return _modelOptions(runtimeAgent).first;
}
