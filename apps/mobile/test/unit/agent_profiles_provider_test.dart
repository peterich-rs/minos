import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/infrastructure/agent_profile_store.dart';
import 'package:minos/src/rust/api/minos.dart';

class _MemoryAgentProfileStore implements AgentProfileStore {
  _MemoryAgentProfileStore([AgentWorkspaceState? initial])
    : _state = initial ?? const AgentWorkspaceState.empty();

  AgentWorkspaceState _state;
  int saveCount = 0;

  @override
  Future<AgentWorkspaceState> load() async => _state;

  @override
  Future<void> save(AgentWorkspaceState state) async {
    saveCount += 1;
    _state = state;
  }
}

ProviderContainer _container(_MemoryAgentProfileStore store) {
  final container = ProviderContainer(
    overrides: [agentProfileStoreProvider.overrideWithValue(store)],
  );
  addTearDown(container.dispose);
  return container;
}

void main() {
  test('loads an empty workspace when storage has no profiles', () async {
    final store = _MemoryAgentProfileStore();
    final container = _container(store);

    final state = await container.read(agentProfilesControllerProvider.future);
    expect(state.profiles, isEmpty);
    expect(state.preferredProfile, isNull);
    expect(container.read(preferredRuntimeAgentProvider), isNull);
  });

  test(
    'createProfile persists and bindThreadToProfile records the mapping',
    () async {
      final store = _MemoryAgentProfileStore();
      final container = _container(store);
      final controller = container.read(
        agentProfilesControllerProvider.notifier,
      );

      final created = await controller.createProfile(
        const AgentProfileDraft(
          name: 'release-bot',
          description: 'Handles release prep',
          runtimeAgent: AgentName.codex,
          model: 'GPT-5.5',
          reasoningEffort: AgentReasoningEffort.high,
          environmentVariables: <AgentEnvironmentVariable>[
            AgentEnvironmentVariable(key: 'CI', value: '1'),
          ],
        ),
      );
      await controller.setPreferredProfile(created.id);
      await controller.bindThreadToProfile(
        threadId: 'thr-agent-1',
        profileId: created.id,
      );

      final state = await container.read(
        agentProfilesControllerProvider.future,
      );
      expect(state.profiles, hasLength(1));
      expect(state.preferredProfileId, created.id);
      expect(state.profileForThread('thr-agent-1')?.name, 'release-bot');
      expect(store.saveCount, greaterThanOrEqualTo(3));
    },
  );

  test('deleteProfile allows removing the last profile', () async {
    final store = _MemoryAgentProfileStore();
    final container = _container(store);
    final controller = container.read(agentProfilesControllerProvider.notifier);

    final created = await controller.createProfile(
      const AgentProfileDraft(
        name: 'solo-agent',
        description: '',
        runtimeAgent: AgentName.claude,
        model: 'Claude Sonnet 4',
        reasoningEffort: AgentReasoningEffort.medium,
        environmentVariables: <AgentEnvironmentVariable>[],
      ),
    );

    await controller.deleteProfile(created.id);

    final state = await container.read(agentProfilesControllerProvider.future);
    expect(state.profiles, isEmpty);
    expect(state.preferredProfileId, isNull);
  });
}
