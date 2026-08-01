import 'package:flutter_riverpod/flutter_riverpod.dart'
    show AsyncNotifier, AsyncNotifierProvider, FutureProvider;
import 'package:minos/data/repositories/runtime_repository.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'minos_providers.g.dart';

/// Hot stream of connection-state transitions sourced from the Rust core.
@Riverpod(keepAlive: true)
Stream<ConnectionState> connectionState(Ref ref) {
  return ref.watch(runtimeRepositoryProvider).connectionStates;
}

final hasPersistedPairingProvider = FutureProvider<bool>((ref) {
  return ref.watch(runtimeRepositoryProvider).hasPersistedPairing();
});

/// Display name of the currently paired peer, sourced from the QR's
/// `host_display_name` at pair time. `null` when no pairing exists or
/// the name was never recorded (e.g. pairings made before this field
/// was added).
final peerDisplayNameProvider = FutureProvider<String?>((ref) {
  return ref.watch(runtimeRepositoryProvider).peerDisplayName();
});

final runtimeAgentDescriptorsProvider = FutureProvider<List<AgentDescriptor>>((
  ref,
) async {
  final pairedHosts = await ref.watch(pairedMacsProvider.future);
  if (pairedHosts.isEmpty) {
    return const <AgentDescriptor>[];
  }
  return ref.watch(runtimeRepositoryProvider).listRuntimeAgents();
});

final hostSkillsProvider =
    FutureProvider.family<List<HostSkillsEntry>, String?>((
      ref,
      hostDeviceId,
    ) async {
      return ref
          .watch(runtimeRepositoryProvider)
          .listHostSkills(hostDeviceId: hostDeviceId);
    });

final hostWorkspacesProvider =
    FutureProvider.family<ListHostWorkspacesResponse, String?>((
      ref,
      hostDeviceId,
    ) async {
      return ref
          .watch(runtimeRepositoryProvider)
          .listHostWorkspaces(hostDeviceId: hostDeviceId);
    });

/// Paired Macs for the current account. Drives the Partners list. Refresh
/// happens via `ref.invalidate(pairedMacsProvider)` after a forget /
/// successful pair — there is no polling stream yet, the user can pull
/// the partners tab to refresh.
final pairedMacsProvider =
    AsyncNotifierProvider<PairedMacs, List<HostSummaryDto>>(PairedMacs.new);

class PairedMacs extends AsyncNotifier<List<HostSummaryDto>> {
  @override
  Future<List<HostSummaryDto>> build() {
    return ref.watch(runtimeRepositoryProvider).listPairedHosts();
  }

  Future<void> refresh() async {
    final previous = state;
    try {
      state = AsyncValue.data(
        await ref.read(runtimeRepositoryProvider).listPairedHosts(),
      );
    } catch (error, stackTrace) {
      if (previous.hasValue) {
        state = previous;
        Error.throwWithStackTrace(error, stackTrace);
      }
      state = AsyncValue.error(error, stackTrace);
    }
  }
}

/// Routing target for `Forward` envelopes. `null` means no Mac is selected
/// — the daemon falls back to broadcast-style fan-out when this is unset.
@riverpod
class ActiveMac extends _$ActiveMac {
  @override
  Future<String?> build() {
    return ref.watch(runtimeRepositoryProvider).activeHost();
  }

  /// Set [macId] as the routing target. Updates state optimistically; if
  /// the FRB call fails the state surfaces the error and re-reads the
  /// core-side truth.
  Future<void> setActive(String macId) async {
    final previous = state;
    state = AsyncValue.data(macId);
    try {
      await ref.read(runtimeRepositoryProvider).setActiveHost(macId);
    } catch (e, st) {
      state = AsyncValue.error(e, st);
      try {
        state = AsyncValue.data(
          await ref.read(runtimeRepositoryProvider).activeHost(),
        );
      } catch (_) {
        state = previous;
      }
    }
  }

  /// Re-read the active mac from the core; used after a forget so the
  /// cached value doesn't point at a no-longer-paired Mac.
  Future<void> refresh() async {
    state = const AsyncValue.loading();
    try {
      state = AsyncValue.data(
        await ref.read(runtimeRepositoryProvider).activeHost(),
      );
    } catch (e, st) {
      state = AsyncValue.error(e, st);
    }
  }
}

