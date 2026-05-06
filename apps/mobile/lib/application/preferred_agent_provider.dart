import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/agent_profiles_provider.dart';
import 'package:minos/src/rust/api/minos.dart';

final preferredAgentProvider = Provider<AgentName>((ref) {
  return ref.watch(preferredRuntimeAgentProvider);
});
