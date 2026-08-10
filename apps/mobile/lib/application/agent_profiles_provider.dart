import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/auth_provider.dart';
import 'package:minos/data/repositories/agent_profile_repository.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Local launch-preference cache + optional Hub bot mirror.
///
/// Product bot identity SSOT is Hub `agents`.
/// This controller may still hold device-local drafts; create/update prefer
/// Hub register/update when authenticated.
///
/// Bulk-import of offline drafts is **not** started from [build] — see
/// [hubBotImportBootstrapProvider] (auth lifecycle, app root).
final agentProfilesControllerProvider =
    AsyncNotifierProvider<AgentProfilesController, AgentWorkspaceState>(
      AgentProfilesController.new,
    );

final preferredAgentProfileProvider = Provider<AgentProfile?>((ref) {
  final state = ref.watch(agentProfilesControllerProvider).asData?.value;
  return state?.preferredProfile;
});

final preferredRuntimeAgentProvider = Provider<AgentName?>((ref) {
  return ref.watch(preferredAgentProfileProvider)?.runtimeAgent;
});

final threadBoundAgentProfileProvider = Provider.family<AgentProfile?, String>((
  ref,
  sessionId,
) {
  final state = ref.watch(agentProfilesControllerProvider).asData?.value;
  return state?.profileForThread(sessionId);
});

/// App-root bootstrap: import offline local bot drafts → Hub on **auth
/// transition into** [AuthAuthenticated] only.
///
/// Pattern matches [imOutboxBootstrapProvider]: `ref.watch` this from
/// [MinosApp], never from a high-frequency [AsyncNotifier.build].
///
/// - Does not run on every provider rebuild.
/// - Resets after logout so a later login can import again.
/// - Concurrent calls coalesce via the controller's in-flight future.
final hubBotImportBootstrapProvider = Provider<void>((ref) {
  ref.listen<AuthState>(authControllerProvider, (previous, next) {
    if (next is AuthUnauthenticated) {
      // Next login may need import for a different account's drafts.
      ref
          .read(agentProfilesControllerProvider.notifier)
          .resetHubImportSession();
      return;
    }
    if (next is! AuthAuthenticated) return;
    // Only fire on edge into Authenticated (e.g. Bootstrapping → Authenticated).
    if (previous is AuthAuthenticated) return;
    unawaited(
      ref
          .read(agentProfilesControllerProvider.notifier)
          .importLocalProfilesToHub(),
    );
  });
});

class AgentProfilesController extends AsyncNotifier<AgentWorkspaceState> {
  AgentProfileRepository get _repository =>
      ref.read(agentProfileRepositoryProvider);

  /// Coalesces concurrent import calls; cleared when finished or on logout.
  Future<int>? _importInFlight;

  @override
  Future<AgentWorkspaceState> build() async {
    // Pure load only — no auth side effects here (build re-runs on invalidate).
    return _repository.loadWorkspace();
  }

  /// Allow a future auth session to run import again (logout).
  void resetHubImportSession() {
    _importInFlight = null;
  }

  /// Idempotent by bot name: register missing local drafts on Hub and rewrite
  /// local cache `agentId` to the Hub `agent_id`. Safe to call repeatedly.
  ///
  /// Call from auth lifecycle ([hubBotImportBootstrapProvider]), not from
  /// [build]. Concurrent callers share one in-flight future.
  Future<int> importLocalProfilesToHub() {
    final existing = _importInFlight;
    if (existing != null) return existing;
    final future = _importLocalProfilesToHubBody();
    _importInFlight = future;
    return future;
  }

  Future<int> _importLocalProfilesToHubBody() async {
    try {
      final current = await future;
      if (current.profiles.isEmpty) return 0;

      final repository = ref.read(socialRepositoryProvider);
      final ListAgentsResponse remote;
      try {
        remote = await repository.listAgents();
      } catch (_) {
        // Not online / not authenticated — clear in-flight so next auth edge retries.
        _importInFlight = null;
        return 0;
      }

      final byName = <String, AgentSummary>{};
      for (final agent in remote.agents) {
        final key = agent.name.trim().toLowerCase();
        if (key.isNotEmpty) byName[key] = agent;
      }

      var imported = 0;
      final now = DateTime.now().millisecondsSinceEpoch;
      final nextProfiles = <AgentProfile>[];
      for (final profile in current.profiles) {
        final nameKey = profile.name.trim().toLowerCase();
        if (nameKey.isEmpty) {
          nextProfiles.add(profile);
          continue;
        }

        // Already bound to a Hub bot that still exists.
        final existingById = remote.agents
            .where((a) => a.agentId == profile.agentId)
            .toList(growable: false);
        if (existingById.isNotEmpty) {
          nextProfiles.add(profile);
          continue;
        }

        // Name match on Hub → rebind local cache to that agent_id.
        final named = byName[nameKey];
        if (named != null) {
          if (named.agentId != profile.agentId) {
            nextProfiles.add(
              profile.copyWith(agentId: named.agentId, updatedAtMs: now),
            );
            imported += 1;
          } else {
            nextProfiles.add(profile);
          }
          continue;
        }

        // Mint on Hub.
        try {
          final registered = await repository.registerAgent(
            name: profile.name.trim(),
            description: profile.description.trim(),
            runtimeAgent: profile.runtimeAgent.name,
            model: profile.model.trim(),
            workspacePath: profile.workspacePath?.trim(),
            displayName: profile.name.trim(),
            defaultReasoningEffort: profile.reasoningEffort.name,
            systemPrompt: '',
          );
          byName[nameKey] = registered;
          nextProfiles.add(
            profile.copyWith(agentId: registered.agentId, updatedAtMs: now),
          );
          imported += 1;
        } catch (_) {
          nextProfiles.add(profile);
        }
      }

      if (imported > 0) {
        final next = current.copyWith(profiles: nextProfiles).normalized();
        await _persist(next);
      }
      return imported;
    } catch (_) {
      _importInFlight = null;
      return 0;
    }
  }

  Future<AgentProfile> createProfile(AgentProfileDraft draft) async {
    final current = await future;
    final now = DateTime.now().millisecondsSinceEpoch;
    final id = 'agent-${now.toRadixString(36)}';
    // Hub-first: authenticated create must mint on Hub. Offline drafts keep an
    // empty agentId and are not multi-end collab identity until import/mint.
    String agentId = '';
    try {
      final hub = await ref
          .read(socialRepositoryProvider)
          .registerAgent(
            name: draft.name.trim(),
            description: draft.description.trim(),
            runtimeAgent: draft.runtimeAgent.name,
            model: draft.model.trim(),
            workspacePath: draft.workspacePath?.trim(),
            displayName: draft.name.trim(),
            defaultReasoningEffort: draft.reasoningEffort.name,
            systemPrompt: '',
          );
      agentId = hub.agentId;
    } catch (error) {
      // Only allow local cache draft when Hub is unreachable / unauthenticated.
      // Empty agentId marks pendingSync — cannot join collab as identity.
      agentId = '';
    }
    final profile = AgentProfile(
      id: id,
      agentId: agentId,
      name: draft.name.trim(),
      description: draft.description.trim(),
      runtimeAgent: draft.runtimeAgent,
      model: draft.model.trim(),
      reasoningEffort: draft.reasoningEffort,
      environmentVariables: draft.environmentVariables,
      hostDeviceId: draft.hostDeviceId?.trim(),
      hostDisplayName: draft.hostDisplayName?.trim(),
      workspacePath: draft.workspacePath?.trim(),
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
    // Best-effort Hub update when this profile already has a real bot id.
    // Round-trip digital-body fields the draft does not edit (status, system
    // prompt, avatar) so a full-replace server cannot wipe them. Prefer
    // current Hub values when list succeeds.
    if (profile.agentId.trim().isNotEmpty) {
      try {
        final repository = ref.read(socialRepositoryProvider);
        String? systemPrompt;
        String? status;
        try {
          final remote = await repository.listAgents();
          for (final agent in remote.agents) {
            if (agent.agentId == profile.agentId) {
              systemPrompt = agent.systemPrompt;
              status = agent.status;
              break;
            }
          }
        } catch (_) {
          // Fall through: omit optional fields; backend partial-merge keeps them.
        }
        await repository.updateAgent(
          agentId: profile.agentId,
          name: draft.name.trim(),
          description: draft.description.trim(),
          runtimeAgent: draft.runtimeAgent.name,
          model: draft.model.trim(),
          workspacePath: draft.workspacePath?.trim(),
          displayName: draft.name.trim(),
          defaultReasoningEffort: draft.reasoningEffort.name,
          systemPrompt: systemPrompt,
          status: status,
        );
      } catch (error) {
        // Fail closed for multi-end body: do not pretend local cache is SSOT.
        rethrow;
      }
    }
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
    final nextProfiles = current.profiles
        .where((profile) => profile.id != profileId)
        .toList(growable: false);
    final next = current
        .copyWith(
          profiles: nextProfiles,
          preferredProfileId:
              current.preferredProfileId == profileId && nextProfiles.isNotEmpty
              ? nextProfiles.first.id
              : current.preferredProfileId == profileId
              ? null
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
    required String sessionId,
    required String profileId,
  }) async {
    final current = await future;
    final nextBindings = Map<String, String>.from(current.threadProfileBindings)
      ..[sessionId] = profileId;
    final next = current
        .copyWith(threadProfileBindings: nextBindings)
        .normalized();
    await _persist(next);
  }

  Future<AgentProfile> syncServerAgentId({
    required String profileId,
    required String agentId,
  }) async {
    final current = await future;
    final now = DateTime.now().millisecondsSinceEpoch;
    AgentProfile? updated;
    final nextProfiles = current.profiles
        .map((profile) {
          if (profile.id != profileId) return profile;
          updated = profile.copyWith(agentId: agentId, updatedAtMs: now);
          return updated!;
        })
        .toList(growable: false);
    if (updated == null) {
      throw StateError('agent profile not found: $profileId');
    }
    final next = current.copyWith(profiles: nextProfiles).normalized();
    await _persist(next);
    return updated!;
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
    await _repository.saveWorkspace(next);
  }
}
