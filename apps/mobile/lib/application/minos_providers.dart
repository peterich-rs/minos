import 'dart:async';
import 'dart:convert';

import 'package:flutter_riverpod/flutter_riverpod.dart'
    show AsyncNotifier, AsyncNotifierProvider, FutureProvider, Provider;
import 'package:minos/data/repositories/hosts_repository.dart';
import 'package:minos/data/repositories/runtime_repository.dart';
import 'package:minos/data/repositories/thread_repository.dart';
import 'package:minos/domain/linked_host.dart';
import 'package:minos/infrastructure/platform_int64.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'minos_providers.g.dart';

/// Hot stream of connection-state transitions sourced from the Rust core.
///
/// This is IM **account online** for this phone: live `/ws/client` to the hub.
@Riverpod(keepAlive: true)
Stream<ConnectionState> connectionState(Ref ref) {
  return ref.watch(runtimeRepositoryProvider).connectionStates;
}

/// Keep [pairedMacsProvider] in sync with hub presence StreamEvents
/// (`UiEventMessage.raw(kind: presence)`). Device online = host WS on server.
///
/// Also arms HostLinked / HostUnlinked durable roster events (Realtime R1):
/// upsert/remove members — not presence-only.
final hostPresenceSyncProvider = Provider<void>((ref) {
  final repo = ref.watch(threadRepositoryProvider);
  final sub = repo.uiEvents.listen((frame) {
    final ui = frame.ui;
    if (ui is! UiEventMessage_Raw) return;
    try {
      if (ui.kind == 'presence') {
        final map = jsonDecode(ui.payloadJson);
        if (map is! Map) return;
        final kind = map['principal_kind']?.toString();
        // Only host device rows live in pairedMacs; account_client is for hosts.
        if (kind != null && kind != 'host') return;
        final id = map['installation_id']?.toString().trim() ?? '';
        if (id.isEmpty) return;
        final online = map['online'] == true;
        final lastSeen = map['last_seen_at_ms'];
        final lastSeenMs = lastSeen is int
            ? lastSeen
            : lastSeen is num
            ? lastSeen.toInt()
            : int.tryParse(lastSeen?.toString() ?? '') ?? 0;
        unawaited(
          ref
              .read(pairedMacsProvider.notifier)
              .applyHostPresence(
                installationId: id,
                online: online,
                lastSeenAtMs: lastSeenMs > 0 ? lastSeenMs : null,
              ),
        );
        return;
      }
      if (ui.kind == 'host_linked') {
        final map = jsonDecode(ui.payloadJson);
        if (map is! Map) return;
        final id = map['host_installation_id']?.toString().trim() ?? '';
        if (id.isEmpty) return;
        final display =
            map['host_display_name']?.toString().trim().isNotEmpty == true
            ? map['host_display_name'].toString().trim()
            : 'host';
        final atMsRaw = map['at_ms'];
        final atMs = atMsRaw is int
            ? atMsRaw
            : atMsRaw is num
            ? atMsRaw.toInt()
            : int.tryParse(atMsRaw?.toString() ?? '') ?? 0;
        unawaited(
          ref
              .read(pairedMacsProvider.notifier)
              .applyHostLinked(
                installationId: id,
                displayName: display,
                linkedAtMs: atMs > 0 ? atMs : null,
              ),
        );
        return;
      }
      if (ui.kind == 'host_unlinked') {
        final map = jsonDecode(ui.payloadJson);
        if (map is! Map) return;
        final id = map['host_installation_id']?.toString().trim() ?? '';
        if (id.isEmpty) return;
        unawaited(
          ref
              .read(pairedMacsProvider.notifier)
              .applyHostUnlinked(installationId: id),
        );
        return;
      }
      // friend_request_updated + subscription_limit_exceeded: armed in
      // social_providers / logging path (not host roster).
    } catch (_) {
      // Malformed presence/roster payload — ignore; next list refresh corrects.
    }
  });
  ref.onDispose(sub.cancel);
});

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

/// Linked hosts for the current account (`GET /v1/hosts` via
/// [HostsRepository]). Drives the Partners / Hosts list. Refresh happens
/// via `ref.invalidate(pairedMacsProvider)` or pull-to-refresh.
final pairedMacsProvider =
    AsyncNotifierProvider<PairedMacs, List<HostSummaryDto>>(PairedMacs.new);

class PairedMacs extends AsyncNotifier<List<HostSummaryDto>> {
  /// last_seen_at_ms by host installation id (not on FRB DTO).
  final Map<String, int> _lastSeenByHost = {};

  @override
  Future<List<HostSummaryDto>> build() async {
    // Arm presence listener for the app lifetime.
    ref.watch(hostPresenceSyncProvider);
    final linked = await ref.watch(hostsRepositoryProvider).listLinkedHosts();
    _ingestLastSeen(linked);
    final hosts = linked.map(linkedHostToDto).toList(growable: false);
    await _ensureActiveHost(hosts);
    return hosts;
  }

  Future<void> refresh() async {
    final previous = state;
    try {
      final linked = await ref.read(hostsRepositoryProvider).listLinkedHosts();
      _ingestLastSeen(linked);
      final hosts = linked.map(linkedHostToDto).toList(growable: false);
      await _ensureActiveHost(hosts);
      state = AsyncValue.data(hosts);
    } catch (error, stackTrace) {
      if (previous.hasValue) {
        state = previous;
        Error.throwWithStackTrace(error, stackTrace);
      }
      state = AsyncValue.error(error, stackTrace);
    }
  }

  /// Live hub presence for a host device (IM friend-device online).
  Future<void> applyHostPresence({
    required String installationId,
    required bool online,
    int? lastSeenAtMs,
  }) async {
    if (lastSeenAtMs != null && lastSeenAtMs > 0) {
      _lastSeenByHost[installationId] = lastSeenAtMs;
    }
    final current = state.asData?.value;
    if (current == null) {
      // No list yet — full refresh when possible.
      try {
        await refresh();
      } catch (_) {}
      return;
    }
    var changed = false;
    final next = current
        .map((h) {
          if (h.hostDeviceId != installationId) return h;
          if (h.online == online) return h;
          changed = true;
          return HostSummaryDto(
            hostDeviceId: h.hostDeviceId,
            hostDisplayName: h.hostDisplayName,
            pairedAtMs: h.pairedAtMs,
            pairedViaDeviceId: h.pairedViaDeviceId,
            online: online,
          );
        })
        .toList(growable: false);
    if (changed) {
      state = AsyncValue.data(next);
    }
  }

  /// Durable HostLinked: upsert roster row (membership, not presence).
  Future<void> applyHostLinked({
    required String installationId,
    required String displayName,
    int? linkedAtMs,
  }) async {
    final current = state.asData?.value;
    if (current == null) {
      try {
        await refresh();
      } catch (_) {}
      return;
    }
    final at = linkedAtMs != null && linkedAtMs > 0
        ? platformInt64FromInt(linkedAtMs)
        : platformInt64FromInt(DateTime.now().millisecondsSinceEpoch);
    final idx = current.indexWhere((h) => h.hostDeviceId == installationId);
    if (idx >= 0) {
      final prev = current[idx];
      final next = [...current];
      next[idx] = HostSummaryDto(
        hostDeviceId: prev.hostDeviceId,
        hostDisplayName: displayName.isNotEmpty
            ? displayName
            : prev.hostDisplayName,
        pairedAtMs: at,
        pairedViaDeviceId: prev.pairedViaDeviceId,
        online: prev.online,
      );
      state = AsyncValue.data(next);
      return;
    }
    final inserted = [
      HostSummaryDto(
        hostDeviceId: installationId,
        hostDisplayName: displayName,
        pairedAtMs: at,
        pairedViaDeviceId: '00000000-0000-0000-0000-000000000000',
        online: false,
      ),
      ...current,
    ];
    state = AsyncValue.data(inserted);
    await _ensureActiveHost(inserted);
  }

  /// Durable HostUnlinked: remove roster row.
  Future<void> applyHostUnlinked({required String installationId}) async {
    final current = state.asData?.value;
    if (current == null) {
      try {
        await refresh();
      } catch (_) {}
      return;
    }
    final next = current
        .where((h) => h.hostDeviceId != installationId)
        .toList(growable: false);
    if (next.length == current.length) return;
    _lastSeenByHost.remove(installationId);
    state = AsyncValue.data(next);
  }

  int? lastSeenAtMs(String hostInstallationId) =>
      _lastSeenByHost[hostInstallationId];

  void _ingestLastSeen(List<LinkedHost> linked) {
    for (final h in linked) {
      if (h.lastSeenAtMs > 0) {
        _lastSeenByHost[h.hostInstallationId] = h.lastSeenAtMs;
      }
    }
  }

  /// Golden path: if no active routing target is set but the account has
  /// linked hosts, prefer the first online host (else first listed).
  Future<void> _ensureActiveHost(List<HostSummaryDto> hosts) async {
    if (hosts.isEmpty) return;
    final runtime = ref.read(runtimeRepositoryProvider);
    final current = await runtime.activeHost();
    if (current != null && hosts.any((h) => h.hostDeviceId == current)) {
      return;
    }
    final online = hosts.where((h) => h.online);
    final preferred = online.isNotEmpty ? online.first : hosts.first;
    try {
      await runtime.setActiveHost(preferred.hostDeviceId);
      ref.invalidate(activeMacProvider);
    } catch (_) {
      // Best-effort: session list still works; forwards may need manual select.
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
