import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/infrastructure/agent_profile_store.dart';

final agentProfileStoreProvider = Provider<AgentProfileStore>((ref) {
  return const JsonFileAgentProfileStore();
});

final agentProfilesControllerProvider =
    AsyncNotifierProvider<AgentProfilesController, AgentWorkspaceState>(
      AgentProfilesController.new,
    );

final preferredAgentProfileProvider = Provider<AgentProfile?>((ref) {
  final state = ref.watch(agentProfilesControllerProvider).asData?.value;
  return state?.preferredProfile;
});

final preferredRuntimeAgentProvider = Provider<AgentName>((ref) {
  return ref.watch(preferredAgentProfileProvider)?.runtimeAgent ??
      AgentName.codex;
});

final threadBoundAgentProfileProvider = Provider.family<AgentProfile?, String>((
  ref,
  threadId,
) {
  final state = ref.watch(agentProfilesControllerProvider).asData?.value;
  return state?.profileForThread(threadId);
});

class AgentProfilesController extends AsyncNotifier<AgentWorkspaceState> {
  AgentProfileStore get _store => ref.read(agentProfileStoreProvider);

  @override
  Future<AgentWorkspaceState> build() async {
    return (await _store.load()).normalized();
  }

  Future<AgentProfile> createProfile(AgentProfileDraft draft) async {
    final current = await future;
    final now = DateTime.now().millisecondsSinceEpoch;
    final id = 'agent-${now.toRadixString(36)}';
    final profile = AgentProfile(
      id: id,
      name: draft.name.trim(),
      description: draft.description.trim(),
      runtimeAgent: draft.runtimeAgent,
      model: draft.model.trim(),
      reasoningEffort: draft.reasoningEffort,
      environmentVariables: draft.environmentVariables,
      hostDeviceId: draft.hostDeviceId?.trim(),
      hostDisplayName: draft.hostDisplayName?.trim(),
      createdAtMs: now,
      updatedAtMs: now,
    ).copyWithDraft(draft, updatedAtMs: now);
    final next = current
        .copyWith(profiles: <AgentProfile>[...current.profiles, profile])
        .normalized();
    await _persist(next);
    return profile;
  }

  Future<void> updateProfile(
    AgentProfile profile,
    AgentProfileDraft draft,
  ) async {
    final current = await future;
    final now = DateTime.now().millisecondsSinceEpoch;
    final nextProfiles = current.profiles
        .map((candidate) {
          if (candidate.id != profile.id) return candidate;
          return candidate.copyWithDraft(draft, updatedAtMs: now);
        })
        .toList(growable: false);
    final next = current.copyWith(profiles: nextProfiles).normalized();
    await _persist(next);
  }

  Future<void> deleteProfile(String profileId) async {
    final current = await future;
    if (current.profiles.length == 1) {
      return;
    }
    final nextProfiles = current.profiles
        .where((profile) => profile.id != profileId)
        .toList(growable: false);
    final next = current
        .copyWith(
          profiles: nextProfiles,
          preferredProfileId: current.preferredProfileId == profileId
              ? nextProfiles.first.id
              : current.preferredProfileId,
          threadProfileBindings: Map<String, String>.from(
            current.threadProfileBindings,
          )..removeWhere((_, value) => value == profileId),
        )
        .normalized();
    await _persist(next);
  }

  Future<void> setPreferredProfile(String profileId) async {
    final current = await future;
    final next = current.copyWith(preferredProfileId: profileId).normalized();
    await _persist(next);
  }

  Future<void> bindThreadToProfile({
    required String threadId,
    required String profileId,
  }) async {
    final current = await future;
    final nextBindings = Map<String, String>.from(current.threadProfileBindings)
      ..[threadId] = profileId;
    final next = current
        .copyWith(threadProfileBindings: nextBindings)
        .normalized();
    await _persist(next);
  }

  Future<void> updateProfileHost({
    required String profileId,
    String? hostDeviceId,
    String? hostDisplayName,
  }) async {
    final current = await future;
    final now = DateTime.now().millisecondsSinceEpoch;
    final nextProfiles = current.profiles
        .map((profile) {
          if (profile.id != profileId) return profile;
          return profile.copyWith(
            hostDeviceId: hostDeviceId,
            hostDisplayName: hostDisplayName,
            updatedAtMs: now,
          );
        })
        .toList(growable: false);
    final next = current.copyWith(profiles: nextProfiles).normalized();
    await _persist(next);
  }

  Future<void> _persist(AgentWorkspaceState next) async {
    state = AsyncValue.data(next);
    await _store.save(next);
  }
}
