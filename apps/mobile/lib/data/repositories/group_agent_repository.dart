import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

final groupAgentRepositoryProvider = Provider<GroupAgentRepository>((ref) {
  return GroupAgentRepository(ref.watch(minosCoreServiceProvider));
});

class GroupAgentRepository {
  const GroupAgentRepository(this._core);

  final MinosCoreProtocol _core;

  Future<List<AgentSummary>> listConversationAgents(
    String conversationId,
  ) async {
    final response = await _core.listConversationAgents(
      conversationId: conversationId,
    );
    return response.agents;
  }
}
