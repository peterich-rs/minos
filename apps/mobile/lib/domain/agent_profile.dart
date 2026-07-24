import 'package:minos/src/rust/api/minos.dart';

enum AgentReasoningEffort { low, medium, high }

class AgentEnvironmentVariable {
  const AgentEnvironmentVariable({required this.key, required this.value});

  final String key;
  final String value;

  AgentEnvironmentVariable copyWith({String? key, String? value}) {
    return AgentEnvironmentVariable(
      key: key ?? this.key,
      value: value ?? this.value,
    );
  }

  Map<String, Object?> toJson() {
    return <String, Object?>{'key': key, 'value': value};
  }

  factory AgentEnvironmentVariable.fromJson(Map<String, Object?> json) {
    return AgentEnvironmentVariable(
      key: json['key'] as String? ?? '',
      value: json['value'] as String? ?? '',
    );
  }
}

class AgentProfileDraft {
  const AgentProfileDraft({
    required this.name,
    required this.description,
    required this.runtimeAgent,
    required this.model,
    required this.reasoningEffort,
    required this.environmentVariables,
    this.hostDeviceId,
    this.hostDisplayName,
    this.workspacePath,
  });

  final String name;
  final String description;
  final AgentName runtimeAgent;
  final String model;
  final AgentReasoningEffort reasoningEffort;
  final List<AgentEnvironmentVariable> environmentVariables;
  final String? hostDeviceId;
  final String? hostDisplayName;
  final String? workspacePath;

  AgentProfileDraft copyWith({
    String? name,
    String? description,
    AgentName? runtimeAgent,
    String? model,
    AgentReasoningEffort? reasoningEffort,
    List<AgentEnvironmentVariable>? environmentVariables,
    String? hostDeviceId,
    String? hostDisplayName,
    String? workspacePath,
  }) {
    return AgentProfileDraft(
      name: name ?? this.name,
      description: description ?? this.description,
      runtimeAgent: runtimeAgent ?? this.runtimeAgent,
      model: model ?? this.model,
      reasoningEffort: reasoningEffort ?? this.reasoningEffort,
      environmentVariables: environmentVariables ?? this.environmentVariables,
      hostDeviceId: hostDeviceId ?? this.hostDeviceId,
      hostDisplayName: hostDisplayName ?? this.hostDisplayName,
      workspacePath: workspacePath ?? this.workspacePath,
    );
  }

  factory AgentProfileDraft.fromProfile(AgentProfile profile) {
    return AgentProfileDraft(
      name: profile.name,
      description: profile.description,
      runtimeAgent: profile.runtimeAgent,
      model: profile.model,
      reasoningEffort: profile.reasoningEffort,
      environmentVariables: profile.environmentVariables,
      hostDeviceId: profile.hostDeviceId,
      hostDisplayName: profile.hostDisplayName,
      workspacePath: profile.workspacePath,
    );
  }
}

class AgentProfile {
  const AgentProfile({
    required this.id,
    required this.agentId,
    required this.name,
    required this.description,
    required this.runtimeAgent,
    required this.model,
    required this.reasoningEffort,
    required this.environmentVariables,
    required this.createdAtMs,
    required this.updatedAtMs,
    this.hostDeviceId,
    this.hostDisplayName,
    this.workspacePath,
  });

  final String id;

  /// Unique agent identifier used in group chats, similar to a user's uid.
  /// This allows agents to be added to group conversations and mentioned
  /// via @agentId.
  final String agentId;
  final String name;
  final String description;
  final AgentName runtimeAgent;
  final String model;
  final AgentReasoningEffort reasoningEffort;
  final List<AgentEnvironmentVariable> environmentVariables;
  final String? hostDeviceId;
  final String? hostDisplayName;
  final String? workspacePath;
  final int createdAtMs;
  final int updatedAtMs;

  AgentProfile copyWith({
    String? id,
    String? agentId,
    String? name,
    String? description,
    AgentName? runtimeAgent,
    String? model,
    AgentReasoningEffort? reasoningEffort,
    List<AgentEnvironmentVariable>? environmentVariables,
    String? hostDeviceId,
    String? hostDisplayName,
    String? workspacePath,
    int? createdAtMs,
    int? updatedAtMs,
  }) {
    return AgentProfile(
      id: id ?? this.id,
      agentId: agentId ?? this.agentId,
      name: name ?? this.name,
      description: description ?? this.description,
      runtimeAgent: runtimeAgent ?? this.runtimeAgent,
      model: model ?? this.model,
      reasoningEffort: reasoningEffort ?? this.reasoningEffort,
      environmentVariables: environmentVariables ?? this.environmentVariables,
      hostDeviceId: hostDeviceId ?? this.hostDeviceId,
      hostDisplayName: hostDisplayName ?? this.hostDisplayName,
      workspacePath: workspacePath ?? this.workspacePath,
      createdAtMs: createdAtMs ?? this.createdAtMs,
      updatedAtMs: updatedAtMs ?? this.updatedAtMs,
    );
  }

  AgentProfile copyWithDraft(
    AgentProfileDraft draft, {
    required int updatedAtMs,
  }) {
    return AgentProfile(
      id: id,
      agentId: agentId,
      name: draft.name.trim(),
      description: draft.description.trim(),
      runtimeAgent: draft.runtimeAgent,
      model: draft.model.trim(),
      reasoningEffort: draft.reasoningEffort,
      environmentVariables: _normalizedEnvironmentVariables(
        draft.environmentVariables,
      ),
      hostDeviceId: _trimmedOrNull(draft.hostDeviceId),
      hostDisplayName: _trimmedOrNull(draft.hostDisplayName),
      workspacePath: _trimmedOrNull(draft.workspacePath),
      createdAtMs: createdAtMs,
      updatedAtMs: updatedAtMs,
    );
  }

  Map<String, Object?> toJson() {
    return <String, Object?>{
      'id': id,
      'agentId': agentId,
      'name': name,
      'description': description,
      'runtimeAgent': runtimeAgent.name,
      'model': model,
      'reasoningEffort': reasoningEffort.name,
      'environmentVariables': environmentVariables
          .map((entry) => entry.toJson())
          .toList(),
      'hostDeviceId': hostDeviceId,
      'hostDisplayName': hostDisplayName,
      'workspacePath': workspacePath,
      'createdAtMs': createdAtMs,
      'updatedAtMs': updatedAtMs,
    };
  }

  factory AgentProfile.fromJson(Map<String, Object?> json) {
    final id = json['id'] as String;
    return AgentProfile(
      id: id,
      agentId: json['agentId'] as String? ?? _generateAgentId(id),
      name: json['name'] as String? ?? 'Agent',
      description: json['description'] as String? ?? '',
      runtimeAgent: _agentNameFromJson(json['runtimeAgent'] as String?),
      model: json['model'] as String? ?? 'GPT-5.5',
      reasoningEffort: _reasoningEffortFromJson(
        json['reasoningEffort'] as String?,
      ),
      environmentVariables:
          ((json['environmentVariables'] as List<Object?>?) ??
                  const <Object?>[])
              .whereType<Map<Object?, Object?>>()
              .map(
                (entry) => AgentEnvironmentVariable.fromJson(
                  entry.map((key, value) => MapEntry(key.toString(), value)),
                ),
              )
              .toList(),
      hostDeviceId: _trimmedOrNull(json['hostDeviceId'] as String?),
      hostDisplayName: _trimmedOrNull(json['hostDisplayName'] as String?),
      workspacePath: _trimmedOrNull(json['workspacePath'] as String?),
      createdAtMs: (json['createdAtMs'] as num?)?.toInt() ?? 0,
      updatedAtMs: (json['updatedAtMs'] as num?)?.toInt() ?? 0,
    );
  }
}

class AgentWorkspaceState {
  const AgentWorkspaceState({
    required this.profiles,
    required this.preferredProfileId,
    required this.threadProfileBindings,
  });

  const AgentWorkspaceState.empty()
    : profiles = const <AgentProfile>[],
      preferredProfileId = null,
      threadProfileBindings = const <String, String>{};

  final List<AgentProfile> profiles;
  final String? preferredProfileId;
  final Map<String, String> threadProfileBindings;

  AgentProfile? get preferredProfile {
    if (preferredProfileId == null) {
      return profiles.isEmpty ? null : profiles.first;
    }
    for (final profile in profiles) {
      if (profile.id == preferredProfileId) return profile;
    }
    return profiles.isEmpty ? null : profiles.first;
  }

  AgentProfile? profileById(String id) {
    for (final profile in profiles) {
      if (profile.id == id) return profile;
    }
    return null;
  }

  AgentProfile? profileForThread(String sessionId) {
    final profileId = threadProfileBindings[sessionId];
    if (profileId == null) return null;
    return profileById(profileId);
  }

  AgentWorkspaceState copyWith({
    List<AgentProfile>? profiles,
    String? preferredProfileId,
    Map<String, String>? threadProfileBindings,
  }) {
    return AgentWorkspaceState(
      profiles: profiles ?? this.profiles,
      preferredProfileId: preferredProfileId ?? this.preferredProfileId,
      threadProfileBindings:
          threadProfileBindings ?? this.threadProfileBindings,
    );
  }

  Map<String, Object?> toJson() {
    return <String, Object?>{
      'profiles': profiles.map((profile) => profile.toJson()).toList(),
      'preferredProfileId': preferredProfileId,
      'threadProfileBindings': threadProfileBindings,
    };
  }

  factory AgentWorkspaceState.fromJson(Map<String, Object?> json) {
    final rawProfiles =
        (json['profiles'] as List<Object?>?) ?? const <Object?>[];
    final profiles = rawProfiles
        .whereType<Map<Object?, Object?>>()
        .map(
          (entry) => AgentProfile.fromJson(
            entry.map((key, value) => MapEntry(key.toString(), value)),
          ),
        )
        .toList();
    final preferredProfileId = json['preferredProfileId'] as String?;
    final rawBindings =
        (json['threadProfileBindings'] as Map<Object?, Object?>?) ??
        const <Object?, Object?>{};
    return AgentWorkspaceState(
      profiles: profiles,
      preferredProfileId: preferredProfileId,
      threadProfileBindings: rawBindings.map(
        (key, value) => MapEntry(key.toString(), value.toString()),
      ),
    ).normalized();
  }

  AgentWorkspaceState normalized() {
    final normalizedProfiles = profiles;
    final preferred =
        normalizedProfiles.isNotEmpty &&
            normalizedProfiles.any(
              (profile) => profile.id == preferredProfileId,
            )
        ? preferredProfileId
        : normalizedProfiles.isEmpty
        ? null
        : normalizedProfiles.first.id;
    final validIds = normalizedProfiles.map((profile) => profile.id).toSet();
    final filteredBindings = <String, String>{};
    for (final entry in threadProfileBindings.entries) {
      if (validIds.contains(entry.value)) {
        filteredBindings[entry.key] = entry.value;
      }
    }
    return AgentWorkspaceState(
      profiles: normalizedProfiles,
      preferredProfileId: preferred,
      threadProfileBindings: filteredBindings,
    );
  }
}

List<AgentEnvironmentVariable> _normalizedEnvironmentVariables(
  List<AgentEnvironmentVariable> values,
) {
  return values
      .map(
        (entry) => AgentEnvironmentVariable(
          key: entry.key.trim(),
          value: entry.value.trim(),
        ),
      )
      .where((entry) => entry.key.isNotEmpty)
      .toList(growable: false);
}

String? _trimmedOrNull(String? value) {
  final trimmed = value?.trim();
  if (trimmed == null || trimmed.isEmpty) return null;
  return trimmed;
}

AgentName _agentNameFromJson(String? value) {
  return switch (value) {
    'claude' => AgentName.claude,
    'gemini' => AgentName.gemini,
    'opencode' => AgentName.opencode,
    'grok' => AgentName.grok,
    _ => AgentName.codex,
  };
}

AgentReasoningEffort _reasoningEffortFromJson(String? value) {
  return switch (value) {
    'low' => AgentReasoningEffort.low,
    'high' => AgentReasoningEffort.high,
    _ => AgentReasoningEffort.medium,
  };
}

/// Generate a stable agentId from the profile id for backward compatibility
/// with profiles created before agentId was introduced.
String _generateAgentId(String profileId) {
  return 'bot-$profileId';
}
