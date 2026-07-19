import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/services/services.dart';
import 'package:minos/domain/agent_profile.dart';

final agentProfileRepositoryProvider = Provider<AgentProfileRepository>((ref) {
  return AgentProfileRepository(ref.watch(agentProfileStoreProvider));
});

class AgentProfileRepository {
  const AgentProfileRepository(this._store);

  final AgentProfileStore _store;

  Future<AgentWorkspaceState> loadWorkspace() async {
    return (await _store.load()).normalized();
  }

  Future<void> saveWorkspace(AgentWorkspaceState state) {
    return _store.save(state.normalized());
  }
}
