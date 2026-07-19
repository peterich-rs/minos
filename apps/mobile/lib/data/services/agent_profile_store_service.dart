import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/infrastructure/agent_profile_store.dart';

final agentProfileStoreProvider = Provider<AgentProfileStore>((ref) {
  return const JsonFileAgentProfileStore();
});
