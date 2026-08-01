import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/data/cloud/cloud_config.dart';
import 'package:minos/data/cloud/minos_cloud_client.dart';
import 'package:minos/data/repositories/runtime_repository.dart';
import 'package:minos/domain/linked_host.dart';
import 'package:minos/infrastructure/platform_int64.dart';
import 'package:minos/infrastructure/secure_pairing_store.dart';
import 'package:minos/src/rust/api/minos.dart' show HostSummaryDto;

final cloudConfigProvider = Provider<CloudConfig>((ref) {
  return CloudConfig.fromEnvironment();
});

final minosCloudClientProvider = Provider<MinosCloudClient>((ref) {
  return MinosCloudClient(config: ref.watch(cloudConfigProvider));
});

final securePairingStoreProvider = Provider<SecurePairingStore>((ref) {
  return SecurePairingStore();
});

final hostsRepositoryProvider = Provider<HostsRepository>((ref) {
  return HostsRepository(
    cloud: ref.watch(minosCloudClientProvider),
    secureStore: ref.watch(securePairingStoreProvider),
    runtime: ref.watch(runtimeRepositoryProvider),
  );
});

/// Loads linked hosts via pure-Dart `GET /v1/hosts` when a Minos bearer is
/// available in the Keychain; falls back to the FRB `listPairedHosts` path
/// (which now also hits `GET /v1/hosts` on the Rust side).
class HostsRepository {
  HostsRepository({
    required MinosCloudClient cloud,
    required SecurePairingStore secureStore,
    required RuntimeRepository runtime,
  }) : _cloud = cloud,
       _secure = secureStore,
       _runtime = runtime;

  final MinosCloudClient _cloud;
  final SecurePairingStore _secure;
  final RuntimeRepository _runtime;

  /// Domain model list (unit-test friendly).
  Future<List<LinkedHost>> listLinkedHosts() async {
    final state = await _secure.loadState();
    final deviceId = state?.deviceId;
    final accessToken = state?.accessToken;
    if (deviceId != null &&
        deviceId.isNotEmpty &&
        accessToken != null &&
        accessToken.isNotEmpty) {
      try {
        return await _cloud.listHosts(
          deviceId: deviceId,
          accessToken: accessToken,
        );
      } catch (_) {
        // Fall through to FRB — token may be mid-refresh while the Rust
        // client already holds a rotated bearer.
      }
    }
    final frbHosts = await _runtime.listPairedHosts();
    return frbHosts
        .map(
          (h) => LinkedHost(
            hostInstallationId: h.hostDeviceId,
            hostDisplayName: h.hostDisplayName,
            linkedAtMs: platformInt64ToInt(h.pairedAtMs),
            online: h.online,
            // FRB DTO has no last_seen yet; use linked time as cold fallback.
            lastSeenAtMs: platformInt64ToInt(h.pairedAtMs),
          ),
        )
        .toList(growable: false);
  }

  /// FRB-compatible DTO list for existing UI / providers.
  Future<List<HostSummaryDto>> listHostsAsDto() async {
    final linked = await listLinkedHosts();
    return linked.map(linkedHostToDto).toList(growable: false);
  }
}

/// Maps pure-Dart [LinkedHost] → FRB [HostSummaryDto] for existing widgets.
HostSummaryDto linkedHostToDto(LinkedHost host) {
  return HostSummaryDto(
    hostDeviceId: host.hostInstallationId,
    hostDisplayName: host.hostDisplayName,
    pairedAtMs: platformInt64FromInt(host.linkedAtMs),
    pairedViaDeviceId: '00000000-0000-0000-0000-000000000000',
    online: host.online,
  );
}
