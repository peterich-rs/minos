import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

final runtimeRepositoryProvider = Provider<RuntimeRepository>((ref) {
  return RuntimeRepository(ref.watch(minosCoreServiceProvider));
});

class RuntimeRepository {
  const RuntimeRepository(this._core);

  final MinosCoreProtocol _core;

  Stream<ConnectionState> get connectionStates => _core.connectionStates;

  ConnectionState get currentConnectionState => _core.currentConnectionState;

  Future<bool> hasPersistedPairing() {
    return _core.hasPersistedPairing();
  }

  Future<String?> peerDisplayName() {
    return _core.peerDisplayName();
  }

  Future<List<AgentDescriptor>> listRuntimeAgents() {
    return _core.listClis();
  }

  Future<List<HostSkillsEntry>> listHostSkills({String? hostDeviceId}) async {
    final response = await _core.listHostSkills(
      hostDeviceId: hostDeviceId,
      forceReload: true,
    );
    return response.data;
  }

  Future<ListHostWorkspacesResponse> listHostWorkspaces({
    String? hostDeviceId,
    String? root,
    int limit = 100,
  }) {
    return _core.listHostWorkspaces(
      hostDeviceId: hostDeviceId,
      root: root,
      limit: limit,
    );
  }

  Future<List<HostSummaryDto>> listPairedHosts() {
    return _core.listPairedHosts();
  }

  Future<String?> activeHost() {
    return _core.activeHost();
  }

  Future<void> setActiveHost(String hostDeviceId) {
    return _core.setActiveHost(hostDeviceId);
  }

  void notifyForegrounded() {
    _core.notifyForegrounded();
  }

  void notifyBackgrounded() {
    _core.notifyBackgrounded();
  }

  Future<void> forgetHost(String hostDeviceId) {
    return _core.forgetHost(hostDeviceId);
  }

  Future<void> writeHostSkillConfig({
    required String hostDeviceId,
    required String path,
    required bool enabled,
  }) async {
    await _core.writeHostSkillConfig(
      hostDeviceId: hostDeviceId,
      path: path,
      enabled: enabled,
    );
  }
}
