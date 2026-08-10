import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Represents a member in a group conversation.
/// A member can be either a human user or an agent.
enum GroupMemberKind { user, agent }

class GroupMember {
  const GroupMember({
    required this.id,
    required this.displayName,
    required this.minosId,
    required this.kind,
    this.agentProfile,
  });

  /// The unique identifier (accountId for users, agentId for agents).
  final String id;

  /// Display name shown in the group member list and mention picker.
  final String displayName;

  /// The @-mentionable identifier (minosId for users, agentId for agents).
  final String minosId;

  /// Whether this member is a human user or an agent.
  final GroupMemberKind kind;

  /// Non-null only when [kind] is [GroupMemberKind.agent].
  final AgentProfile? agentProfile;

  bool get isAgent => kind == GroupMemberKind.agent;

  /// Create a [GroupMember] from a [UserSummary].
  factory GroupMember.fromUser(UserSummary user) {
    return GroupMember(
      id: user.accountId,
      displayName: user.displayName,
      minosId: user.minosId,
      kind: GroupMemberKind.user,
    );
  }

  /// Create a [GroupMember] from an [AgentProfile].
  factory GroupMember.fromAgent(AgentProfile profile) {
    final label = profile.name.trim().isEmpty
        ? profile.agentId
        : profile.name.trim();
    return GroupMember(
      id: profile.agentId,
      displayName: '🤖 $label',
      minosId: profile.agentId,
      kind: GroupMemberKind.agent,
      agentProfile: profile,
    );
  }

  /// Create a [GroupMember] from Hub [AgentSummary] (prefer display_name).
  factory GroupMember.fromAgentSummary(AgentSummary agent) {
    final display = agent.displayName.trim().isNotEmpty
        ? agent.displayName.trim()
        : (agent.name.trim().isNotEmpty ? agent.name.trim() : agent.agentId);
    return GroupMember(
      id: agent.agentId,
      displayName: '🤖 $display',
      minosId: agent.agentId,
      kind: GroupMemberKind.agent,
    );
  }
}
