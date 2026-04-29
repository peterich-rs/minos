import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/src/rust/api/minos.dart';

class PreferredAgent extends Notifier<AgentName> {
  @override
  AgentName build() => AgentName.codex;

  void setAgent(AgentName agent) {
    state = agent;
  }
}

final preferredAgentProvider = NotifierProvider<PreferredAgent, AgentName>(
  PreferredAgent.new,
);
