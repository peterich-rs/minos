/// Feature: Agents Hub
///
/// Agent profile management — create, edit, delete agent profiles,
/// manage host bindings, and view runtime descriptors. The "Members"
/// tab in the app shell.
///
/// View Models:
///   - [AgentProfilesController] (application/agent_profiles_provider.dart)
///   - [GroupAgentBindingsController] (application/group_agent_provider.dart)
///
/// Views:
///   - [AgentsHubTab]
///   - [AgentProfilePage]
///   - [AgentEditorSheet]
library;

export 'package:minos/application/agent_profiles_provider.dart';
export 'package:minos/application/group_agent_provider.dart';
export 'package:minos/application/preferred_agent_provider.dart';
export 'package:minos/presentation/pages/agents_hub_page.dart';
