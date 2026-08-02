import 'dart:async';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:minos/application/agent_conversation_actions.dart';
import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/application/flutter_log.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/ui_state_providers.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/ui/core/widgets/error_feedback.dart';
import 'package:minos/ui/core/widgets/minos_button.dart';
import 'package:minos/ui/core/widgets/minos_progress.dart';
import 'package:minos/ui/features/agents/views/agents_hub_page.dart'
    show AgentEditorSheet;
import 'package:minos/ui/features/shell/router.dart';
import 'package:minos/ui/theme/theme.dart';

class AgentStartPage extends ConsumerWidget {
  const AgentStartPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profilesAsync = ref.watch(agentProfilesControllerProvider);
    final hosts =
        ref.watch(pairedMacsProvider).asData?.value ?? const <HostSummaryDto>[];
    final descriptors =
        ref.watch(runtimeAgentDescriptorsProvider).asData?.value ??
        const <AgentDescriptor>[];
    final pageState = ref.watch(agentStartPageStateControllerProvider);
    final colors = context.minosColors;

    return Scaffold(
      backgroundColor: colors.canvas,
      appBar: AppBar(
        title: const Text('开始 Agent 对话'),
        surfaceTintColor: Colors.transparent,
      ),
      body: profilesAsync.when(
        loading: () => const Center(child: MinosProgress()),
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
            pageState.selectedProfileId,
            workspaceState.preferredProfileId,
          );

          return SafeArea(
            bottom: false,
            child: ListView(
              padding: const EdgeInsets.fromLTRB(16, 16, 16, 24),
              children: <Widget>[
                Text(
                  '先选择一个现有 Agent，然后创建一条新的对话。',
                  style: Theme.of(
                    context,
                  ).textTheme.bodyMedium?.copyWith(color: colors.textSecondary),
                ),
                const SizedBox(height: 16),
                _PickerSection(
                  title: 'Agent',
                  trailing: MinosButton.ghost(
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
                      : RadioGroup<String>(
                          groupValue: selectedProfile?.id,
                          onChanged: (value) {
                            if (value == null) return;
                            ref
                                .read(
                                  agentStartPageStateControllerProvider
                                      .notifier,
                                )
                                .selectProfile(value);
                          },
                          child: Column(
                            children: <Widget>[
                              for (final profile in orderedProfiles)
                                RadioListTile<String>(
                                  value: profile.id,
                                  title: Text(profile.name),
                                  subtitle: Text(
                                    _profileSubtitle(profile, hosts),
                                    maxLines: 2,
                                    overflow: .ellipsis,
                                  ),
                                  contentPadding: const .symmetric(
                                    horizontal: 12,
                                  ),
                                ),
                            ],
                          ),
                        ),
                ),
                const SizedBox(height: 20),
                MinosButton(
                  onPressed: selectedProfile == null || pageState.isSubmitting
                      ? null
                      : () =>
                            _createConversation(context, ref, selectedProfile),
                  child: Text(pageState.isSubmitting ? '创建中…' : '开始对话'),
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
    String? selectedProfileId,
    String? preferredProfileId,
  ) {
    if (orderedProfiles.isEmpty) {
      return null;
    }
    for (final profile in orderedProfiles) {
      if (profile.id == selectedProfileId) {
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

  Future<void> _createConversation(
    BuildContext context,
    WidgetRef ref,
    AgentProfile profile,
  ) async {
    ref
        .read(agentStartPageStateControllerProvider.notifier)
        .setSubmitting(true);
    logFlutterInfo(
      'agent_start',
      'create conversation requested profileId=${profile.id} hostDeviceId=${profile.hostDeviceId ?? '<none>'}',
    );
    try {
      final conversation = await createAgentConversation(ref, profile: profile);
      logFlutterInfo(
        'agent_start',
        'create conversation succeeded profileId=${profile.id} conversationId=${conversation.conversationId}',
      );
      if (!context.mounted) return;
      unawaited(
        context.push(
          '/social/chat/${conversation.conversationId}',
          extra: SocialChatRouteExtra(
            title: profile.name,
            kind: ConversationKind.group,
          ),
        ),
      );
    } catch (error) {
      if (!context.mounted) return;
      showLoggedErrorToast(
        context,
        target: 'agent_start',
        title: '创建对话失败',
        error: error,
      );
    } finally {
      ref
          .read(agentStartPageStateControllerProvider.notifier)
          .setSubmitting(false);
    }
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
        padding: const .all(24),
        child: Column(
          mainAxisSize: .min,
          children: <Widget>[
            Text('加载 Agent 失败: $error', textAlign: .center),
            const SizedBox(height: 12),
            MinosButton(onPressed: onRetry, child: const Text('重试')),
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
      crossAxisAlignment: .start,
      children: <Widget>[
        Row(
          children: <Widget>[
            Expanded(
              child: Text(
                title,
                style: Theme.of(
                  context,
                ).textTheme.titleSmall?.copyWith(fontWeight: .w700),
              ),
            ),
            ?trailing,
          ],
        ),
        const SizedBox(height: 8),
        DecoratedBox(
          decoration: BoxDecoration(
            color: Theme.of(context).colorScheme.surfaceContainerLow,
            borderRadius: .circular(16),
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
      padding: const .all(16),
      child: Column(
        crossAxisAlignment: .start,
        children: <Widget>[
          Text(
            title,
            style: Theme.of(
              context,
            ).textTheme.titleMedium?.copyWith(fontWeight: .w700),
          ),
          const SizedBox(height: 6),
          Text(
            description,
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: 12),
          MinosButton(onPressed: onAction, child: Text(actionLabel)),
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
    AgentName.opencode => 'OpenCode',
    AgentName.grok => 'Grok',
  };
}
