import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/infrastructure/agent_profile_store.dart';
import 'package:minos/src/rust/api/minos.dart';

class _MemoryAgentProfileStore implements AgentProfileStore {
  _MemoryAgentProfileStore([AgentWorkspaceState? initial])
    : _state = initial ?? AgentWorkspaceState.bootstrap();

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
  test('bootstraps a preferred profile from storage', () async {
    final store = _MemoryAgentProfileStore();
    final container = _container(store);

    final state = await container.read(agentProfilesControllerProvider.future);
    expect(state.profiles, hasLength(1));
    expect(state.preferredProfile?.name, 'codex');
    expect(container.read(preferredRuntimeAgentProvider), AgentName.codex);
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
      expect(state.profiles, hasLength(2));
      expect(state.preferredProfileId, created.id);
      expect(state.profileForThread('thr-agent-1')?.name, 'release-bot');
      expect(store.saveCount, greaterThanOrEqualTo(3));
    },
  );
}
