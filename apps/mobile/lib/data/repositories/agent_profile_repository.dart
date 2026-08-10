import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/services/services.dart';
import 'package:minos/domain/agent_profile.dart';

/// Device-local cache of bot drafts / launch prefs.
///
/// Hub `agents` is bot identity SSOT; this repository does not mint multi-end
/// identity by itself — prefer [SocialRepository.registerAgent] /
/// [SocialRepository.updateAgent] when authenticated.
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
